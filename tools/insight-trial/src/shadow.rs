//! The shadow stage's review: the Worker's records (`insights/*.json`, saved
//! by `fetch-shadow.sh`) turned into the trial's `lines.jsonl` shape, and a
//! cumulative labelling page with every line the judge answered for.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use prebell_core::insight::step;

use crate::score::Label;
use crate::{line_key, JudgeResult, Record};

/// The trial's name for a judge model ("claude" or "deepseek"), as `score`
/// keys its judges.
fn judge_name(model: &str) -> &'static str {
    if model.starts_with("claude") {
        "claude"
    } else {
        "deepseek"
    }
}

/// The trial's post name for a facts key: `premarket/2026-11-17-update.json`
/// is `premarket_2026-11-17-update`, as `fetch-facts.sh` names bundles.
pub(crate) fn post_name(facts_key: &str) -> String {
    facts_key
        .strip_suffix(".json")
        .unwrap_or(facts_key)
        .replace('/', "_")
}

/// One line per mover of every record. A mover the judge has not answered
/// for (the record still waits for its judge, or the call failed) keeps
/// its error and no verdict.
pub(crate) fn lines(records: &[step::Record]) -> Vec<Record> {
    let mut out = Vec::new();
    for rec in records {
        let judge = judge_name(&rec.judge_model);
        for m in &rec.movers {
            let sent = m.insight.is_some() && m.check.is_empty();
            let error = match (&m.verdict, &m.judge_error) {
                (_, Some(e)) => Some(e.clone()),
                (None, None) if sent => Some("not judged yet".to_string()),
                _ => None,
            };
            let j = JudgeResult {
                verdict: m.verdict.clone(),
                passed: m.passed,
                error,
                retried: false,
            };
            out.push(Record {
                post: post_name(&rec.facts_key),
                ticker: m.ticker.clone(),
                pack: m.pack.clone(),
                insight: m.insight.clone(),
                write_error: m.write_error.clone(),
                check: m.check.clone(),
                judges: BTreeMap::from([(judge.to_string(), j)]),
                published_if: BTreeMap::from([(judge.to_string(), sent && m.passed)]),
                // A rewritten line is keyed by the rewrite that wrote it, so a
                // label never carries over from the writer's refused line.
                writer_version: match (&m.rewritten_from, &rec.rewrite_version) {
                    (Some(_), Some(v)) => v.clone(),
                    _ => rec.writer_version.clone(),
                },
                judge_version: rec.judge_version.clone(),
            });
        }
    }
    out
}

/// Every record in `dir` (`*.json`, as `fetch-shadow.sh` saves them), by
/// file name. A file that is not a record is skipped with a warning.
pub(crate) fn read_records(dir: &Path) -> Vec<step::Record> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let text = fs::read_to_string(&p).expect("read record");
        match serde_json::from_str::<step::Record>(&text) {
            Ok(r) => out.push(r),
            Err(e) => eprintln!("skipping {}: not an insight record: {e}", p.display()),
        }
    }
    out
}

/// The lines the review page shows: every line the judge answered for,
/// passed or refused. Lines the code checks refused, and lines with no
/// verdict, never reach readers and are left out.
pub(crate) fn reviewable(lines: &[Record]) -> Vec<&Record> {
    lines
        .iter()
        .filter(|r| r.insight.is_some() && r.check.is_empty())
        .filter(|r| r.judges.values().any(JudgeResult::answered))
        .collect()
}

/// The label already given to each line, by `line_key`.
pub(crate) fn done(labels: &[Label]) -> BTreeMap<String, String> {
    labels
        .iter()
        .filter_map(|l| l.key.clone().map(|k| (k, l.label.clone())))
        .collect()
}

/// `(line, its earlier label)` for the page: new lines first, in a fixed
/// random order, then the ones already labelled.
pub(crate) fn page_lines<'a>(
    lines: &'a [Record],
    labels: &[Label],
) -> Vec<(&'a Record, Option<String>)> {
    let done = done(labels);
    let label = |r: &Record| {
        let i = r
            .insight
            .as_ref()
            .expect("reviewable lines have an insight");
        done.get(&line_key(&r.writer_version, i)).cloned()
    };
    let (mut new, old): (Vec<&Record>, Vec<&Record>) = reviewable(lines)
        .into_iter()
        .partition(|r| label(r).is_none());
    crate::page::Rng(crate::page::SEED).shuffle(&mut new);
    new.into_iter()
        .map(|r| (r, None))
        .chain(old.into_iter().map(|r| (r, label(r))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::{score, Bar};

    /// The Worker's own record for the recap of 2026-11-06, as Plan 2's
    /// step test writes it (the real bundle, the trial's lines and Claude's
    /// verdicts).
    fn record() -> step::Record {
        serde_json::from_str(include_str!(
            "../../../crates/core/src/insight/testdata/shadow/record.json"
        ))
        .unwrap()
    }

    #[test]
    fn a_record_becomes_one_line_per_mover() {
        let l = lines(&[record()]);
        assert_eq!(l.len(), 5);
        assert!(l.iter().all(|r| r.post == "recap_2026-11-06"));
        let passed: Vec<&str> = l
            .iter()
            .filter(|r| r.published_if["claude"])
            .map(|r| r.ticker.as_str())
            .collect();
        assert_eq!(passed, ["AMD", "AXON", "CHTR"]);
        // A mover the writer left out has no line and no judge answer.
        let cohr = l.iter().find(|r| r.ticker == "COHR").unwrap();
        assert!(cohr.insight.is_none() && !cohr.judges["claude"].answered());
        assert_eq!(l[0].writer_version, "insight-writer/v6");
        assert_eq!(
            post_name("premarket/2026-11-17-update.json"),
            "premarket_2026-11-17-update"
        );
    }

    /// A line the rewrite fixed is the judged line, keyed by the rewrite's
    /// version; the writer's refused line is not shown.
    #[test]
    fn a_rewritten_line_is_keyed_by_the_rewrite() {
        let mut r = record();
        r.rewrite_version = Some("insight-rewrite/v1".into());
        let amd = r.movers.iter_mut().find(|m| m.ticker == "AMD").unwrap();
        let mut refused = amd.insight.clone().unwrap();
        refused.zh.watch = refused.zh.watch.replace("下次财报", "近期财报");
        amd.rewritten_from = Some(step::Refusal {
            insight: refused,
            check: vec!["zh: relative time: 近期".into()],
        });
        let l = lines(&[r]);
        let amd = l.iter().find(|x| x.ticker == "AMD").unwrap();
        assert_eq!(amd.writer_version, "insight-rewrite/v1");
        assert!(amd.published_if["claude"]);
        let axon = l.iter().find(|x| x.ticker == "AXON").unwrap();
        assert_eq!(axon.writer_version, "insight-writer/v6");
        assert_eq!(reviewable(&l).len(), 3);
    }

    #[test]
    fn a_record_still_waiting_for_its_judge_shows_no_line() {
        let mut r = record();
        r.stage = step::Stage::Written;
        for m in &mut r.movers {
            m.verdict = None;
            m.passed = false;
        }
        let l = lines(&[r]);
        assert!(reviewable(&l).is_empty());
        let amd = l.iter().find(|x| x.ticker == "AMD").unwrap();
        assert_eq!(
            amd.judges["claude"].error.as_deref(),
            Some("not judged yet")
        );
    }

    /// The page shows every judge-passed line and every line the judge
    /// refused (no sampling), verdicts hidden, and lines already labelled
    /// as done after the new ones.
    #[test]
    fn the_page_has_every_judged_line_and_marks_labelled_ones_done() {
        let mut refused = record();
        refused.facts_key = "recap/2026-11-07.json".into();
        for m in &mut refused.movers {
            m.passed = false;
            if let Some(i) = m.insight.as_mut() {
                i.en.hook.push_str(" again");
            }
        }
        let l = lines(&[record(), refused]);
        let amd = l.iter().find(|r| r.ticker == "AMD").unwrap();
        let labels = vec![Label {
            post: amd.post.clone(),
            ticker: "AMD".into(),
            key: Some(line_key(&amd.writer_version, amd.insight.as_ref().unwrap())),
            label: "fine".into(),
        }];
        let picked = page_lines(&l, &labels);
        assert_eq!(picked.len(), 6);
        assert_eq!(picked.iter().filter(|(_, d)| d.is_some()).count(), 1);
        assert_eq!(picked.last().unwrap().1.as_deref(), Some("fine"));
        let html = crate::page::shadow_page(&picked);
        assert_eq!(html.matches("<article class=\"card").count(), 6);
        assert!(html.contains("5 new lines"), "{html}");
        assert!(html.contains("value=\"fine\" checked"));
        assert!(!html.contains("\"passed\"") && !html.contains("claude"));
        assert!(!html.contains("<script src"));
    }

    /// `score` on shadow lines (no seeded rows) reports the reading part of
    /// the bar: not yet assessable below 50 lines read, met at 50 with none
    /// marked wrong, not met with one.
    #[test]
    fn the_shadow_report_states_the_reading_part_of_the_bar() {
        let recs: Vec<step::Record> = (0..17)
            .map(|k| {
                let mut r = record();
                r.facts_key = format!("recap/2026-12-{:02}.json", k + 1);
                r
            })
            .collect();
        let l = lines(&recs);
        let label = |r: &Record, label: &str| Label {
            post: r.post.clone(),
            ticker: r.ticker.clone(),
            key: Some(line_key(&r.writer_version, r.insight.as_ref().unwrap())),
            label: label.into(),
        };
        let fine: Vec<Label> = reviewable(&l).iter().map(|r| label(r, "fine")).collect();
        assert_eq!(fine.len(), 51);
        let md = crate::score::report(&l, &[], &[], &[], &fine[..9]);
        assert!(md.contains("Shadow: 9 judge-passed lines read"), "{md}");
        assert!(md.contains("The bar is not yet assessable"), "{md}");
        assert!(!md.contains("The trial's own part of the bar"), "{md}");
        let md = crate::score::report(&l, &[], &[], &[], &fine);
        assert!(md.contains("Shadow: 51 judge-passed lines read"), "{md}");
        assert!(md.contains("The reading part of the bar is met"), "{md}");
        let mut one_wrong = fine.clone();
        one_wrong[0].label = "wrong_fact".into();
        let md = crate::score::report(&l, &[], &[], &[], &one_wrong);
        assert!(
            md.contains("The bar is not met: 1 judge-passed lines marked wrong."),
            "{md}"
        );
    }

    /// A line labelled before with the same text (the brief and its update
    /// often repeat one) is done wherever it appears.
    #[test]
    fn the_same_line_in_another_post_keeps_its_label() {
        let mut update = record();
        update.facts_key = "recap/2026-11-07.json".into();
        let l = lines(&[record(), update]);
        let amd = l.iter().find(|r| r.ticker == "AMD").unwrap();
        let labels = vec![Label {
            post: amd.post.clone(),
            ticker: "AMD".into(),
            key: Some(line_key(&amd.writer_version, amd.insight.as_ref().unwrap())),
            label: "fine".into(),
        }];
        let picked = page_lines(&l, &labels);
        assert_eq!(picked.iter().filter(|(_, d)| d.is_some()).count(), 2);
    }

    /// The 8:00 brief and its 8:35 update often carry the same mover: the
    /// page shows both, the bar counts one.
    #[test]
    fn the_bar_counts_a_mover_in_the_brief_and_its_update_once() {
        let mut base = record();
        base.facts_key = "premarket/2026-11-06.json".into();
        base.content_type = "premarket".into();
        let mut update = base.clone();
        update.facts_key = "premarket/2026-11-06-update.json".into();
        let l = lines(&[base, update]);
        assert_eq!(reviewable(&l).len(), 6);
        let labels: Vec<Label> = reviewable(&l)
            .iter()
            .map(|r| Label {
                post: r.post.clone(),
                ticker: r.ticker.clone(),
                key: Some(line_key(&r.writer_version, r.insight.as_ref().unwrap())),
                label: "fine".into(),
            })
            .collect();
        let s = score(&l, &[], &labels);
        let c = &s.judges[0];
        assert_eq!((c.name, c.passed_labelled, c.read), ("claude", 6, 3));
        assert_eq!(s.bar, Bar::NotYetAssessable);
    }
}
