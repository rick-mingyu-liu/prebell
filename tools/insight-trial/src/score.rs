//! The judge score: agreement with the owner's labels, seeded recall, the
//! false-reject rate on controls, what the code checks refused, eligibility
//! and spend, ending with the judge to choose and whether the spec's bar is
//! met.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::Value;

use crate::seed::{Kind, Seeded};
use crate::{line_key, Record, JUDGES};

/// Seeded recall the chosen judge needs (the spec's bar).
const RECALL_BAR: f64 = 0.9;
/// Judge-passed lines Rick must have read (the spec's bar).
const READ_BAR: usize = 20;

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct Label {
    pub post: String,
    pub ticker: String,
    /// `line_key` of the line labelled; absent in labels saved before it.
    #[serde(default)]
    pub key: Option<String>,
    pub label: String,
}

pub(crate) struct JudgeScore<'a> {
    pub name: &'static str,
    /// Labelled lines this judge answered for, and how many it agreed on.
    pub labelled: usize,
    pub agree: usize,
    /// Labelled lines left out because the judge did not answer (a failed
    /// call or an omitted mover).
    pub unanswered_labels: usize,
    /// Labelled lines it passed.
    pub passed_labelled: usize,
    /// The same, counting a mover once per trading date across the 8:00
    /// brief and its 8:35 update: what the bar's `READ_BAR` counts.
    pub read: usize,
    /// Lines it passed that the owner marked wrong, with the label.
    pub passed_wrong: Vec<(&'a Record, String)>,
    /// Seeded wrong lines that count (the judge passed the original and
    /// answered for the variant), and how many it caught.
    pub seeded: usize,
    pub caught: usize,
    /// Wrong lines not counted because the judge had rejected the original.
    pub original_rejected: usize,
    /// Seeded rows left out because the judge did not answer.
    pub unanswered_seeds: usize,
    /// (counted, caught) per kind.
    pub by_kind: BTreeMap<Kind, (usize, usize)>,
    /// Controls of lines this judge passed in `run`, answered, and how many
    /// it now rejected (it changed its mind: a flip).
    pub controls: usize,
    pub controls_rejected: usize,
    /// Controls of lines it had rejected in `run`, and how many it rejected
    /// again (consistent rejections, not false rejects).
    pub controls_of_rejected: usize,
    pub rejected_again: usize,
}

impl JudgeScore<'_> {
    pub fn agreement(&self) -> f64 {
        ratio(self.agree, self.labelled)
    }
    pub fn recall(&self) -> f64 {
        ratio(self.caught, self.seeded)
    }
    /// The flip rate: controls of lines it passed in `run` that it rejected.
    pub fn false_rejects(&self) -> f64 {
        ratio(self.controls_rejected, self.controls)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

/// The spec's bar for the chosen judge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bar {
    /// Fewer than `READ_BAR` judge-passed lines labelled: the offline
    /// trial did not reach them (34 eligible movers in the final run); the
    /// rest come in the shadow stage.
    NotYetAssessable,
    Met,
    NotMet,
}

pub(crate) struct Score<'a> {
    pub judges: Vec<JudgeScore<'a>>,
    pub chosen: &'static str,
    pub bar: Bar,
    /// Labels for a line whose text has changed since the page was made.
    pub stale_labels: usize,
    /// Seeded rows made from a line whose text has changed since.
    pub stale_seeds: usize,
}

fn judged(r: &Record) -> bool {
    r.insight.is_some() && r.check.is_empty()
}

/// What the bar counts a line as: a premarket brief and its 8:35 update
/// (`premarket_<date>` and `premarket_<date>-update`) are one post.
fn bar_key(r: &Record) -> (String, String) {
    let post = match r.post.strip_prefix("premarket_") {
        Some(rest) => format!("premarket_{}", rest.strip_suffix("-update").unwrap_or(rest)),
        None => r.post.clone(),
    };
    (post, r.ticker.clone())
}

fn current_key(records: &[Record], post: &str, ticker: &str) -> Option<String> {
    records
        .iter()
        .find(|r| judged(r) && r.post == post && r.ticker == ticker)
        .and_then(|r| r.insight.as_ref().map(|i| line_key(&r.writer_version, i)))
}

/// The trial's own part of the bar: no judge-passed line marked wrong, and
/// seeded recall at least 0.9.
fn trial_ok(c: &JudgeScore) -> bool {
    c.passed_wrong.is_empty() && c.seeded > 0 && c.recall() >= RECALL_BAR
}

pub(crate) fn score<'a>(records: &'a [Record], seeded: &[Seeded], labels: &[Label]) -> Score<'a> {
    let mut stale_labels = 0;
    let labelled: Vec<(&Record, &Label)> = labels
        .iter()
        .filter_map(|l| {
            let r = records
                .iter()
                .find(|r| judged(r) && r.post == l.post && r.ticker == l.ticker)?;
            if l.key
                .as_ref()
                .is_some_and(|k| Some(k) != current_key(records, &l.post, &l.ticker).as_ref())
            {
                stale_labels += 1;
                return None;
            }
            Some((r, l))
        })
        .collect();
    let fresh_seeds: Vec<&Seeded> = seeded
        .iter()
        .filter(|s| {
            current_key(records, &s.post, &s.ticker).as_deref() == Some(s.line_key.as_str())
        })
        .collect();
    let stale_seeds = seeded.len() - fresh_seeds.len();
    let judges: Vec<JudgeScore> = JUDGES
        .iter()
        .map(|&name| {
            let mut s = JudgeScore {
                name,
                labelled: 0,
                agree: 0,
                unanswered_labels: 0,
                passed_labelled: 0,
                read: 0,
                passed_wrong: vec![],
                seeded: 0,
                caught: 0,
                original_rejected: 0,
                unanswered_seeds: 0,
                by_kind: BTreeMap::new(),
                controls: 0,
                controls_rejected: 0,
                controls_of_rejected: 0,
                rejected_again: 0,
            };
            let mut read: Vec<(String, String)> = Vec::new();
            for (r, l) in &labelled {
                let Some(j) = r.judges.get(name) else {
                    continue;
                };
                if !j.answered() {
                    s.unanswered_labels += 1;
                    continue;
                }
                let fine = l.label == "fine";
                s.labelled += 1;
                s.agree += usize::from(j.passed == fine);
                if j.passed {
                    s.passed_labelled += 1;
                    if !read.contains(&bar_key(r)) {
                        read.push(bar_key(r));
                    }
                    if !fine {
                        s.passed_wrong.push((r, l.label.clone()));
                    }
                }
            }
            for v in fresh_seeds.iter().filter(|v| v.check.is_empty()) {
                let Some(j) = v.judges.get(name) else {
                    continue;
                };
                if !j.answered() {
                    s.unanswered_seeds += 1;
                    continue;
                }
                if v.kind == Kind::Control {
                    if v.original_passed.get(name) == Some(&true) {
                        s.controls += 1;
                        s.controls_rejected += usize::from(!j.passed);
                    } else {
                        s.controls_of_rejected += 1;
                        s.rejected_again += usize::from(!j.passed);
                    }
                    continue;
                }
                match v.caught(name) {
                    Some(caught) => {
                        s.seeded += 1;
                        s.caught += usize::from(caught);
                        let k = s.by_kind.entry(v.kind).or_default();
                        k.0 += 1;
                        k.1 += usize::from(caught);
                    }
                    None => s.original_rejected += 1,
                }
            }
            s.read = read.len();
            s
        })
        .collect();
    let mut best = 0;
    for (k, j) in judges.iter().enumerate() {
        let b = &judges[best];
        if j.agreement() > b.agreement()
            || j.agreement() == b.agreement() && j.recall() > b.recall()
        {
            best = k;
        }
    }
    let c = &judges[best];
    // The shadow stage has no seeded lines: there the bar's seeded recall is
    // the trial's (its own report), and this score holds the reading part.
    let ok = if seeded.is_empty() {
        c.passed_wrong.is_empty()
    } else {
        trial_ok(c)
    };
    Score {
        chosen: c.name,
        bar: if c.read < READ_BAR {
            Bar::NotYetAssessable
        } else if ok {
            Bar::Met
        } else {
            Bar::NotMet
        },
        judges,
        stale_labels,
        stale_seeds,
    }
}

/// A refusal reason without its particulars, for counting: the language and
/// the rule ("en: numbers not in the cited evidence"), digits as "N".
pub(crate) fn reason_kind(r: &str) -> String {
    let mut parts = r.splitn(3, ": ");
    let head = [parts.next(), parts.next()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(": ");
    let mut out = String::new();
    let mut digit = false;
    for c in head.chars() {
        if c.is_ascii_digit() {
            if !digit {
                out.push('N');
            }
            digit = true;
        } else {
            digit = false;
            out.push(c);
        }
    }
    out
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

fn quote_line(out: &mut String, r: &Record) {
    if let Some(i) = &r.insight {
        let _ = writeln!(out, "  - EN: **{}** {}", i.en.hook, i.en.watch);
        let _ = writeln!(out, "  - ZH: **{}** {}", i.zh.hook, i.zh.watch);
    }
}

/// `out/report.md`.
pub(crate) fn report(
    records: &[Record],
    posts: &[Value],
    seeded: &[Seeded],
    spend: &[Value],
    labels: &[Label],
) -> String {
    let s = score(records, seeded, labels);
    let mut out = String::new();
    let _ = writeln!(out, "# Mover insights trial: judge score\n");

    let movers: u64 = posts.iter().filter_map(|p| p["movers"].as_u64()).sum();
    let eligible: u64 = posts.iter().filter_map(|p| p["eligible"].as_u64()).sum();
    let written = records.iter().filter(|r| r.insight.is_some()).count();
    let refused: Vec<&Record> = records
        .iter()
        .filter(|r| r.insight.is_some() && !r.check.is_empty())
        .collect();
    let to_judges = records.iter().filter(|r| judged(r)).count();
    let _ = writeln!(out, "## Lines\n");
    let _ = writeln!(out, "- Posts: {}", posts.len());
    let _ = writeln!(
        out,
        "- Eligible movers: {eligible} of {movers} ({})",
        pct(ratio(eligible as usize, movers as usize))
    );
    let _ = writeln!(out, "- Written by the writer: {written} of {eligible}");
    let _ = writeln!(
        out,
        "- Refused by the code checks: {} of {written}; sent to the judges: {to_judges}",
        refused.len()
    );
    for j in JUDGES {
        let n = records
            .iter()
            .filter(|r| r.published_if.get(j) == Some(&true))
            .count();
        let failed = records
            .iter()
            .filter(|r| judged(r) && r.judges.get(j).is_some_and(|x| !x.answered()))
            .count();
        let _ = writeln!(
            out,
            "- Passed by {j}: {n} of {to_judges} ({failed} with no answer: a failed call or an omitted mover)"
        );
    }
    let matched: usize = s
        .judges
        .first()
        .map_or(0, |j| j.labelled + j.unanswered_labels);
    let _ = writeln!(
        out,
        "- Labels: {} read, {matched} matched a judged line, {} for a line whose text has changed since (left out)",
        labels.len(),
        s.stale_labels
    );
    if s.stale_seeds > 0 {
        let _ = writeln!(
            out,
            "- Seeded rows made from an older line: {} (left out; re-run `seed --fresh`)",
            s.stale_seeds
        );
    }

    let _ = writeln!(out, "\n## Judges\n");
    let _ = writeln!(
        out,
        "| Judge | Labelled | Agreement with Rick | Judge-passed lines labelled | Judge-passed lines marked wrong | Seeded recall | Controls flipped | Left out (no answer) |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|");
    for j in &s.judges {
        let _ = writeln!(
            out,
            "| {} | {} | {} ({}/{}) | {} | {} | {} ({}/{}) | {} ({}/{}) | {} labels, {} seeds |",
            j.name,
            j.labelled,
            pct(j.agreement()),
            j.agree,
            j.labelled,
            j.passed_labelled,
            j.passed_wrong.len(),
            pct(j.recall()),
            j.caught,
            j.seeded,
            pct(j.false_rejects()),
            j.controls_rejected,
            j.controls,
            j.unanswered_labels,
            j.unanswered_seeds
        );
    }
    let _ = writeln!(
        out,
        "\nA judge pass counts as agreeing with \"Fine\". Seeded recall counts a wrong line only when that judge passed the original line in `run`; {}. Controls are the original lines sent again beside the wrong ones: rejecting one is a false reject.",
        s.judges
            .iter()
            .map(|j| format!("{} wrong lines were not counted for {} because it had rejected the original", j.original_rejected, j.name))
            .collect::<Vec<_>>()
            .join("; ")
    );

    for j in &s.judges {
        let _ = writeln!(
            out,
            "- {}: rejected again {} of {} controls of lines it had rejected in `run` (consistent, not counted as flips).",
            j.name, j.rejected_again, j.controls_of_rejected
        );
    }

    let _ = writeln!(out, "\n## Seeded wrong lines by kind\n");
    let mut head = "| Kind | Made | Skipped (no genuinely wrong, like-for-like change) | Refused by the checks |"
        .to_string();
    let mut rule = "|---|---|---|---|".to_string();
    for j in JUDGES {
        let _ = write!(head, " {j} caught |");
        rule.push_str("---|");
    }
    let _ = writeln!(out, "{head}\n{rule}");
    for kind in Kind::ALL {
        let made = seeded
            .iter()
            .filter(|v| v.kind == kind && !v.skipped)
            .count();
        let skipped = seeded
            .iter()
            .filter(|v| v.kind == kind && v.skipped)
            .count();
        let refused = seeded
            .iter()
            .filter(|v| v.kind == kind && !v.check.is_empty())
            .count();
        let mut row = format!("| {} | {made} | {skipped} | {refused} |", kind.name());
        for j in &s.judges {
            match j.by_kind.get(&kind) {
                Some((n, c)) => {
                    let _ = write!(row, " {c} of {n} |");
                }
                None => row.push_str(" N/A |"),
            }
        }
        let _ = writeln!(out, "{row}");
    }

    let _ = writeln!(out, "\n### Each seeded line\n");
    let mut any = false;
    for v in seeded.iter().filter(|v| v.kind != Kind::Control) {
        any = true;
        let what = v.change.as_deref().unwrap_or("(no change recorded)");
        let verdicts = if v.skipped {
            "skipped: no genuinely wrong, like-for-like change".to_string()
        } else if !v.check.is_empty() {
            "refused by the checks".to_string()
        } else {
            JUDGES
                .iter()
                .map(|j| {
                    let r = match v.caught(j) {
                        Some(true) => "caught",
                        Some(false) => "missed",
                        None => "not counted",
                    };
                    format!("{j} {r}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(
            out,
            "- {} {} {}: {what} ({verdicts})",
            v.post,
            v.ticker,
            v.kind.name()
        );
    }
    if !any {
        let _ = writeln!(out, "None.");
    }

    let _ = writeln!(out, "\n## Judge-passed lines Rick marked wrong\n");
    for j in &s.judges {
        if j.passed_wrong.is_empty() {
            let _ = writeln!(out, "- {}: none", j.name);
            continue;
        }
        let _ = writeln!(out, "- {}:", j.name);
        for (r, label) in &j.passed_wrong {
            let _ = writeln!(out, "  - {} {} (marked {label})", r.post, r.ticker);
            let mut q = String::new();
            quote_line(&mut q, r);
            for l in q.lines() {
                let _ = writeln!(out, "  {l}");
            }
        }
    }

    let _ = writeln!(out, "\n## What the code checks refused\n");
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for r in &refused {
        let mut kinds: Vec<String> = r.check.iter().map(|c| reason_kind(c)).collect();
        kinds.sort();
        kinds.dedup();
        for k in kinds {
            *reasons.entry(k).or_default() += 1;
        }
    }
    let mut reasons: Vec<(String, usize)> = reasons.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if reasons.is_empty() {
        let _ = writeln!(out, "Nothing.");
    }
    for (k, n) in reasons.iter().take(10) {
        let _ = writeln!(out, "- {k}: {n} lines");
    }

    let _ = writeln!(out, "\n## Spend\n");
    let total: f64 = spend.iter().filter_map(|v| v["usd"].as_f64()).sum();
    let mut by: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    for v in spend {
        let k = format!(
            "{} {}",
            v["role"].as_str().unwrap_or("?"),
            v["model"].as_str().unwrap_or("?")
        );
        let e = by.entry(k).or_default();
        e.0 += 1;
        e.1 += v["usd"].as_f64().unwrap_or(0.0);
    }
    let _ = writeln!(
        out,
        "- Total: ${:.4} over {} calls",
        total + 0.0,
        spend.len()
    );
    for (k, (n, usd)) in &by {
        let _ = writeln!(out, "- {k}: {n} calls, ${usd:.4}");
    }

    let c = s
        .judges
        .iter()
        .find(|j| j.name == s.chosen)
        .expect("chosen judge is scored");
    let _ = writeln!(out, "\n## Recommendation\n");
    let _ = writeln!(
        out,
        "Judge: **{}**, with the higher agreement with Rick's labels ({}; ties broken by seeded recall, {}).",
        c.name,
        pct(c.agreement()),
        pct(c.recall())
    );
    let _ = writeln!(
        out,
        "\nJudge-passed lines Rick labelled for this judge: {}.",
        c.passed_labelled
    );
    if seeded.is_empty() {
        // The shadow stage: the reading part of the bar.
        let _ = writeln!(
            out,
            "\nShadow: {} judge-passed lines read (a mover in the 8:00 brief and its 8:35 update counts once) of the {READ_BAR} the bar needs; {} marked wrong.",
            c.read,
            c.passed_wrong.len()
        );
        let verdict = match s.bar {
            Bar::NotYetAssessable => format!("The bar is not yet assessable: fewer than {READ_BAR} judge-passed lines read."),
            Bar::Met => format!("The reading part of the bar is met: at least {READ_BAR} judge-passed lines read, none marked wrong. Its seeded-recall part is measured by the trial's seed run, in the trial's report."),
            Bar::NotMet => format!("The bar is not met: {} judge-passed lines marked wrong.", c.passed_wrong.len()),
        };
        // Lines the Worker's rewrite wrote count toward the bar like the
        // writer's (they are what readers would get); shown apart so the
        // evidence says how much rests on rewrites.
        let rewritten: Vec<&Record> = records
            .iter()
            .filter(|r| r.rewritten_from.is_some())
            .filter(|r| r.judges.get(c.name).is_some_and(|j| j.answered()))
            .collect();
        if !rewritten.is_empty() {
            let passed = |r: &&&Record| r.judges[c.name].passed;
            let labelled = |r: &&&Record| {
                let key = r.insight.as_ref().map(|i| line_key(&r.writer_version, i));
                labels.iter().any(|l| {
                    l.post == r.post
                        && l.ticker == r.ticker
                        && l.key.as_ref().is_none_or(|k| Some(k) == key.as_ref())
                })
            };
            let _ = writeln!(
                out,
                "\nRewritten lines (refused by the code checks, then rewritten once): {} judged, {} passed the judge, {} labelled, {} of them judge-passed (counted in the lines read above).",
                rewritten.len(),
                rewritten.iter().filter(passed).count(),
                rewritten.iter().filter(labelled).count(),
                rewritten.iter().filter(passed).filter(labelled).count(),
            );
        }
        let _ = writeln!(out, "\n{verdict}");
        return out;
    }
    let _ = writeln!(
        out,
        "\nThe trial's own part of the bar (no judge-passed line marked wrong, seeded recall at least {}): {}: {} judge-passed lines marked wrong, seeded recall {}.",
        pct(RECALL_BAR),
        if trial_ok(c) { "holds" } else { "does not hold" },
        c.passed_wrong.len(),
        pct(c.recall())
    );
    match s.bar {
        Bar::NotYetAssessable => {
            let _ = writeln!(
                out,
                "\nThe bar is not yet assessable: it needs at least {READ_BAR} judge-passed lines labelled, and this trial has {}. The rest come from the shadow stage.",
                c.read
            );
        }
        Bar::Met => {
            let _ = writeln!(
                out,
                "\nThe bar is met: {} judge-passed lines labelled, none marked wrong, seeded recall {}.",
                c.passed_labelled,
                pct(c.recall())
            );
        }
        Bar::NotMet => {
            let _ = writeln!(
                out,
                "\nThe bar is not met: {} judge-passed lines labelled, but its trial part does not hold.",
                c.passed_labelled
            );
        }
    }

    let _ = writeln!(out, "\n## Limits of this trial\n");
    let _ = writeln!(
        out,
        "- The calendar and macro-event rows are today's, read for past posts: a moved event shows its new date, a postponed one is missing, and a report date may read \"reported\" for a post written before the report. The live step will read the rows as they stood when each post was built."
    );
    let _ = writeln!(
        out,
        "- Seeded lines are made by code from lines that passed the checks; they test the kinds of error listed above, not every way a line can be wrong."
    );
    let _ = writeln!(
        out,
        "- Rick's labels cover a sample of about 60 lines; agreement on the rest is not measured."
    );
    if records
        .iter()
        .any(|r| r.writer_version == "insight-writer/v4")
    {
        let _ = writeln!(
            out,
            "- Lines written by insight-writer/v4, whose prompt examples came from this trial's own evidence, are in-sample for those examples; the shadow week is the first out-of-sample test."
        );
    }
    let outlet_swaps = seeded
        .iter()
        .filter(|v| v.kind == Kind::OutletSwap && !v.skipped)
        .count();
    if outlet_swaps == 0 {
        let _ = writeln!(
            out,
            "- No line in this run cited two outlets, so the outlet_swap kind was not exercised."
        );
    }
    let per_kind: Vec<String> = Kind::ALL
        .iter()
        .map(|k| {
            let counts: Vec<String> = s
                .judges
                .iter()
                .map(|j| j.by_kind.get(k).map_or(0, |(n, _)| *n).to_string())
                .collect();
            format!("{} {}", k.name(), counts.join(" / "))
        })
        .collect();
    let _ = writeln!(
        out,
        "- Seeds counted per kind ({}): {}. A kind with fewer than 5 counted seeds is anecdotal.",
        JUDGES.join(" / "),
        per_kind.join(", ")
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::tests::{record, verdict};
    use crate::JudgeResult;
    use serde_json::json;

    fn label(r: &Record, label: &str) -> Label {
        Label {
            post: r.post.clone(),
            ticker: r.ticker.clone(),
            key: Some(line_key(&r.writer_version, r.insight.as_ref().unwrap())),
            label: label.into(),
        }
    }

    fn answer(ticker: &str, p: bool) -> JudgeResult {
        JudgeResult {
            verdict: Some(verdict(ticker, p)),
            passed: p,
            error: None,
            retried: false,
        }
    }

    /// Ten labelled lines: Rick marks r0..r7 fine, r8 a wrong fact, r9 advice.
    /// Claude passes r0..r7 and r9 (agrees on 9); DeepSeek passes r0..r4
    /// (agrees on 7) and no line Rick marked wrong.
    fn fixture() -> (Vec<Record>, Vec<Label>) {
        let records: Vec<Record> = (0..10)
            .map(|k| {
                let claude = k != 8;
                let deepseek = k < 5;
                record("recap_2026-11-12", &format!("T{k}"), (claude, deepseek))
            })
            .collect();
        let labels = records
            .iter()
            .enumerate()
            .map(|(k, r)| {
                label(
                    r,
                    match k {
                        8 => "wrong_fact",
                        9 => "advice_or_informal",
                        _ => "fine",
                    },
                )
            })
            .collect();
        (records, labels)
    }

    fn row(r: &Record, kind: Kind, judges: [(&str, JudgeResult); 2]) -> Seeded {
        Seeded {
            post: r.post.clone(),
            ticker: r.ticker.clone(),
            kind,
            insight: r.insight.clone().unwrap(),
            check: vec![],
            judges: judges
                .into_iter()
                .map(|(n, j)| (n.to_string(), j))
                .collect(),
            original_passed: BTreeMap::from([("claude".into(), true), ("deepseek".into(), true)]),
            line_key: line_key(&r.writer_version, r.insight.as_ref().unwrap()),
            skipped: false,
            change: Some(format!("{} change", kind.name())),
        }
    }

    /// Ten judged seeded lines, two per kind, plus one the checks refused,
    /// and two controls. Claude misses one formal_advice line and passes
    /// both controls; DeepSeek catches all and rejects one control.
    fn seeded(r: &Record) -> Vec<Seeded> {
        let t = &r.ticker;
        let mut out: Vec<Seeded> = Kind::ALL
            .iter()
            .flat_map(|&kind| [kind, kind])
            .enumerate()
            .map(|(k, kind)| {
                row(
                    r,
                    kind,
                    [
                        ("claude", answer(t, k == 9)),
                        ("deepseek", answer(t, false)),
                    ],
                )
            })
            .collect();
        out.push(Seeded {
            check: vec!["en: does not name Reuters, the outlet of a cited item".into()],
            judges: BTreeMap::new(),
            kind: Kind::OutletSwap,
            ..out[0].clone()
        });
        out.push(row(
            r,
            Kind::Control,
            [("claude", answer(t, true)), ("deepseek", answer(t, true))],
        ));
        out.push(row(
            r,
            Kind::Control,
            [("claude", answer(t, true)), ("deepseek", answer(t, false))],
        ));
        out
    }

    #[test]
    fn score_reports_agreement_recall_and_the_bar() {
        let (records, labels) = fixture();
        let seeded = seeded(&records[0]);
        let s = score(&records, &seeded, &labels);
        let c = &s.judges[0];
        let d = &s.judges[1];
        assert_eq!(
            (c.name, c.labelled, c.agree, c.passed_labelled),
            ("claude", 10, 9, 9)
        );
        assert_eq!(
            (d.name, d.labelled, d.agree, d.passed_labelled),
            ("deepseek", 10, 7, 5)
        );
        assert_eq!((c.seeded, c.caught), (10, 9));
        assert_eq!((d.seeded, d.caught), (10, 10));
        assert!((c.recall() - 0.9).abs() < 1e-12);
        assert_eq!(c.by_kind[&Kind::FormalAdvice], (2, 1));
        assert_eq!(c.by_kind[&Kind::NumberSwap], (2, 2));
        assert_eq!((c.controls, c.controls_rejected), (2, 0));
        assert_eq!((d.controls, d.controls_rejected), (2, 1));
        assert_eq!(s.chosen, "claude");
        // Claude passed r9, which Rick marked as advice; only 9 lines read.
        assert_eq!(s.bar, Bar::NotYetAssessable);

        // Equal agreement: seeded recall decides.
        let mut records2 = records.clone();
        for r in records2.iter_mut().take(7).skip(5) {
            r.judges.insert("deepseek".into(), answer(&r.ticker, true));
            r.published_if.insert("deepseek".into(), true);
        }
        let s = score(&records2, &seeded, &labels);
        assert_eq!((s.judges[1].agree, s.chosen), (9, "deepseek"));
        // DeepSeek passed nothing Rick marked wrong and caught every seed,
        // but Rick read only 7 lines it passed, not 20: never "met".
        assert_eq!(s.bar, Bar::NotYetAssessable);

        let posts = [json!({"post": "recap_2026-11-12", "movers": 16, "eligible": 10})];
        let spend = [
            json!({"role": "writer", "model": "claude-sonnet-5-5", "usd": 0.02}),
            json!({"role": "judge", "model": "deepseek-flash", "usd": 0.001}),
        ];
        let md = report(&records, &posts, &seeded, &spend, &labels);
        for want in [
            "Eligible movers: 10 of 16 (62%)",
            "| claude | 10 | 90% (9/10) | 9 | 1 | 90% (9/10) | 0% (0/2) | 0 labels, 0 seeds |",
            "| deepseek | 10 | 70% (7/10) | 5 | 0 | 100% (10/10) | 50% (1/2) | 0 labels, 0 seeds |",
            "| outlet_swap | 3 | 0 | 1 | 2 of 2 | 2 of 2 |",
            "| formal_advice | 2 | 0 | 0 | 1 of 2 | 2 of 2 |",
            "Total: $0.0210 over 2 calls",
            "Judge: **claude**",
            "Judge-passed lines Rick labelled for this judge: 9.",
            "does not hold: 1 judge-passed lines marked wrong, seeded recall 90%.",
            "The bar is not yet assessable: it needs at least 20 judge-passed lines labelled, and this trial has 9.",
            "## Limits of this trial",
            "today's, read for past posts",
            "### Each seeded line",
            "recap_2026-11-12 T0 formal_advice: formal_advice change (claude missed, deepseek caught)",
            "Seeds counted per kind (claude / deepseek): number_swap 2 / 2, direction_flip 2 / 2, outlet_swap 2 / 2, zh_only_number 2 / 2, formal_advice 2 / 2. A kind with fewer than 5 counted seeds is anecdotal.",
        ] {
            assert!(md.contains(want), "{want}\n{md}");
        }
    }

    /// Review H2: the bar needs at least 20 judge-passed lines read (50
    /// until Rick lowered it on 2026-11-19).
    #[test]
    fn the_bar_needs_twenty_read_judge_passed_lines() {
        assert_eq!(READ_BAR, 20);
        let records: Vec<Record> = (0..60)
            .map(|k| record("recap_2026-11-12", &format!("T{k}"), (true, true)))
            .collect();
        let t = &records[0].ticker;
        let seeded: Vec<Seeded> = Kind::ALL
            .iter()
            .map(|&k| {
                row(
                    &records[0],
                    k,
                    [("claude", answer(t, false)), ("deepseek", answer(t, false))],
                )
            })
            .collect();
        let labels: Vec<Label> = records.iter().map(|r| label(r, "fine")).collect();
        let s = score(&records, &seeded, &labels);
        assert_eq!(s.judges[0].passed_labelled, 60);
        assert_eq!(s.bar, Bar::Met);
        assert_eq!(score(&records, &seeded, &labels[..20]).bar, Bar::Met);
        let s = score(&records, &seeded, &labels[..19]);
        assert_eq!(s.bar, Bar::NotYetAssessable);
        let md = report(&records, &[], &seeded, &[], &labels[..19]);
        assert!(!md.contains("The bar is met"), "{md}");
        // Enough read, but one passed line marked wrong: not met.
        let mut wrong = labels.clone();
        wrong[0].label = "wrong_fact".into();
        assert_eq!(score(&records, &seeded, &wrong).bar, Bar::NotMet);
        // A judge that passes nothing never meets the bar.
        let none: Vec<Record> = (0..60)
            .map(|k| record("recap_2026-11-12", &format!("T{k}"), (false, false)))
            .collect();
        let s = score(&none, &seeded, &labels);
        assert_ne!(s.bar, Bar::Met);
    }

    /// Review H2: a failed call or an omitted mover is neither a pass nor a
    /// catch, and a wrong line counts only when the judge passed its original.
    #[test]
    fn failures_and_rejected_originals_are_left_out() {
        let (mut records, labels) = fixture();
        let t = records[0].ticker.clone();
        records[0].judges.insert(
            "deepseek".into(),
            JudgeResult {
                verdict: None,
                passed: false,
                error: Some("http status 500".into()),
                retried: false,
            },
        );
        records[1].judges.insert(
            "deepseek".into(),
            JudgeResult {
                verdict: None,
                passed: false,
                error: None,
                retried: false,
            },
        );
        let mut seeds = seeded(&records[0]);
        seeds[0].judges.insert(
            "claude".into(),
            JudgeResult {
                verdict: None,
                passed: false,
                error: Some("parse error".into()),
                retried: false,
            },
        );
        seeds[1].original_passed.insert("claude".into(), false);
        seeds[2].judges.insert("deepseek".into(), answer(&t, true));
        let s = score(&records, &seeds, &labels);
        let (c, d) = (&s.judges[0], &s.judges[1]);
        assert_eq!((d.labelled, d.unanswered_labels), (8, 2));
        assert_eq!(
            (c.seeded, c.unanswered_seeds, c.original_rejected),
            (8, 1, 1)
        );
        assert_eq!((d.seeded, d.caught), (10, 9));
    }

    #[test]
    fn stale_labels_and_seeds_are_left_out() {
        let (records, mut labels) = fixture();
        labels[0].key = Some("0000000000000000".into());
        labels[1].key = None;
        let mut seeds = seeded(&records[0]);
        seeds[0].line_key = "0000000000000000".into();
        let s = score(&records, &seeds, &labels);
        assert_eq!(s.stale_labels, 1);
        assert_eq!(s.judges[0].labelled, 9);
        assert_eq!(s.stale_seeds, 1);
        assert_eq!(s.judges[0].seeded, 9);
        let md = report(&records, &[], &seeds, &[], &labels);
        assert!(
            md.contains("1 for a line whose text has changed since"),
            "{md}"
        );
        assert!(
            md.contains("Seeded rows made from an older line: 1"),
            "{md}"
        );
    }

    #[test]
    fn a_kind_with_no_counted_seed_reads_n_a() {
        let (records, labels) = fixture();
        let seeds: Vec<Seeded> = seeded(&records[0])
            .into_iter()
            .filter(|s| s.kind != Kind::OutletSwap)
            .collect();
        let md = report(&records, &[], &seeds, &[], &labels);
        assert!(
            md.contains("| outlet_swap | 0 | 0 | 0 | N/A | N/A |"),
            "{md}"
        );
    }

    #[test]
    fn score_lists_judge_passed_lines_marked_wrong() {
        let (mut records, labels) = fixture();
        records[9].insight.as_mut().unwrap().en.watch =
            "Reuters reports the shares look cheap.".into();
        let labels: Vec<Label> = labels
            .iter()
            .zip(&records)
            .map(|(l, r)| Label {
                key: Some(line_key(&r.writer_version, r.insight.as_ref().unwrap())),
                ..l.clone()
            })
            .collect();
        let mut refused = record("recap_2026-11-12", "BAD", (false, false));
        refused.check = vec![
            "en: numbers not in the cited evidence: $5.1 billion".into(),
            "zh: numbers not in the cited evidence: 51亿美元".into(),
            "en: hook has 9 words (at most 8)".into(),
        ];
        records.push(refused);
        let md = report(&records, &[], &seeded(&records[0]), &[], &labels);
        let section = md
            .split("## Judge-passed lines Rick marked wrong")
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap();
        assert!(section.contains("- claude:"), "{section}");
        assert!(
            section.contains("recap_2026-11-12 T9 (marked advice_or_informal)"),
            "{section}"
        );
        assert!(
            section.contains("Reuters reports the shares look cheap."),
            "{section}"
        );
        assert!(section.contains("- deepseek: none"), "{section}");
        assert!(!section.contains("T8"), "{section}");
        assert!(
            md.contains("- en: numbers not in the cited evidence: 1 lines"),
            "{md}"
        );
        assert!(
            md.contains("- en: hook has N words (at most N): 1 lines"),
            "{md}"
        );
    }

    /// Round 2, M4: a control of a line the judge passed in `run` and now
    /// rejects is a flip; one it had rejected and rejects again is a
    /// consistent rejection, reported apart.
    #[test]
    fn controls_count_flips_apart_from_consistent_rejections() {
        let (records, labels) = fixture();
        let r = &records[0];
        let t = &r.ticker;
        let mut again = row(
            r,
            Kind::Control,
            [("claude", answer(t, false)), ("deepseek", answer(t, false))],
        );
        again.original_passed.insert("deepseek".into(), false);
        let flipped = row(
            r,
            Kind::Control,
            [("claude", answer(t, true)), ("deepseek", answer(t, false))],
        );
        let s = score(&records, &[again, flipped], &labels);
        let (c, d) = (&s.judges[0], &s.judges[1]);
        assert_eq!((c.controls, c.controls_rejected), (2, 1));
        assert_eq!((d.controls, d.controls_rejected), (1, 1));
        assert_eq!((d.controls_of_rejected, d.rejected_again), (1, 1));
        let md = report(&records, &[], &seeded(r), &[], &labels);
        assert!(
            md.contains("rejected again 0 of 0 controls of lines it had rejected"),
            "{md}"
        );
    }

    /// Round 2, H1: a kind that could only make a true line is counted as
    /// skipped, not made, and never scored.
    #[test]
    fn skipped_seeds_are_counted_and_never_scored() {
        let (records, labels) = fixture();
        let r = &records[0];
        let mut skipped = row(
            r,
            Kind::NumberSwap,
            [
                ("claude", answer(&r.ticker, true)),
                ("deepseek", answer(&r.ticker, true)),
            ],
        );
        skipped.skipped = true;
        skipped.judges.clear();
        let s = score(&records, std::slice::from_ref(&skipped), &labels);
        assert_eq!(s.judges[0].seeded, 0);
        let md = report(&records, &[], &[skipped], &[], &labels);
        assert!(
            md.contains("| number_swap | 0 | 1 | 0 | N/A | N/A |"),
            "{md}"
        );
    }
}
