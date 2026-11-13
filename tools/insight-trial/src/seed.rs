//! Seeded wrong lines: deliberately broken variants of lines that passed the
//! code checks, to measure how many each judge catches.

use std::collections::BTreeMap;

use prebell_core::insight::check::{self, outlet_names};
use prebell_core::insight::{EvidenceKind, Insight, Pack};
use prebell_core::numbers::{self, Lang, Num};
use serde::{Deserialize, Serialize};

use crate::{JudgeResult, Record};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    NumberSwap,
    DirectionFlip,
    OutletSwap,
    ZhOnlyNumber,
    FormalAdvice,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::NumberSwap,
        Kind::DirectionFlip,
        Kind::OutletSwap,
        Kind::ZhOnlyNumber,
        Kind::FormalAdvice,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Kind::NumberSwap => "number_swap",
            Kind::DirectionFlip => "direction_flip",
            Kind::OutletSwap => "outlet_swap",
            Kind::ZhOnlyNumber => "zh_only_number",
            Kind::FormalAdvice => "formal_advice",
        }
    }
}

/// One line of `seeded.jsonl`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Seeded {
    pub post: String,
    pub ticker: String,
    pub kind: Kind,
    pub insight: Insight,
    /// The code checks' reasons: a variant they refuse is not sent to a judge.
    pub check: Vec<String>,
    /// Each judge's answer, for variants the checks let through.
    pub judges: BTreeMap<String, JudgeResult>,
}

impl Seeded {
    /// Whether `judge` caught this wrong line (did not pass it).
    pub fn caught(&self, judge: &str) -> Option<bool> {
        self.judges.get(judge).map(|j| !j.passed)
    }
}

/// Formal sentences that read as advice but avoid every refused word,
/// joined to the watch line so it stays one sentence.
const ADVICE: [(&str, &str); 5] = [
    (
        "investors may wish to add exposure ahead of that date",
        "投资者不妨在此之前提高配置",
    ),
    (
        "this may be an opportune moment to increase positions",
        "当前或是增加仓位的合适时机",
    ),
    (
        "a measured addition to positions appears warranted",
        "适度增加仓位似属合理",
    ),
    (
        "the shares appear attractively priced at current levels",
        "当前股价似乎颇具吸引力",
    ),
    (
        "reducing exposure before the report may be prudent",
        "在财报公布前降低仓位或属审慎",
    ),
];

/// Direction words and their opposites, both ways.
const FLIPS_EN: &[(&str, &str)] = &[
    ("raised", "cut"),
    ("raises", "cuts"),
    ("above", "below"),
    ("increase", "decrease"),
    ("increased", "decreased"),
    ("increases", "decreases"),
    ("higher", "lower"),
    ("rose", "fell"),
    ("growth", "decline"),
];
const FLIPS_ZH: &[(&str, &str)] = &[
    ("上调", "下调"),
    ("高于", "低于"),
    ("增长", "下降"),
    ("上涨", "下跌"),
];

/// Byte range of the digits (with "." and ",") inside a found number.
fn core(text: &str, n: &Num) -> (usize, usize) {
    let raw = &text[n.at..n.at + n.raw.len()];
    let start = raw.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    let len = raw[start..]
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(raw.len() - start);
    (n.at + start, n.at + start + len)
}

fn fmt_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Every value written in the cited items of `line_cites`, in base units.
fn cited_values(pack: &Pack, cites: &[String]) -> Vec<f64> {
    pack.evidence
        .iter()
        .filter(|e| cites.contains(&e.id))
        .flat_map(|e| numbers::extract(&e.text, Lang::En))
        .map(|n| n.value.abs())
        .collect()
}

/// `text` with the number `n` rewritten as another cited value in the
/// same unit ("42亿" with $1.0 billion cited reads "10亿"), or, when no
/// other value is cited, with its last digit changed.
fn replace_number(text: &str, n: &Num, others: &[f64]) -> String {
    let (a, b) = core(text, n);
    let other = others
        .iter()
        .map(|v| v / n.multiplier)
        .find(|v| (v - (n.value.abs() / n.multiplier)).abs() > 1e-9 && *v >= 0.01 && *v < 1e6);
    let new = match other {
        Some(v) => fmt_num(v),
        None => {
            let mut digits: Vec<char> = text[a..b].chars().collect();
            if let Some(d) = digits.iter_mut().rev().find(|c| c.is_ascii_digit()) {
                *d = if *d == '9' {
                    '8'
                } else {
                    (*d as u8 + 1) as char
                };
            }
            digits.into_iter().collect()
        }
    };
    format!("{}{new}{}", &text[..a], &text[b..])
}

fn number_swap(pack: &Pack, i: &Insight) -> Option<Insight> {
    let w = &i.en.watch;
    let nums = numbers::extract(w, Lang::En);
    let first = nums.first()?;
    let second = nums
        .iter()
        .find(|n| (n.value.abs() - first.value.abs()).abs() > 1e-9);
    let watch = match second {
        Some(s) => {
            let (a1, b1) = (first.at, first.at + first.raw.len());
            let (a2, b2) = (s.at, s.at + s.raw.len());
            format!(
                "{}{}{}{}{}",
                &w[..a1],
                &w[a2..b2],
                &w[b1..a2],
                &w[a1..b1],
                &w[b2..]
            )
        }
        None => replace_number(w, first, &cited_values(pack, &i.en.cites)),
    };
    let mut v = i.clone();
    v.en.watch = watch;
    Some(v)
}

fn has_word_at(text: &str, at: usize, len: usize) -> bool {
    let joined = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    !joined(text[..at].chars().next_back()) && !joined(text[at + len..].chars().next())
}

/// The first direction word in `text` and its opposite.
fn flip_first(text: &str, pairs: &[(&str, &str)], whole: bool) -> Option<String> {
    let mut best: Option<(usize, &str, &str)> = None;
    for (a, b) in pairs {
        for (from, to) in [(*a, *b), (*b, *a)] {
            if let Some((at, _)) = text
                .match_indices(from)
                .find(|(at, m)| !whole || has_word_at(text, *at, m.len()))
            {
                // Longest match at the earliest place ("increased" before "increase").
                let better = best.is_none_or(|(b_at, b_from, _)| {
                    at < b_at || at == b_at && from.len() > b_from.len()
                });
                if better {
                    best = Some((at, from, to));
                }
            }
        }
    }
    let (at, from, to) = best?;
    Some(format!("{}{to}{}", &text[..at], &text[at + from.len()..]))
}

fn direction_flip(i: &Insight) -> Option<Insight> {
    let mut v = i.clone();
    let en = flip_first(&i.en.watch, FLIPS_EN, true);
    let zh = flip_first(&i.zh.watch, FLIPS_ZH, false);
    if en.is_none() && zh.is_none() {
        return None;
    }
    if let Some(w) = en {
        v.en.watch = w;
    }
    if let Some(w) = zh {
        v.zh.watch = w;
    }
    Some(v)
}

/// The first of `names` in `text`, as (at, name).
fn find_name<'a>(text: &str, names: &'a [String]) -> Option<(usize, &'a str)> {
    names
        .iter()
        .filter_map(|n| text.find(n.as_str()).map(|at| (at, n.as_str())))
        .min_by_key(|(at, n)| (*at, std::cmp::Reverse(n.len())))
}

fn outlet_swap(pack: &Pack, i: &Insight) -> Option<Insight> {
    let outlet_of = |id: &String| {
        pack.evidence
            .iter()
            .find(|e| {
                e.id == *id && matches!(e.kind, EvidenceKind::News | EvidenceKind::AnalystAction)
            })
            .and_then(|e| e.outlet.clone())
    };
    let cited = i.en.cites.iter().find_map(outlet_of)?;
    let other = pack
        .evidence
        .iter()
        .filter_map(|e| e.outlet.clone())
        .find(|o| *o != cited)?;
    let mut v = i.clone();
    let mut changed = false;
    for (line, lang) in [(&mut v.en, Lang::En), (&mut v.zh, Lang::Zh)] {
        if let Some((at, name)) = find_name(&line.watch, &outlet_names(&cited, lang)) {
            let to = outlet_names(&other, lang).remove(0);
            line.watch = format!(
                "{}{to}{}",
                &line.watch[..at],
                &line.watch[at + name.len()..]
            );
            changed = true;
        }
    }
    changed.then_some(v)
}

fn zh_only_number(pack: &Pack, i: &Insight) -> Option<Insight> {
    let n = numbers::extract(&i.zh.watch, Lang::Zh).into_iter().next()?;
    let mut v = i.clone();
    v.zh.watch = replace_number(&i.zh.watch, &n, &cited_values(pack, &i.zh.cites));
    Some(v)
}

fn formal_advice(i: &Insight, pick: usize) -> Insight {
    let (en, zh) = ADVICE[pick % ADVICE.len()];
    let mut v = i.clone();
    let body = i.en.watch.trim_end().trim_end_matches(['.', '!', '?']);
    v.en.watch = format!("{body}; {en}.");
    let body = i.zh.watch.trim_end().trim_end_matches(['。', '！', '？']);
    v.zh.watch = format!("{body}，{zh}。");
    v
}

/// Up to five wrong variants of `i`, one per kind that applies. `pick`
/// chooses the advice sentence.
pub(crate) fn variants(pack: &Pack, i: &Insight, pick: usize) -> Vec<(Kind, Insight)> {
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let v = match kind {
            Kind::NumberSwap => number_swap(pack, i),
            Kind::DirectionFlip => direction_flip(i),
            Kind::OutletSwap => outlet_swap(pack, i),
            Kind::ZhOnlyNumber => zh_only_number(pack, i),
            Kind::FormalAdvice => Some(formal_advice(i, pick)),
        };
        if let Some(v) = v.filter(|v| v != i) {
            out.push((kind, v));
        }
    }
    out
}

/// The variants of every line that passed the checks, grouped by post, each
/// with its own check result. A judge call later covers one post's variants
/// that the checks let through.
pub(crate) fn seed_records(records: &[Record]) -> Vec<(Seeded, Pack)> {
    let mut out = Vec::new();
    for (k, r) in records.iter().enumerate() {
        let Some(i) = r.insight.as_ref().filter(|_| r.check.is_empty()) else {
            continue;
        };
        for (kind, v) in variants(&r.pack, i, k) {
            out.push((
                Seeded {
                    post: r.post.clone(),
                    ticker: r.ticker.clone(),
                    kind,
                    check: check::check(&r.pack, &v),
                    insight: v,
                    judges: BTreeMap::new(),
                },
                r.pack.clone(),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use prebell_core::insight::{Evidence, Line};

    fn ev(id: &str, kind: EvidenceKind, text: &str, outlet: Option<&str>) -> Evidence {
        Evidence {
            id: id.into(),
            kind,
            text: text.into(),
            outlet: outlet.map(str::to_string),
            url: None,
            date: None,
        }
    }

    fn pack() -> Pack {
        Pack {
            ticker: "AAPL".into(),
            name: "Apple".into(),
            evidence: vec![
                ev("e1", EvidenceKind::Move, "Apple (AAPL) rose 4.20% in premarket trading.", None),
                ev(
                    "e2",
                    EvidenceKind::News,
                    "Apple raised its forecast to revenue of $4.2 billion and announced a $1.0 billion buyback.",
                    Some("Reuters"),
                ),
                ev("e3", EvidenceKind::News, "Apple will build a plant in Texas.", Some("Bloomberg")),
            ],
        }
    }

    fn insight(en: &str, zh: &str) -> Insight {
        let l = |hook: &str, watch: &str| Line {
            hook: hook.into(),
            watch: watch.into(),
            cites: vec!["e2".into()],
        };
        Insight {
            ticker: "AAPL".into(),
            en: l("Guidance sets the test", en),
            zh: l("指引成为考验", zh),
        }
    }

    fn base() -> Insight {
        insight(
            "Reuters reports Apple raised its revenue forecast to $4.2 billion alongside a $1.0 billion buyback.",
            "据路透社报道，苹果将营收预期上调至42亿美元，并宣布10亿美元回购。",
        )
    }

    fn kind(kind: Kind) -> Insight {
        variants(&pack(), &base(), 0)
            .into_iter()
            .find(|(k, _)| *k == kind)
            .unwrap_or_else(|| panic!("no {kind:?}"))
            .1
    }

    #[test]
    fn each_seed_kind_changes_the_line_it_claims_to() {
        let b = base();
        let v = kind(Kind::NumberSwap);
        assert_eq!(
            v.en.watch,
            "Reuters reports Apple raised its revenue forecast to $1.0 billion alongside a $4.2 billion buyback."
        );
        assert_eq!(v.zh, b.zh);

        let v = kind(Kind::DirectionFlip);
        assert_eq!(
            v.en.watch,
            "Reuters reports Apple cut its revenue forecast to $4.2 billion alongside a $1.0 billion buyback."
        );
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期下调至42亿美元，并宣布10亿美元回购。"
        );

        let v = kind(Kind::OutletSwap);
        assert!(
            v.en.watch.starts_with("Bloomberg reports"),
            "{}",
            v.en.watch
        );
        assert!(v.zh.watch.starts_with("据彭博社报道"), "{}", v.zh.watch);
        assert_eq!(v.en.cites, b.en.cites);

        let v = kind(Kind::ZhOnlyNumber);
        assert_eq!(v.en, b.en);
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期上调至10亿美元，并宣布10亿美元回购。"
        );

        let v = kind(Kind::FormalAdvice);
        assert_eq!(
            v.en.watch,
            "Reuters reports Apple raised its revenue forecast to $4.2 billion alongside a $1.0 billion buyback; investors may wish to add exposure ahead of that date."
        );
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期上调至42亿美元，并宣布10亿美元回购，投资者不妨在此之前提高配置。"
        );
        // The checks let the swapped number, the flipped direction, the
        // zh-only number and the advice through; the swapped outlet no
        // longer names the cited item's outlet.
        for k in [
            Kind::NumberSwap,
            Kind::DirectionFlip,
            Kind::ZhOnlyNumber,
            Kind::FormalAdvice,
        ] {
            assert_eq!(
                check::check(&pack(), &kind(k)),
                Vec::<String>::new(),
                "{k:?}"
            );
        }
        assert!(!check::check(&pack(), &kind(Kind::OutletSwap)).is_empty());
    }

    #[test]
    fn a_seed_with_no_number_to_swap_is_skipped() {
        let i = insight(
            "Reuters reports Apple raised its forecast, a plan to track at its report.",
            "据路透社报道，苹果上调了预期，后续财报将检验这一计划。",
        );
        let kinds: Vec<Kind> = variants(&pack(), &i, 1)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(!kinds.contains(&Kind::NumberSwap));
        assert!(!kinds.contains(&Kind::ZhOnlyNumber));
        assert!(kinds.contains(&Kind::DirectionFlip));
        assert!(kinds.contains(&Kind::FormalAdvice));
        // With one number, the swap uses another number of the cited item.
        let i = insight(
            "Reuters reports Apple expects revenue of $4.2 billion at its report.",
            "据路透社报道，苹果预计营收42亿美元。",
        );
        let v = variants(&pack(), &i, 0);
        let swap = &v.iter().find(|(k, _)| *k == Kind::NumberSwap).unwrap().1;
        assert_eq!(
            swap.en.watch,
            "Reuters reports Apple expects revenue of $1 billion at its report."
        );
        // No direction word and no other outlet: those kinds are skipped.
        let mut p = pack();
        p.evidence.truncate(2);
        let i = insight(
            "Reuters reports a $1.0 billion buyback.",
            "据路透社报道，公司宣布10亿美元回购。",
        );
        let kinds: Vec<Kind> = variants(&p, &i, 0).into_iter().map(|(k, _)| k).collect();
        assert!(!kinds.contains(&Kind::DirectionFlip));
        assert!(!kinds.contains(&Kind::OutletSwap));
    }
}
