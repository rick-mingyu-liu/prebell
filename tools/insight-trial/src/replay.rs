//! `rewrite`: the Worker's rewrite of refused lines (`insight-rewrite/v1`,
//! `write::rewrite_prompt`) replayed on stored shadow records
//! (`fetch-shadow.sh`), to see what the rewrite would have made of lines the
//! code checks refused before the Worker had it. Per record with a refused
//! line: one rewrite call (Claude, as the Worker's writer) over the refused
//! lines only, the code checks on its lines, then one Claude judge call
//! (the Worker's judge) over the lines that now pass them. Results go to
//! `OUT/rewrite.jsonl`; the spend ledger and cap are `run`'s.

use std::path::Path;

use prebell_core::insight::judge::{self, Verdict};
use prebell_core::insight::write::{self, REWRITE_VERSION};
use prebell_core::insight::{check, step, Insight, Pack};
use serde::{Deserialize, Serialize};

use crate::shadow::post_name;
use crate::{append_jsonl, stop_message, Models, Plan, CLAUDE_MODEL};

/// One refused line and what the rewrite made of it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct Replayed {
    pub post: String,
    pub ticker: String,
    /// The writer's line and the current code checks' refusals of it.
    pub refused: Insight,
    pub refused_check: Vec<String>,
    /// The rewrite's line, `None` when it left the mover out or the call
    /// failed (`error`).
    pub rewritten: Option<Insight>,
    /// The code checks' refusals of the rewritten line.
    pub check: Vec<String>,
    pub error: Option<String>,
    /// Claude's verdict, for a rewritten line that passed the checks.
    pub verdict: Option<Verdict>,
    pub passed: bool,
    pub rewrite_version: String,
}

/// The lines of `rec` the rewrite would be asked for: written lines the
/// current code checks refuse, the Worker's rewrite not already applied.
pub(crate) fn refused(rec: &step::Record) -> Vec<(Pack, Insight, Vec<String>)> {
    rec.movers
        .iter()
        .filter(|m| m.rewritten_from.is_none())
        .filter_map(|m| {
            let i = m.insight.clone()?;
            let c = check::check(&m.pack, &i);
            (!c.is_empty()).then(|| (m.pack.clone(), i, c))
        })
        .collect()
}

/// The calls `rewrite` will make over `records`, skipping posts already in
/// `done`: one rewrite call per record with a refused line, and at most one
/// judge call after it.
pub(crate) fn plan(records: &[step::Record], done: &[Replayed], ledger: &Path) -> Plan {
    let mut plan = Plan::default();
    for rec in records {
        let post = post_name(&rec.facts_key);
        let items = refused(rec);
        if items.is_empty() || done.iter().any(|d| d.post == post) {
            continue;
        }
        let (system, user) = write::rewrite_prompt(&items);
        plan.add(
            ledger,
            "writer",
            CLAUDE_MODEL,
            system.len() + user.len(),
            items.len(),
        );
        let packs: Vec<Pack> = items.iter().map(|(p, _, _)| p.clone()).collect();
        let lines: Vec<Insight> = items.iter().map(|(_, i, _)| i.clone()).collect();
        let (jsystem, juser) = judge::prompt(&packs, &lines);
        plan.add(
            ledger,
            "judge",
            CLAUDE_MODEL,
            jsystem.len() + juser.len(),
            items.len(),
        );
    }
    plan
}

/// Replays the rewrite on every record not already in `OUT/rewrite.jsonl`,
/// appending each record's rows as it finishes. Stops at the budget cap,
/// keeping what was paid for.
pub(crate) async fn replay(
    records: &[step::Record],
    m: &Models,
    out: &Path,
    done: &[Replayed],
) -> Result<Vec<Replayed>, String> {
    let path = out.join("rewrite.jsonl");
    let judge_llm = m
        .judges
        .iter()
        .find(|(n, _)| n == "claude")
        .map(|(_, l)| l.as_ref())
        .ok_or("no Claude judge")?;
    let mut all = Vec::new();
    for rec in records {
        let post = post_name(&rec.facts_key);
        let items = refused(rec);
        if items.is_empty() || done.iter().any(|d| d.post == post) {
            continue;
        }
        let row = |(p, i, c): &(Pack, Insight, Vec<String>)| Replayed {
            post: post.clone(),
            ticker: p.ticker.clone(),
            refused: i.clone(),
            refused_check: c.clone(),
            rewritten: None,
            check: vec![],
            error: None,
            verdict: None,
            passed: false,
            rewrite_version: REWRITE_VERSION.into(),
        };
        let mut rows: Vec<Replayed> = items.iter().map(row).collect();
        let packs: Vec<Pack> = items.iter().map(|(p, _, _)| p.clone()).collect();
        let (system, user) = write::rewrite_prompt(&items);
        match m.writer.complete_json(&system, &user).await {
            Err(e) => {
                if m.budget.exceeded.get() {
                    return Err(stop_message(&m.budget));
                }
                for r in &mut rows {
                    r.error = Some(format!("rewrite call failed: {e}"));
                }
            }
            Ok(reply) => {
                let parsed = write::parse(&reply, &packs);
                for r in &mut rows {
                    let i = parsed
                        .iter()
                        .filter_map(|x| x.as_ref().ok())
                        .find(|i| i.ticker == r.ticker);
                    match i {
                        Some(i) => {
                            let p = packs.iter().find(|p| p.ticker == r.ticker).expect("pack");
                            r.check = check::check(p, i);
                            r.rewritten = Some(i.clone());
                        }
                        None => r.error = Some("the rewrite left the mover out".into()),
                    }
                }
                let (jp, ji): (Vec<Pack>, Vec<Insight>) = rows
                    .iter()
                    .filter(|r| r.check.is_empty())
                    .filter_map(|r| {
                        let p = packs.iter().find(|p| p.ticker == r.ticker)?;
                        Some((p.clone(), r.rewritten.clone()?))
                    })
                    .unzip();
                if !ji.is_empty() {
                    match judge::judge(judge_llm, &jp, &ji).await {
                        Err(e) => {
                            if m.budget.exceeded.get() {
                                append_jsonl(&path, &rows);
                                return Err(stop_message(&m.budget));
                            }
                            for r in rows.iter_mut().filter(|r| r.check.is_empty()) {
                                if r.rewritten.is_some() {
                                    r.error = Some(format!("judge call failed: {e}"));
                                }
                            }
                        }
                        Ok(verdicts) => {
                            for r in &mut rows {
                                let (Some(i), Some(p)) =
                                    (&r.rewritten, jp.iter().find(|p| p.ticker == r.ticker))
                                else {
                                    continue;
                                };
                                let v = verdicts.iter().find(|v| v.ticker == r.ticker).cloned();
                                r.passed = v.as_ref().is_some_and(|v| judge::passed(p, i, v));
                                r.verdict = v;
                            }
                        }
                    }
                }
            }
        }
        append_jsonl(&path, &rows);
        all.extend(rows);
    }
    Ok(all)
}

/// One line per refused line: what the rewrite did.
pub(crate) fn summary(rows: &[Replayed]) -> String {
    let rewritten = rows.iter().filter(|r| r.rewritten.is_some()).count();
    let fixed = rows
        .iter()
        .filter(|r| r.rewritten.is_some() && r.check.is_empty())
        .count();
    let passed = rows.iter().filter(|r| r.passed).count();
    format!(
        "{} refused lines: {rewritten} rewritten, {fixed} passed the code checks, {passed} passed the judge",
        rows.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{budget, claude_body, mock_models, tmp, Mock};
    use serde_json::json;

    /// The Worker's recap record of 2026-11-06 with AMD's Chinese line
    /// given a relative time word, as an older record would hold a line
    /// the checks refused.
    fn refused_record() -> (step::Record, Insight, Verdict) {
        let mut rec: step::Record = serde_json::from_str(include_str!(
            "../../../crates/core/src/insight/testdata/shadow/record.json"
        ))
        .unwrap();
        let amd = rec.movers.iter_mut().find(|m| m.ticker == "AMD").unwrap();
        let good = amd.insight.clone().unwrap();
        let verdict = amd.verdict.clone().unwrap();
        let bad = amd.insight.as_mut().unwrap();
        bad.zh.watch = bad.zh.watch.replace("下次财报", "近期财报");
        amd.check = check::check(&amd.pack, bad);
        amd.passed = false;
        (rec, good, verdict)
    }

    #[tokio::test]
    async fn a_refused_line_is_rewritten_checked_and_judged() {
        let out = tmp("rewrite-replay");
        let (rec, good, verdict) = refused_record();
        let items = refused(&rec);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].2, ["zh: relative time: 近期"]);
        let writer = Mock::new(vec![claude_body(&json!({"insights": [good]}), 300)]);
        let judge = Mock::new(vec![claude_body(&json!({"verdicts": [verdict]}), 300)]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        let p = plan(std::slice::from_ref(&rec), &[], &out.join("spend.jsonl"));
        assert_eq!(p.total_calls(), 2);
        let rows = replay(std::slice::from_ref(&rec), &m, &out, &[])
            .await
            .unwrap();
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r.ticker.as_str(), r.passed), ("AMD", true));
        assert!(r.check.is_empty() && r.error.is_none());
        assert_eq!(r.rewritten.as_ref(), Some(&good));
        assert_eq!(r.rewrite_version, "insight-rewrite/v1");
        assert_eq!(
            summary(&rows),
            "1 refused lines: 1 rewritten, 1 passed the code checks, 1 passed the judge"
        );
        // Kept, and a second run skips the post with no call.
        let kept: Vec<Replayed> = crate::read_jsonl(&out.join("rewrite.jsonl"));
        assert_eq!(kept, rows);
        assert_eq!(
            plan(&[rec.clone()], &kept, &out.join("spend.jsonl")).total_calls(),
            0
        );
        assert!(replay(&[rec], &m, &out, &kept).await.unwrap().is_empty());
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        std::fs::remove_dir_all(out).unwrap();
    }

    /// A rewrite that leaves the mover out, or still fails the checks,
    /// makes no judge call; a record with nothing refused makes no call.
    #[tokio::test]
    async fn no_judge_call_without_a_fixed_line_and_no_call_without_a_refusal() {
        let out = tmp("rewrite-replay-none");
        let (rec, mut good, _) = refused_record();
        good.zh.watch = good.zh.watch.replace("下次财报", "明年财报");
        let writer = Mock::new(vec![
            claude_body(&json!({"insights": []}), 100),
            claude_body(&json!({"insights": [good]}), 100),
        ]);
        let judge = Mock::new(vec![]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        let rows = replay(std::slice::from_ref(&rec), &m, &out, &[])
            .await
            .unwrap();
        assert_eq!(
            rows[0].error.as_deref(),
            Some("the rewrite left the mover out")
        );
        let rows = replay(std::slice::from_ref(&rec), &m, &out, &[])
            .await
            .unwrap();
        assert_eq!(rows[0].check, ["zh: relative time: 明年"]);
        assert_eq!((writer.calls.get(), judge.calls.get()), (2, 0));
        let clean: step::Record = serde_json::from_str(include_str!(
            "../../../crates/core/src/insight/testdata/shadow/record.json"
        ))
        .unwrap();
        assert!(refused(&clean).is_empty());
        assert!(replay(&[clean], &m, &out, &[]).await.unwrap().is_empty());
        assert_eq!(writer.calls.get(), 2);
        std::fs::remove_dir_all(out).unwrap();
    }
}
