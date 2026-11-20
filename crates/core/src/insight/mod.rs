//! Mover insights: a short hook and one "watch next" sentence per big mover,
//! in English and Chinese, written by a model from an evidence pack, checked
//! by code (`check`) and then by a judge model that must quote verbatim
//! support (`judge`). `step` runs them in the Worker's `insight` queue job
//! in shadow mode: every line and verdict is stored, readers see nothing.
//! Spec: `docs/superpowers/specs/2026-11-13-mover-insights-design.md`.

use serde::{Deserialize, Serialize};

pub mod check;
pub mod judge;
pub mod pack;
pub mod step;
pub mod write;

/// Patterns the insight code (packs, checks, numbers) has compiled in this
/// process: what a cold isolate pays on its first call.
pub fn compiled() -> usize {
    check::compiled() + pack::compiled() + crate::numbers::compiled()
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Move,
    News,
    AnalystAction,
    CompanyFigure,
    CompanyGuidance,
    Theme,
    ReportDate,
    /// No longer put in packs (trial run 1: a macro release is not a
    /// mover's own thing to track); kept so stored trial lines still parse.
    MacroEvent,
}

/// One item the writer and the judge may rest a line on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Evidence {
    /// "e1", "e2", ... unique within a pack.
    pub id: String,
    pub kind: EvidenceKind,
    /// What the model reads; verbatim source text for `News`,
    /// `AnalystAction` and the `Company*` kinds.
    pub text: String,
    /// English outlet name, `News` and `AnalystAction` only.
    pub outlet: Option<String>,
    pub url: Option<String>,
    pub date: Option<chrono::NaiveDate>,
    /// The verbatim source sentence when `text` is written by code from
    /// verified fields (`CompanyFigure`). Shown to people, never used to
    /// allow a number or confirm support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// A `ReportDate` the company has not confirmed: a line citing it must
    /// hedge it (`check`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconfirmed: bool,
}

/// Everything the writer and the judge see for one mover.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pack {
    pub ticker: String,
    pub name: String,
    pub evidence: Vec<Evidence>,
}

/// One language's hook and watch line, with the evidence ids it rests on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub hook: String,
    pub watch: String,
    pub cites: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Insight {
    pub ticker: String,
    pub en: Line,
    pub zh: Line,
}

#[cfg(test)]
mod tests {
    use super::judge::{self, Verdict};
    use super::step::{self, Record};
    use super::*;
    use crate::cover::Session;
    use crate::earnings::facts::EarningsFacts;
    use crate::facts::{NewsReason, PremarketFacts};
    use crate::recap::RecapFacts;
    use serde_json::Value;

    /// The lazily compiled statics a source file declares, outside its tests.
    fn statics(source: &str) -> usize {
        source
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .matches("OnceLock<")
            .count()
    }

    /// A new lazily compiled pattern must be added to its module's `warm`:
    /// each `warm` touches one entry per `OnceLock` in its file.
    #[test]
    fn warm_covers_every_static() {
        assert_eq!(statics(include_str!("check.rs")), check::warm());
        assert_eq!(statics(include_str!("pack.rs")), pack::warm());
        assert_eq!(
            statics(include_str!("../numbers.rs")),
            crate::numbers::warm()
        );
    }

    /// Lines, packs and verdicts the trial stored, for the probes.
    fn corpus() -> Vec<(Pack, Option<Insight>, Vec<Verdict>)> {
        let mut out = Vec::new();
        for l in include_str!("testdata/run2_records.jsonl")
            .lines()
            .chain(include_str!("testdata/run3_records.jsonl").lines())
            .chain(
                include_str!("../../../../tools/insight-trial/testdata/final_run_lines.jsonl")
                    .lines(),
            )
        {
            let v: Value = serde_json::from_str(l).unwrap();
            let verdicts = v["judges"]
                .as_object()
                .map(|j| {
                    j.values()
                        .filter_map(|r| {
                            serde_json::from_value::<Verdict>(r["verdict"].clone()).ok()
                        })
                        .map(|mut v| {
                            v.personal_claims_attributed = true;
                            v
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push((
                serde_json::from_value(v["pack"].clone()).unwrap(),
                serde_json::from_value(v["insight"].clone()).ok(),
                verdicts,
            ));
        }
        let rec: Record =
            serde_json::from_str(include_str!("testdata/shadow/record.json")).unwrap();
        for m in rec.movers {
            out.push((m.pack, m.insight, m.verdict.into_iter().collect()));
        }
        out
    }

    /// The work of one step over every stored input, after its `warm_*`.
    fn probe(step_name: &str) {
        let before = |f: fn() -> usize| f();
        match step_name {
            "write" => {
                step::warm_write();
                let c0 = compiled();
                let recap: RecapFacts =
                    serde_json::from_str(include_str!("testdata/shadow/recap_2026-11-06.json"))
                        .unwrap();
                let pre: PremarketFacts =
                    serde_json::from_str(include_str!("testdata/shadow/premarket_2026-11-12.json"))
                        .unwrap();
                let checks: Vec<EarningsFacts> = [
                    include_str!("testdata/shadow/earnings_AMD_0000002488-26-000188.json"),
                    include_str!("testdata/shadow/earnings_CHTR_0001091667-26-000072.json"),
                ]
                .iter()
                .map(|t| serde_json::from_str(t).unwrap())
                .collect();
                for (session, td, as_of, movers, themes) in [
                    (
                        Session::Recap,
                        recap.trading_date,
                        recap.as_of_utc,
                        [recap.gainers.clone(), recap.losers.clone()].concat(),
                        recap.themes.clone(),
                    ),
                    (
                        Session::Premarket,
                        pre.trading_date,
                        pre.as_of_utc,
                        [pre.gainers.clone(), pre.losers.clone()].concat(),
                        pre.themes.clone(),
                    ),
                ] {
                    pack::packs(&pack::PackInputs {
                        session,
                        trading_date: td,
                        as_of_utc: as_of,
                        movers: &movers,
                        themes: &themes,
                        checks: &checks,
                        calendar: &[],
                    });
                }
                for line in include_str!("testdata/reasons_2026_11.tsv").lines().skip(1) {
                    let quote = line.splitn(3, '\t').nth(2).unwrap_or("");
                    let r = NewsReason {
                        url: "https://www.reuters.com/x".into(),
                        title: quote.into(),
                        quote: quote.into(),
                        summary_en: quote.into(),
                    };
                    pack::is_analyst_action(&r);
                    pack::plain(quote);
                    pack::names_period(quote);
                }
                assert_eq!(
                    compiled(),
                    c0,
                    "the write step compiled a pattern its warm missed"
                );
                // Neither the checks' nor the numbers' patterns.
                assert_eq!(before(check::compiled), 0);
                assert_eq!(before(crate::numbers::compiled), 0);
            }
            "rewrite" => {
                // The rewrite message builds its prompt and stores the reply:
                // no pattern at all.
                let mut n = 0;
                for (p, i, _) in corpus() {
                    if let Some(i) = i {
                        write::rewrite_prompt(&[(p, i, vec!["zh: relative time: 近期".into()])]);
                        n += 1;
                    }
                }
                assert!(n > 20, "{n}");
                assert_eq!(compiled(), 0, "the rewrite step compiled a pattern");
            }
            "check" | "recheck" => {
                // The recheck is the check step's work on the rewrite's reply.
                step::warm_check();
                let c0 = compiled();
                for (p, i, _) in corpus() {
                    if let Some(i) = i {
                        let reply = serde_json::json!({"insights": [i]});
                        for i in write::parse(&reply, std::slice::from_ref(&p)) {
                            check::check(&p, &i.unwrap());
                        }
                    }
                }
                assert_eq!(
                    compiled(),
                    c0,
                    "the check step compiled a pattern its warm missed"
                );
                assert_eq!(before(pack::compiled), 0);
            }
            "judge" => {
                step::warm_judge();
                let c0 = compiled();
                let mut n = 0;
                for (p, i, vs) in corpus() {
                    for v in vs {
                        if let Some(i) = &i {
                            judge::passed(&p, i, &v);
                            n += 1;
                        }
                    }
                }
                assert!(n > 20, "{n}");
                assert_eq!(
                    compiled(),
                    c0,
                    "the judge step compiled a pattern its warm missed"
                );
                assert_eq!(before(pack::compiled), 0);
            }
            other => panic!("unknown probe {other}"),
        }
    }

    /// Runs in a child process (`each_step_compiles_only_what_it_warmed`),
    /// where no other test has compiled anything yet.
    #[test]
    fn warm_probe() {
        if let Ok(s) = std::env::var("INSIGHT_WARM_PROBE") {
            probe(&s);
        }
    }

    /// Each step, in a fresh process: its `warm_*` compiles every pattern its
    /// work over all stored inputs uses (none compiled after), and none of
    /// another step's groups. So a paying step compiles before it pays, and a
    /// message never compiles a group it does not need.
    #[test]
    fn each_step_compiles_only_what_it_warmed() {
        let exe = std::env::current_exe().unwrap();
        for s in ["write", "check", "rewrite", "recheck", "judge"] {
            let out = std::process::Command::new(&exe)
                .args(["--exact", "insight::tests::warm_probe", "--test-threads=1"])
                .env("INSIGHT_WARM_PROBE", s)
                .output()
                .unwrap();
            assert!(
                out.status.success() && String::from_utf8_lossy(&out.stdout).contains("1 passed"),
                "{s}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
