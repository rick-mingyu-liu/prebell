//! The judge score: agreement with the owner's labels, seeded recall, what
//! the code checks refused, eligibility and spend, ending with the judge to
//! choose and whether the spec's bar is met.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::Value;

use crate::seed::{Kind, Seeded};
use crate::{Record, JUDGES};

/// Seeded recall the chosen judge needs (the spec's bar).
const RECALL_BAR: f64 = 0.9;

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct Label {
    pub post: String,
    pub ticker: String,
    pub label: String,
}

pub(crate) struct JudgeScore<'a> {
    pub name: &'static str,
    /// Labelled lines this judge judged, and how many it agreed on.
    pub labelled: usize,
    pub agree: usize,
    /// Labelled lines it passed.
    pub passed_labelled: usize,
    /// Lines it passed that the owner marked wrong, with the label.
    pub passed_wrong: Vec<(&'a Record, String)>,
    /// Seeded wrong lines it judged, and caught.
    pub seeded: usize,
    pub caught: usize,
    /// (judged, caught) per kind.
    pub by_kind: BTreeMap<Kind, (usize, usize)>,
}

impl JudgeScore<'_> {
    pub fn agreement(&self) -> f64 {
        ratio(self.agree, self.labelled)
    }
    pub fn recall(&self) -> f64 {
        ratio(self.caught, self.seeded)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

pub(crate) struct Score<'a> {
    pub judges: Vec<JudgeScore<'a>>,
    pub chosen: &'static str,
    pub bar_met: bool,
}

fn judged(r: &Record) -> bool {
    r.insight.is_some() && r.check.is_empty()
}

pub(crate) fn score<'a>(records: &'a [Record], seeded: &[Seeded], labels: &[Label]) -> Score<'a> {
    let labelled: Vec<(&Record, &Label)> = labels
        .iter()
        .filter_map(|l| {
            records
                .iter()
                .find(|r| judged(r) && r.post == l.post && r.ticker == l.ticker)
                .map(|r| (r, l))
        })
        .collect();
    let judges: Vec<JudgeScore> = JUDGES
        .iter()
        .map(|&name| {
            let mut s = JudgeScore {
                name,
                labelled: 0,
                agree: 0,
                passed_labelled: 0,
                passed_wrong: vec![],
                seeded: 0,
                caught: 0,
                by_kind: BTreeMap::new(),
            };
            for (r, l) in &labelled {
                let Some(&passed) = r.published_if.get(name) else {
                    continue;
                };
                let fine = l.label == "fine";
                s.labelled += 1;
                s.agree += usize::from(passed == fine);
                if passed {
                    s.passed_labelled += 1;
                    if !fine {
                        s.passed_wrong.push((r, l.label.clone()));
                    }
                }
            }
            for v in seeded {
                if let Some(caught) = v.caught(name) {
                    s.seeded += 1;
                    s.caught += usize::from(caught);
                    let k = s.by_kind.entry(v.kind).or_default();
                    k.0 += 1;
                    k.1 += usize::from(caught);
                }
            }
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
    Score {
        chosen: c.name,
        bar_met: c.passed_wrong.is_empty() && c.seeded > 0 && c.recall() >= RECALL_BAR,
        judges,
    }
}

/// A refusal reason without its particulars, for counting: the language and
/// the rule ("en: numbers not in the cited evidence"), digits as "N".
fn reason_kind(r: &str) -> String {
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
        let _ = writeln!(out, "- Passed by {j}: {n} of {to_judges}");
    }
    let matched: usize = s.judges.first().map_or(0, |j| j.labelled);
    let _ = writeln!(
        out,
        "- Labels: {} read, {matched} matched a judged line",
        labels.len()
    );

    let _ = writeln!(out, "\n## Judges\n");
    let _ = writeln!(
        out,
        "| Judge | Labelled | Agreement with Rick | Judge-passed lines labelled | Judge-passed lines marked wrong | Seeded recall |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for j in &s.judges {
        let _ = writeln!(
            out,
            "| {} | {} | {} ({}/{}) | {} | {} | {} ({}/{}) |",
            j.name,
            j.labelled,
            pct(j.agreement()),
            j.agree,
            j.labelled,
            j.passed_labelled,
            j.passed_wrong.len(),
            pct(j.recall()),
            j.caught,
            j.seeded
        );
    }

    let _ = writeln!(out, "\n## Seeded wrong lines by kind\n");
    let mut head = "| Kind | Refused by the checks |".to_string();
    let mut rule = "|---|---|".to_string();
    for j in JUDGES {
        let _ = write!(head, " {j} caught |");
        rule.push_str("---|");
    }
    let _ = writeln!(out, "{head}\n{rule}");
    for kind in Kind::ALL {
        let refused = seeded
            .iter()
            .filter(|v| v.kind == kind && !v.check.is_empty())
            .count();
        let mut row = format!("| {} | {refused} |", kind.name());
        for j in &s.judges {
            let (n, c) = j.by_kind.get(&kind).copied().unwrap_or_default();
            let _ = write!(row, " {c} of {n} |");
        }
        let _ = writeln!(out, "{row}");
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
    let mut why = Vec::new();
    if !c.passed_wrong.is_empty() {
        why.push(format!(
            "{} judge-passed lines were marked wrong",
            c.passed_wrong.len()
        ));
    }
    if c.seeded == 0 || c.recall() < RECALL_BAR {
        why.push(format!(
            "seeded recall is {} (needs at least {})",
            pct(c.recall()),
            pct(RECALL_BAR)
        ));
    }
    if s.bar_met {
        let _ = writeln!(
            out,
            "\nThe bar is met: no judge-passed line was marked wrong, and seeded recall is {}.",
            pct(c.recall())
        );
    } else {
        let _ = writeln!(out, "\nThe bar is not met: {}.", why.join("; "));
    }
    let _ = writeln!(
        out,
        "\n{} judge-passed lines were labelled with this judge (the spec's shadow-to-on bar asks for at least 50).",
        c.passed_labelled
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::tests::record;
    use crate::JudgeResult;
    use serde_json::json;

    fn label(r: &Record, label: &str) -> Label {
        Label {
            post: r.post.clone(),
            ticker: r.ticker.clone(),
            label: label.into(),
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

    /// Ten judged seeded lines, two per kind, plus one the checks refused.
    /// Claude misses one formal_advice line; DeepSeek catches all.
    fn seeded(r: &Record) -> Vec<Seeded> {
        let j = |p: bool| JudgeResult {
            verdict: None,
            passed: p,
            error: None,
        };
        let mut out: Vec<Seeded> = Kind::ALL
            .iter()
            .flat_map(|&kind| [kind, kind])
            .enumerate()
            .map(|(k, kind)| Seeded {
                post: r.post.clone(),
                ticker: r.ticker.clone(),
                kind,
                insight: r.insight.clone().unwrap(),
                check: vec![],
                judges: BTreeMap::from([
                    ("claude".to_string(), j(k == 9)),
                    ("deepseek".to_string(), j(false)),
                ]),
            })
            .collect();
        out.push(Seeded {
            check: vec!["en: does not name Reuters, the outlet of a cited item".into()],
            judges: BTreeMap::new(),
            kind: Kind::OutletSwap,
            ..out[0].clone()
        });
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
        assert_eq!(s.chosen, "claude");
        // Claude passed r9, which Rick marked as advice.
        assert!(!s.bar_met);

        // Equal agreement: seeded recall decides.
        let mut records2 = records.clone();
        for r in records2.iter_mut().take(7).skip(5) {
            r.published_if.insert("deepseek".into(), true);
        }
        let s = score(&records2, &seeded, &labels);
        assert_eq!((s.judges[1].agree, s.chosen), (9, "deepseek"));
        // DeepSeek passed nothing Rick marked wrong and caught every seed.
        assert!(s.bar_met);

        // Labels for lines that were never judged are not counted.
        let mut labels3 = labels.clone();
        labels3.push(Label {
            post: "recap_2026-11-12".into(),
            ticker: "NOPE".into(),
            label: "fine".into(),
        });
        assert_eq!(score(&records, &seeded, &labels3).judges[0].labelled, 10);

        let posts = [json!({"post": "recap_2026-11-12", "movers": 16, "eligible": 10})];
        let spend = [
            json!({"role": "writer", "model": "claude-sonnet-5-5", "usd": 0.02}),
            json!({"role": "judge", "model": "deepseek-flash", "usd": 0.001}),
        ];
        let md = report(&records, &posts, &seeded, &spend, &labels);
        for want in [
            "Eligible movers: 10 of 16 (62%)",
            "| claude | 10 | 90% (9/10) | 9 | 1 | 90% (9/10) |",
            "| deepseek | 10 | 70% (7/10) | 5 | 0 | 100% (10/10) |",
            "| outlet_swap | 1 | 2 of 2 | 2 of 2 |",
            "| formal_advice | 0 | 1 of 2 | 2 of 2 |",
            "Total: $0.0210 over 2 calls",
            "Judge: **claude**",
            "The bar is not met: 1 judge-passed lines were marked wrong.",
        ] {
            assert!(md.contains(want), "{want}\n{md}");
        }
    }

    #[test]
    fn score_lists_judge_passed_lines_marked_wrong() {
        let (mut records, labels) = fixture();
        records[9].insight.as_mut().unwrap().en.watch =
            "Reuters reports the shares look cheap.".into();
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
}
