//! Seeded wrong lines: deliberately broken variants of lines that passed the
//! code checks, to measure how many each judge catches, sent beside the
//! original lines as controls.

use std::collections::BTreeMap;

use prebell_core::insight::check::{self, outlet_names};
use prebell_core::insight::{EvidenceKind, Insight, Pack};
use prebell_core::numbers::{self, Lang, Num};
use serde::{Deserialize, Serialize};

use crate::{line_key, JudgeResult, Record};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    NumberSwap,
    DirectionFlip,
    OutletSwap,
    ZhOnlyNumber,
    FormalAdvice,
    /// The original line, unchanged: a judge should pass it again.
    Control,
}

impl Kind {
    /// The wrong kinds.
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
            Kind::Control => "control",
        }
    }
}

/// One line of `seeded.jsonl`: a wrong variant or a control.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Seeded {
    pub post: String,
    pub ticker: String,
    pub kind: Kind,
    pub insight: Insight,
    /// The code checks' reasons: a variant they refuse is not sent to a judge.
    pub check: Vec<String>,
    /// Each judge's answer, for rows the checks let through.
    pub judges: BTreeMap<String, JudgeResult>,
    /// Whether each judge passed the original line in `run`: a variant
    /// counts against a judge only when it did.
    pub original_passed: BTreeMap<String, bool>,
    /// `line_key` of the original line, to notice a stale file.
    pub line_key: String,
}

impl Seeded {
    /// Whether `judge` caught this wrong line, when it counts: the judge
    /// passed the original, and answered for this one (no failed call, no
    /// omitted mover).
    pub fn caught(&self, judge: &str) -> Option<bool> {
        if self.kind == Kind::Control || self.original_passed.get(judge) != Some(&true) {
            return None;
        }
        self.judges
            .get(judge)
            .filter(|j| j.answered())
            .map(|j| !j.passed)
    }
}

/// Formal clauses that read as advice but avoid every refused word, joined
/// to the watch line with a comma so it stays one sentence.
const ADVICE: [(&str, &str); 5] = [
    (
        "investors may wish to add exposure ahead of that date",
        "投资者不妨在此之前适度布局",
    ),
    (
        "this may be an opportune moment to increase holdings",
        "当前或是介入的合适时机",
    ),
    ("a measured addition appears warranted", "适度介入似属合理"),
    (
        "the shares appear attractively priced at current levels",
        "当前股价似乎颇具吸引力",
    ),
    (
        "reducing exposure before the report may be prudent",
        "在财报公布前降低持股比例或属审慎",
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

/// The outlets of the news items `cites` names, distinct, in cite order.
fn cited_outlets(pack: &Pack, cites: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in cites {
        let outlet = pack
            .evidence
            .iter()
            .find(|e| {
                e.id == *id && matches!(e.kind, EvidenceKind::News | EvidenceKind::AnalystAction)
            })
            .and_then(|e| e.outlet.clone());
        if let Some(o) = outlet.filter(|o| !out.contains(o)) {
            out.push(o);
        }
    }
    out
}

/// A line citing two news items from two outlets, with the outlets' names
/// swapped: every cited outlet is still named, so the checks pass, but each
/// fact is now attributed to the other outlet.
fn outlet_swap(pack: &Pack, i: &Insight) -> Option<Insight> {
    let mut v = i.clone();
    let mut changed = false;
    for (line, lang) in [(&mut v.en, Lang::En), (&mut v.zh, Lang::Zh)] {
        let outlets = cited_outlets(pack, &line.cites);
        let [a, b, ..] = outlets.as_slice() else {
            continue;
        };
        let (names_a, names_b) = (outlet_names(a, lang), outlet_names(b, lang));
        let (Some((at_a, name_a)), Some((at_b, name_b))) = (
            find_name(&line.watch, &names_a),
            find_name(&line.watch, &names_b),
        ) else {
            continue;
        };
        if at_a + name_a.len() > at_b && at_b + name_b.len() > at_a {
            continue;
        }
        let w = &line.watch;
        let ((s1, n1), (s2, n2)) = if at_a < at_b {
            ((at_a, name_a), (at_b, name_b))
        } else {
            ((at_b, name_b), (at_a, name_a))
        };
        line.watch = format!(
            "{}{n2}{}{n1}{}",
            &w[..s1],
            &w[s1 + n1.len()..s2],
            &w[s2 + n2.len()..]
        );
        changed = true;
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
    v.en.watch = format!("{body}, {en}.");
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
            Kind::Control => None,
        };
        if let Some(v) = v.filter(|v| v != i) {
            out.push((kind, v));
        }
    }
    out
}

/// One post's seeded rows and the judge calls that cover them.
pub(crate) struct PostPlan {
    pub post: String,
    /// Controls and variants, each with its pack.
    pub rows: Vec<(Seeded, Pack)>,
    /// Each call's rows (indices into `rows`): at most one row per mover, so
    /// a judge never sees two versions of one mover side by side, and every
    /// mover's original line is in one of the calls as a control.
    pub calls: Vec<Vec<usize>>,
}

/// The seeding plan: for every line that passed the checks in `run`, its
/// control and its wrong variants, per post.
pub(crate) fn plan(records: &[Record]) -> Vec<PostPlan> {
    let mut posts: Vec<&str> = records.iter().map(|r| r.post.as_str()).collect();
    posts.dedup();
    let mut out = Vec::new();
    let mut pick = 0;
    for post in posts {
        let mut rows: Vec<(Seeded, Pack)> = Vec::new();
        let mut sequences: Vec<Vec<usize>> = Vec::new();
        for r in records.iter().filter(|r| r.post == post) {
            let Some(i) = r.insight.as_ref().filter(|_| r.check.is_empty()) else {
                continue;
            };
            let original_passed = r
                .judges
                .iter()
                .map(|(j, res)| (j.clone(), res.passed))
                .collect::<BTreeMap<_, _>>();
            let row = |kind, v: Insight, check: Vec<String>| Seeded {
                post: r.post.clone(),
                ticker: r.ticker.clone(),
                kind,
                insight: v,
                check,
                judges: BTreeMap::new(),
                original_passed: original_passed.clone(),
                line_key: line_key(i),
            };
            let mut sendable = Vec::new();
            for (kind, v) in variants(&r.pack, i, pick) {
                let c = check::check(&r.pack, &v);
                if c.is_empty() {
                    sendable.push(rows.len());
                }
                rows.push((row(kind, v, c), r.pack.clone()));
            }
            pick += 1;
            // The control goes in a different round for each mover.
            let at = sequences.len() % (sendable.len() + 1);
            sendable.insert(at, rows.len());
            rows.push((row(Kind::Control, i.clone(), vec![]), r.pack.clone()));
            sequences.push(sendable);
        }
        if rows.is_empty() {
            continue;
        }
        let rounds = sequences.iter().map(Vec::len).max().unwrap_or(0);
        let calls = (0..rounds)
            .map(|k| sequences.iter().filter_map(|s| s.get(k).copied()).collect())
            .collect();
        out.push(PostPlan {
            post: post.to_string(),
            rows,
            calls,
        });
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
            quote: None,
            unconfirmed: false,
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

    fn insight_citing(en: &str, zh: &str, cites: &[&str]) -> Insight {
        let l = |hook: &str, watch: &str| Line {
            hook: hook.into(),
            watch: watch.into(),
            cites: cites.iter().map(|c| c.to_string()).collect(),
        };
        Insight {
            ticker: "AAPL".into(),
            en: l("Guidance sets the test", en),
            zh: l("指引成为考验", zh),
        }
    }

    fn insight(en: &str, zh: &str) -> Insight {
        insight_citing(en, zh, &["e2"])
    }

    fn base() -> Insight {
        insight(
            "Reuters reports Apple raised its revenue forecast to $4.2 billion alongside a $1.0 billion buyback.",
            "据路透社报道，苹果将营收预期上调至42亿美元，并宣布10亿美元回购。",
        )
    }

    /// A line citing two news items from two outlets.
    fn two_outlets() -> Insight {
        insight_citing(
            "Reuters reports Apple raised its revenue forecast to $4.2 billion, and Bloomberg reports it will build a plant in Texas.",
            "据路透社报道，苹果将营收预期上调至42亿美元，彭博社则报道其将在得州建厂。",
            &["e2", "e3"],
        )
    }

    fn kind_of(i: &Insight, kind: Kind) -> Option<Insight> {
        variants(&pack(), i, 0)
            .into_iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, v)| v)
    }

    fn kind(kind: Kind) -> Insight {
        kind_of(&base(), kind).unwrap_or_else(|| panic!("no {kind:?}"))
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

        let v = kind(Kind::ZhOnlyNumber);
        assert_eq!(v.en, b.en);
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期上调至10亿美元，并宣布10亿美元回购。"
        );

        // Review L3: the advice clause joins with a comma, not a semicolon.
        let v = kind(Kind::FormalAdvice);
        assert_eq!(
            v.en.watch,
            "Reuters reports Apple raised its revenue forecast to $4.2 billion alongside a $1.0 billion buyback, investors may wish to add exposure ahead of that date."
        );
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期上调至42亿美元，并宣布10亿美元回购，投资者不妨在此之前适度布局。"
        );

        // Review M1: the outlets of two cited items trade places.
        let o = two_outlets();
        let v = kind_of(&o, Kind::OutletSwap).expect("two outlets");
        assert_eq!(
            v.en.watch,
            "Bloomberg reports Apple raised its revenue forecast to $4.2 billion, and Reuters reports it will build a plant in Texas."
        );
        assert_eq!(
            v.zh.watch,
            "据彭博社报道，苹果将营收预期上调至42亿美元，路透社则报道其将在得州建厂。"
        );
        assert_eq!(v.en.cites, o.en.cites);

        // Every kind still passes the checks, so it reaches the judges.
        assert_eq!(check::check(&pack(), &o), Vec::<String>::new());
        assert_eq!(check::check(&pack(), &b), Vec::<String>::new());
        for (k, v) in [
            (Kind::NumberSwap, kind(Kind::NumberSwap)),
            (Kind::DirectionFlip, kind(Kind::DirectionFlip)),
            (Kind::ZhOnlyNumber, kind(Kind::ZhOnlyNumber)),
            (Kind::FormalAdvice, kind(Kind::FormalAdvice)),
            (Kind::OutletSwap, v),
        ] {
            assert_eq!(check::check(&pack(), &v), Vec::<String>::new(), "{k:?}");
        }
        // Every advice clause passes too.
        for k in 0..ADVICE.len() {
            let v = formal_advice(&b, k);
            assert_eq!(check::check(&pack(), &v), Vec::<String>::new(), "{k}");
        }
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
        // A line citing one outlet has no outlet swap.
        assert!(!kinds.contains(&Kind::OutletSwap));
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
        // No direction word: that kind is skipped.
        let i = insight(
            "Reuters reports a $1.0 billion buyback.",
            "据路透社报道，公司宣布10亿美元回购。",
        );
        let kinds: Vec<Kind> = variants(&pack(), &i, 0)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(!kinds.contains(&Kind::DirectionFlip));
    }

    /// Review M9 and H2: one row per mover per call, each mover's original
    /// line as a control in a different round.
    #[test]
    fn the_plan_mixes_controls_and_never_repeats_a_mover_in_a_call() {
        use crate::page::tests::record;
        let records = vec![
            record("recap_2026-11-12", "T0", (true, true)),
            record("recap_2026-11-12", "T1", (true, false)),
            record("recap_2026-11-13", "T2", (true, true)),
        ];
        let plans = plan(&records);
        assert_eq!(plans.len(), 2);
        let p = &plans[0];
        let kinds = |call: &Vec<usize>| -> Vec<(String, Kind)> {
            call.iter()
                .map(|&k| (p.rows[k].0.ticker.clone(), p.rows[k].0.kind))
                .collect()
        };
        // Each line's only variant the checks pass is the advice clause.
        assert_eq!(
            p.calls.iter().map(kinds).collect::<Vec<_>>(),
            [
                vec![
                    ("T0".to_string(), Kind::Control),
                    ("T1".to_string(), Kind::FormalAdvice)
                ],
                vec![
                    ("T0".to_string(), Kind::FormalAdvice),
                    ("T1".to_string(), Kind::Control)
                ],
            ]
        );
        // The refused variants are rows with no call.
        let sent: Vec<usize> = p.calls.iter().flatten().copied().collect();
        for (k, (row, _)) in p.rows.iter().enumerate() {
            assert_eq!(sent.contains(&k), row.check.is_empty(), "{:?}", row.kind);
        }
        assert!(p.rows[0].0.original_passed["deepseek"]);
        let t1 = p.rows.iter().find(|(r, _)| r.ticker == "T1").unwrap();
        assert!(!t1.0.original_passed["deepseek"]);
        assert_eq!(
            t1.0.line_key,
            line_key(records[1].insight.as_ref().unwrap())
        );
    }
}
