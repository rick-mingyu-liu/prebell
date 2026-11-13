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
    /// The kind applies to the line but could not make it wrong (every
    /// change would still be true); never sent to a judge.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

impl Seeded {
    /// Whether `judge` caught this wrong line, when it counts: the judge
    /// passed the original, and answered for this one (no failed call, no
    /// omitted mover).
    pub fn caught(&self, judge: &str) -> Option<bool> {
        if self.kind == Kind::Control
            || self.skipped
            || self.original_passed.get(judge) != Some(&true)
        {
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

/// Whether `to` is at least 5% away from `from`: closer, a rounded figure
/// can still be true ("$18.5 billion" for a cited "$18.52 billion").
fn far(from: f64, to: f64) -> bool {
    from != 0.0 && (to - from).abs() / from.abs() >= 0.05
}

fn round_to(v: f64, decimals: u8) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (v * scale).round() / scale
}

/// `text` with the number `n` rewritten as another cited value in the same
/// unit ("42亿" with $1.0 billion cited reads "10亿"), at least 5% away at
/// the line's own precision; `None` when no cited value is that far.
fn replace_number(text: &str, n: &Num, others: &[f64]) -> Option<String> {
    let (a, b) = core(text, n);
    let cur = round_to(n.value.abs() / n.multiplier, n.decimals);
    let v = others
        .iter()
        .map(|v| v / n.multiplier)
        .find(|v| *v >= 0.01 && *v < 1e6 && far(cur, round_to(*v, n.decimals)))?;
    Some(format!("{}{}{}", &text[..a], fmt_num(v), &text[b..]))
}

/// Whether two numbers are the ends of one range ("$1.07 to $1.09"):
/// swapping them changes nothing a reader would notice.
fn range_ends(text: &str, a: &Num, b: &Num) -> bool {
    let between = text[a.at + a.raw.len()..b.at]
        .trim()
        .trim_start_matches('$');
    matches!(between, "to" | "-" | "–" | "—" | "至" | "到" | "and" | "or") || a.from_range
}

/// What trying one kind on a line gave.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Made {
    /// A line that is wrong.
    Wrong(Insight),
    /// The kind applies, but every change it could make would still be
    /// true (a figure within 5%, a range's ends, "higher or lower").
    WouldBeTrue,
    /// Nothing in the line for this kind to change.
    NotApplicable,
}

/// The first number of `nums` that can be replaced by a cited value far
/// enough from it.
fn replace_any(text: &str, nums: &[Num], others: &[f64]) -> Option<String> {
    nums.iter().find_map(|n| replace_number(text, n, others))
}

fn number_swap(pack: &Pack, i: &Insight) -> Made {
    let w = &i.en.watch;
    let nums = numbers::extract(w, Lang::En);
    if nums.is_empty() {
        return Made::NotApplicable;
    }
    // Two numbers of the line trade places, when they are far apart and
    // not one range's ends.
    let pair = nums.iter().enumerate().find_map(|(k, a)| {
        nums[k + 1..]
            .iter()
            .find(|b| far(a.value.abs(), b.value.abs()) && !range_ends(w, a, b))
            .map(|b| (a, b))
    });
    let watch = match pair {
        Some((first, s)) => {
            let (a1, b1) = (first.at, first.at + first.raw.len());
            let (a2, b2) = (s.at, s.at + s.raw.len());
            Some(format!(
                "{}{}{}{}{}",
                &w[..a1],
                &w[a2..b2],
                &w[b1..a2],
                &w[a1..b1],
                &w[b2..]
            ))
        }
        None => replace_any(w, &nums, &cited_values(pack, &i.en.cites)),
    };
    match watch {
        Some(watch) => {
            let mut v = i.clone();
            v.en.watch = watch;
            Made::Wrong(v)
        }
        None => Made::WouldBeTrue,
    }
}

fn has_word_at(text: &str, at: usize, len: usize) -> bool {
    let joined = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    !joined(text[..at].chars().next_back()) && !joined(text[at + len..].chars().next())
}

fn holds(text: &str, word: &str, whole: bool) -> bool {
    text.match_indices(word)
        .any(|(at, m)| !whole || has_word_at(text, at, m.len()))
}

/// The first direction word in `text` and its opposite: `Ok(None)` when
/// there is none, `Err(())` when the line already holds both words of its
/// pair ("higher or lower"), so a flip would say the same thing.
#[allow(clippy::result_unit_err)]
fn flip_first(text: &str, pairs: &[(&str, &str)], whole: bool) -> Result<Option<String>, ()> {
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
    let Some((at, from, to)) = best else {
        return Ok(None);
    };
    if holds(text, to, whole) {
        return Err(());
    }
    Ok(Some(format!(
        "{}{to}{}",
        &text[..at],
        &text[at + from.len()..]
    )))
}

fn direction_flip(i: &Insight) -> Made {
    let mut v = i.clone();
    let en = flip_first(&i.en.watch, FLIPS_EN, true);
    let zh = flip_first(&i.zh.watch, FLIPS_ZH, false);
    match (&en, &zh) {
        (Ok(None), Ok(None)) => return Made::NotApplicable,
        (Ok(None) | Err(()), Ok(None) | Err(())) => return Made::WouldBeTrue,
        _ => {}
    }
    if let Ok(Some(w)) = en {
        v.en.watch = w;
    }
    if let Ok(Some(w)) = zh {
        v.zh.watch = w;
    }
    Made::Wrong(v)
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

fn zh_only_number(pack: &Pack, i: &Insight) -> Made {
    let nums = numbers::extract(&i.zh.watch, Lang::Zh);
    if nums.is_empty() {
        return Made::NotApplicable;
    }
    match replace_any(&i.zh.watch, &nums, &cited_values(pack, &i.zh.cites)) {
        Some(w) => {
            let mut v = i.clone();
            v.zh.watch = w;
            Made::Wrong(v)
        }
        None => Made::WouldBeTrue,
    }
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

/// Every wrong kind tried on `i`. `pick` chooses the advice sentence.
pub(crate) fn attempts(pack: &Pack, i: &Insight, pick: usize) -> Vec<(Kind, Made)> {
    Kind::ALL
        .iter()
        .map(|&kind| {
            let made = match kind {
                Kind::NumberSwap => number_swap(pack, i),
                Kind::DirectionFlip => direction_flip(i),
                Kind::OutletSwap => outlet_swap(pack, i).map_or(Made::NotApplicable, Made::Wrong),
                Kind::ZhOnlyNumber => zh_only_number(pack, i),
                Kind::FormalAdvice => Made::Wrong(formal_advice(i, pick)),
                Kind::Control => Made::NotApplicable,
            };
            let made = match made {
                Made::Wrong(v) if v == *i => Made::WouldBeTrue,
                m => m,
            };
            (kind, made)
        })
        .collect()
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
                skipped: false,
            };
            let mut sendable = Vec::new();
            for (kind, made) in attempts(&r.pack, i, pick) {
                match made {
                    Made::Wrong(v) => {
                        let c = check::check(&r.pack, &v);
                        if c.is_empty() {
                            sendable.push(rows.len());
                        }
                        rows.push((row(kind, v, c), r.pack.clone()));
                    }
                    // Kept as a row so the report can count it.
                    Made::WouldBeTrue => {
                        let mut skipped = row(kind, i.clone(), vec![]);
                        skipped.skipped = true;
                        rows.push((skipped, r.pack.clone()));
                    }
                    Made::NotApplicable => {}
                }
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
    use std::collections::BTreeMap;

    /// Up to five wrong variants of `i`, one per kind that can make it wrong.
    fn variants(pack: &Pack, i: &Insight, pick: usize) -> Vec<(Kind, Insight)> {
        attempts(pack, i, pick)
            .into_iter()
            .filter_map(|(k, m)| match m {
                Made::Wrong(v) => Some((k, v)),
                _ => None,
            })
            .collect()
    }

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
        // Refused and skipped rows have no call. One number in the line
        // and no other cited value: the number swaps are skipped, counted.
        assert!(p
            .rows
            .iter()
            .any(|(r, _)| r.kind == Kind::NumberSwap && r.skipped));
        let sent: Vec<usize> = p.calls.iter().flatten().copied().collect();
        for (k, (row, _)) in p.rows.iter().enumerate() {
            assert_eq!(
                sent.contains(&k),
                row.check.is_empty() && !row.skipped,
                "{:?}",
                row.kind
            );
        }
        assert!(p.rows[0].0.original_passed["deepseek"]);
        let t1 = p.rows.iter().find(|(r, _)| r.ticker == "T1").unwrap();
        assert!(!t1.0.original_passed["deepseek"]);
        assert_eq!(
            t1.0.line_key,
            line_key(records[1].insight.as_ref().unwrap())
        );
    }

    fn figure_pack(extra: &[(&str, &str)]) -> Pack {
        let mut p = pack();
        for (id, text) in extra {
            p.evidence
                .push(ev(id, EvidenceKind::CompanyFigure, text, None));
        }
        p
    }

    fn made_kinds(p: &Pack, i: &Insight) -> BTreeMap<Kind, Made> {
        attempts(p, i, 0).into_iter().collect()
    }

    /// Round 2, H1: "$18.5 billion" against a cited "$18.52 billion" is
    /// true at the line's precision, so no number seed is made from it.
    #[test]
    fn a_seeded_number_must_be_genuinely_wrong() {
        let p = figure_pack(&[(
            "e4",
            "Revenue: $18.52 billion, reported for Q1 of fiscal 2027.",
        )]);
        let i = insight_citing(
            "The company reported revenue of $18.5 billion for Q1 of fiscal 2027.",
            "公司公布2027财年第一季度营收185亿美元。",
            &["e4"],
        );
        let m = made_kinds(&p, &i);
        assert_eq!(m[&Kind::NumberSwap], Made::WouldBeTrue);
        assert_eq!(m[&Kind::ZhOnlyNumber], Made::WouldBeTrue);
        // With a figure at least 5% away cited, both are made, and wrong.
        let p = figure_pack(&[
            (
                "e4",
                "Revenue: $18.52 billion, reported for Q1 of fiscal 2027.",
            ),
            (
                "e5",
                "Net income (GAAP): $4.55 billion, reported for Q1 of fiscal 2027.",
            ),
        ]);
        let i = insight_citing(
            "The company reported revenue of $18.5 billion for Q1 of fiscal 2027.",
            "公司公布2027财年第一季度营收185亿美元。",
            &["e4", "e5"],
        );
        let m = made_kinds(&p, &i);
        let Made::Wrong(v) = &m[&Kind::NumberSwap] else {
            panic!("{:?}", m[&Kind::NumberSwap])
        };
        assert_eq!(
            v.en.watch,
            "The company reported revenue of $4.55 billion for Q1 of fiscal 2027."
        );
        let Made::Wrong(v) = &m[&Kind::ZhOnlyNumber] else {
            panic!("{:?}", m[&Kind::ZhOnlyNumber])
        };
        assert_eq!(v.zh.watch, "公司公布2027财年第一季度营收45.5亿美元。");
        assert!(far(18.5, 17.5) && !far(18.5, 18.52) && !far(185.0, 185.2));
    }

    /// Round 2, L6: a range's two ends are never swapped with each other,
    /// and a flip of "higher or lower" says the same thing.
    #[test]
    fn near_true_swaps_and_flips_are_skipped() {
        let p = Pack {
            evidence: vec![ev(
                "e7",
                EvidenceKind::CompanyGuidance,
                "Cisco estimates that GAAP EPS will be $1.07 to $1.09 for the second quarter of fiscal 2027.",
                None,
            )],
            ..pack()
        };
        let i = insight_citing(
            "The company guided EPS of $1.07 to $1.09 for the second quarter of fiscal 2027.",
            "公司预计2027财年第二季度每股收益为1.07至1.09美元。",
            &["e7"],
        );
        let m = made_kinds(&p, &i);
        assert_eq!(m[&Kind::NumberSwap], Made::WouldBeTrue);
        let mut p2 = p.clone();
        p2.evidence.push(ev(
            "e8",
            EvidenceKind::CompanyGuidance,
            "Cisco estimates that GAAP EPS will be $4.38 to $4.44 for fiscal 2027.",
            None,
        ));
        let mut i2 = i.clone();
        i2.en.cites.push("e8".into());
        let Made::Wrong(v) = &made_kinds(&p2, &i2)[&Kind::NumberSwap] else {
            panic!()
        };
        assert!(!v.en.watch.contains("$1.09 to $1.07"), "{}", v.en.watch);
        assert!(v.en.watch.contains("$4.38"), "{}", v.en.watch);

        let i = insight(
            "Reuters reports revenue of $4.2 billion, which could come in higher or lower at the report.",
            "据路透社报道，苹果营收42亿美元，财报数字可能高于或低于这一水平。",
        );
        assert_eq!(
            made_kinds(&pack(), &i)[&Kind::DirectionFlip],
            Made::WouldBeTrue
        );
    }
}
