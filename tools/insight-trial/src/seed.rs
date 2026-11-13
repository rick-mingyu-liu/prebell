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
    /// What the variant changed ("$28.00 ↔ $7.54", "raised → cut").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
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
        "so investors may wish to add exposure",
        "投资者不妨适度布局",
    ),
    (
        "making this an opportune moment to add holdings",
        "当前或是介入良机",
    ),
    ("so a measured addition looks warranted", "适度介入似属合理"),
    (
        "and the shares look attractively priced",
        "当前股价颇具吸引力",
    ),
    (
        "so trimming exposure first may be prudent",
        "财报前降低持股比例或属审慎",
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

/// What a figure measures. A number seed only exchanges or replaces like
/// with like; a bare number (a year, a count, a label, the digits of "24/7
/// Wall St.") has no class and is never changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// An amount of money with a scale ("$4.2 billion", "42亿美元").
    Money,
    /// Money without a scale: a per-share figure or a price ("$28.00").
    PerShare,
    Percent,
    /// Percentage points ("7.2 percentage points", "7.2个百分点").
    Points,
}

fn class(text: &str, n: &Num) -> Option<Class> {
    let after = text[n.at + n.raw.len()..].trim_start().to_ascii_lowercase();
    if n.is_percent {
        return Some(Class::Percent);
    }
    if after.starts_with("percentage point") || after.starts_with("个百分点") {
        return Some(Class::Points);
    }
    let money = n.raw.contains('$') || after.starts_with("美元") || after.starts_with("dollar");
    money.then_some(if n.multiplier >= 1e3 {
        Class::Money
    } else {
        Class::PerShare
    })
}

/// The classed figures written in `text`.
fn figures(text: &str, lang: Lang) -> Vec<(Num, Class)> {
    numbers::extract(text, lang)
        .into_iter()
        .filter_map(|n| class(text, &n).map(|c| (n, c)))
        .collect()
}

/// Every classed value written in the cited items, in base units.
fn cited_values(pack: &Pack, cites: &[String]) -> Vec<(f64, Class)> {
    pack.evidence
        .iter()
        .filter(|e| cites.contains(&e.id))
        .flat_map(|e| figures(&e.text, Lang::En))
        .map(|(n, c)| (n.value.abs(), c))
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

/// The same value, written in either language ("$18.52 billion", "185.2亿").
fn same_value(a: f64, b: f64) -> bool {
    let (a, b) = (a.abs(), b.abs());
    a.max(b) > 0.0 && (a - b).abs() / a.max(b) < 0.006
}

/// `text` with `n`'s digits replaced by `v` in `n`'s own unit, and the old
/// and new figure as written.
fn rewrite(text: &str, n: &Num, v: f64) -> (String, String, String) {
    let (a, b) = core(text, n);
    let new = format!("{}{}{}", &text[..a], fmt_num(v / n.multiplier), &text[b..]);
    let old_raw = n.raw.clone();
    let new_raw = format!(
        "{}{}{}",
        &text[n.at..a],
        fmt_num(v / n.multiplier),
        &text[b..n.at + n.raw.len()]
    );
    (new, old_raw, new_raw)
}

/// A cited value of `n`'s class, at least 5% away from it at the line's
/// own precision.
fn replacement(n: &Num, c: Class, cited: &[(f64, Class)]) -> Option<f64> {
    let cur = round_to(n.value.abs() / n.multiplier, n.decimals);
    cited
        .iter()
        .filter(|(_, k)| *k == c)
        .map(|(v, _)| *v)
        .find(|v| {
            let x = v / n.multiplier;
            (0.01..1e6).contains(&x) && far(cur, round_to(x, n.decimals))
        })
}

/// `text` with the figures `a` and `b` trading places, at every place each
/// is written ("$28.00" twice becomes "$7.54" twice), so the line stays
/// consistent with itself.
fn swapped(text: &str, figs: &[(Num, Class)], a: &Num, b: &Num) -> String {
    let (ra, rb) = (
        text[a.at..a.at + a.raw.len()].to_string(),
        text[b.at..b.at + b.raw.len()].to_string(),
    );
    let mut spans: Vec<(usize, usize, &str)> = figs
        .iter()
        .filter_map(|(n, _)| {
            let to = if same_value(n.value, a.value) {
                rb.as_str()
            } else if same_value(n.value, b.value) {
                ra.as_str()
            } else {
                return None;
            };
            Some((n.at, n.at + n.raw.len(), to))
        })
        .collect();
    spans.sort_by_key(|s| s.0);
    let mut out = String::new();
    let mut last = 0;
    for (s, e, to) in spans {
        out.push_str(&text[last..s]);
        out.push_str(to);
        last = e;
    }
    out.push_str(&text[last..]);
    out
}

/// Whether `n` is one end of a range written in `text` ("38% to 40%").
fn in_range(text: &str, figs: &[(Num, Class)], n: &Num) -> bool {
    figs.iter()
        .any(|(m, _)| m.at != n.at && range_ends(text, m, n))
}

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

/// Whether two numbers are the ends of one range ("$1.07 to $1.09"):
/// swapping them changes nothing a reader would notice.
fn range_ends(text: &str, a: &Num, b: &Num) -> bool {
    let (a, b) = if a.at < b.at { (a, b) } else { (b, a) };
    let between = text[a.at + a.raw.len()..b.at]
        .trim()
        .trim_start_matches('$');
    matches!(between, "to" | "-" | "–" | "—" | "至" | "到" | "and" | "or") || a.from_range
}

/// What trying one kind on a line gave.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Made {
    /// A line that is wrong, and what was changed ("$28.00 ↔ $7.54").
    Wrong(Insight, String),
    /// The kind applies, but no change it could make would be genuinely
    /// wrong and like for like (a figure within 5%, a range's ends, only a
    /// year to swap, "higher or lower").
    WouldBeTrue,
    /// Nothing in the line for this kind to change.
    NotApplicable,
}

/// The Chinese figure that writes the same value as `n`.
fn counterpart<'a>(zh: &'a [(Num, Class)], n: &Num, c: Class) -> Option<&'a Num> {
    zh.iter()
        .find(|(z, k)| *k == c && same_value(z.value, n.value))
        .map(|(z, _)| z)
}

/// Two figures of one class trade places in both languages ("$28.00 ↔
/// $7.54"), or, failing that, one figure becomes another cited value of its
/// class in both languages. Never a year, a date, a label or a range's two
/// ends; skipped (and counted) when no such change exists.
fn number_swap(pack: &Pack, i: &Insight) -> Made {
    let (w, z) = (&i.en.watch, &i.zh.watch);
    let en = figures(w, Lang::En);
    let zh = figures(z, Lang::Zh);
    if numbers::extract(w, Lang::En).is_empty() {
        return Made::NotApplicable;
    }
    for (k, (a, ca)) in en.iter().enumerate() {
        for (b, cb) in &en[k + 1..] {
            if ca != cb || !far(a.value.abs(), b.value.abs()) || range_ends(w, a, b) {
                continue;
            }
            let (Some(za), Some(zb)) = (counterpart(&zh, a, *ca), counterpart(&zh, b, *cb)) else {
                continue;
            };
            if za.at == zb.at || range_ends(z, za, zb) {
                continue;
            }
            let mut v = i.clone();
            v.en.watch = swapped(w, &en, a, b);
            v.zh.watch = swapped(z, &zh, za, zb);
            return Made::Wrong(v, format!("{} ↔ {}", a.raw, b.raw));
        }
    }
    // One figure becomes another cited value of its class, never a range's
    // end ("38% to 40%" must not become "40% to 40%").
    let cited = cited_values(pack, &i.en.cites);
    for (n, c) in &en {
        if in_range(w, &en, n) {
            continue;
        }
        let Some(zn) = counterpart(&zh, n, *c) else {
            continue;
        };
        if in_range(z, &zh, zn) {
            continue;
        }
        let Some(value) = replacement(n, *c, &cited) else {
            continue;
        };
        let (en_new, old_raw, new_raw) = rewrite(w, n, value);
        let (zh_new, _, _) = rewrite(z, zn, value);
        let mut v = i.clone();
        v.en.watch = en_new;
        v.zh.watch = zh_new;
        return Made::Wrong(v, format!("{old_raw} → {new_raw}"));
    }
    Made::WouldBeTrue
}

/// The words that differ between `old` and `new`, widened to whole words:
/// "raised → cut".
fn describe(old: &str, new: &str) -> String {
    let pre = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);
    let suf = old[pre..]
        .chars()
        .rev()
        .zip(new[pre..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let start = old[..pre].rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let end_old = old.len() - suf;
    let end_new = new.len() - suf;
    let widen = |s: &str, end: usize| {
        s[end..]
            .find(char::is_whitespace)
            .map_or(s.len(), |i| end + i)
    };
    let (eo, en) = (widen(old, end_old.max(pre)), widen(new, end_new.max(pre)));
    format!("{} → {}", old[start..eo].trim(), new[start..en].trim())
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
    let change = if v.en.watch != i.en.watch {
        describe(&i.en.watch, &v.en.watch)
    } else {
        describe(&i.zh.watch, &v.zh.watch)
    };
    Made::Wrong(v, change)
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

/// One Chinese figure becomes another cited value of its class; the English
/// line is unchanged, so the two languages differ.
fn zh_only_number(pack: &Pack, i: &Insight) -> Made {
    let z = &i.zh.watch;
    if numbers::extract(z, Lang::Zh).is_empty() {
        return Made::NotApplicable;
    }
    let figs = figures(z, Lang::Zh);
    let cited = cited_values(pack, &i.zh.cites);
    for (n, c) in figs.iter().filter(|(n, _)| !in_range(z, &figs, n)) {
        if let Some(value) = replacement(n, *c, &cited) {
            let (zh_new, old_raw, new_raw) = rewrite(z, n, value);
            let mut v = i.clone();
            v.zh.watch = zh_new;
            return Made::Wrong(v, format!("{old_raw} → {new_raw} (Chinese only)"));
        }
    }
    Made::WouldBeTrue
}

/// Where `body`'s closing clause (after the last `sep`) starts, when what
/// is before it still says something.
fn closing_clause(body: &str, sep: &str, min_chars: usize) -> Option<usize> {
    body.rfind(sep)
        .filter(|at| body[..*at].chars().count() >= min_chars)
}

/// The line with an advice clause in place of its closing clause in both
/// languages, or appended to both when either has no closing clause to
/// replace. Replacing keeps the line within its length limit (in the final
/// run 11 of 15 appended advice seeds were refused for length), and doing
/// the same in both keeps the two languages saying the same thing.
fn formal_advice(i: &Insight, pick: usize) -> Insight {
    let (en, zh) = ADVICE[pick % ADVICE.len()];
    let mut v = i.clone();
    let en_body = i.en.watch.trim_end().trim_end_matches(['.', '!', '?']);
    let zh_body = i.zh.watch.trim_end().trim_end_matches(['。', '！', '？']);
    match (
        closing_clause(en_body, ", ", 60),
        closing_clause(zh_body, "，", 20),
    ) {
        (Some(a), Some(b)) => {
            v.en.watch = format!("{}, {en}.", &en_body[..a]);
            v.zh.watch = format!("{}，{zh}。", &zh_body[..b]);
        }
        _ => {
            v.en.watch = format!("{en_body}, {en}.");
            v.zh.watch = format!("{zh_body}，{zh}。");
        }
    }
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
                Kind::OutletSwap => outlet_swap(pack, i).map_or(Made::NotApplicable, |v| {
                    let change = describe(&i.en.watch, &v.en.watch);
                    Made::Wrong(v, change)
                }),
                Kind::ZhOnlyNumber => zh_only_number(pack, i),
                Kind::FormalAdvice => {
                    let v = formal_advice(i, pick);
                    let change = describe(&i.en.watch, &v.en.watch);
                    Made::Wrong(v, change)
                }
                Kind::Control => Made::NotApplicable,
            };
            let made = match made {
                Made::Wrong(v, _) if v == *i => Made::WouldBeTrue,
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
                change: None,
            };
            let mut sendable = Vec::new();
            for (kind, made) in attempts(&r.pack, i, pick) {
                match made {
                    Made::Wrong(v, change) => {
                        let c = check::check(&r.pack, &v);
                        if c.is_empty() {
                            sendable.push(rows.len());
                        }
                        let mut seeded = row(kind, v, c);
                        seeded.change = Some(change);
                        rows.push((seeded, r.pack.clone()));
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
                Made::Wrong(v, _) => Some((k, v)),
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
        // The Chinese line says the same wrong thing.
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期上调至10亿美元，并宣布42亿美元回购。"
        );

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
            "Reuters reports Apple raised its revenue forecast to $4.2 billion alongside a $1.0 billion buyback, so investors may wish to add exposure."
        );
        assert_eq!(
            v.zh.watch,
            "据路透社报道，苹果将营收预期上调至42亿美元，并宣布10亿美元回购，投资者不妨适度布局。"
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
        let Made::Wrong(v, _) = &m[&Kind::NumberSwap] else {
            panic!("{:?}", m[&Kind::NumberSwap])
        };
        assert_eq!(
            v.en.watch,
            "The company reported revenue of $4.55 billion for Q1 of fiscal 2027."
        );
        let Made::Wrong(v, _) = &m[&Kind::ZhOnlyNumber] else {
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
        // Neither end of a range is replaced either ("$4.38 to $1.09" would
        // garble the range, as "38% to 40%" became "40% to 40%" in the
        // final run), so a line whose only figures are a range is skipped.
        assert_eq!(made_kinds(&p2, &i2)[&Kind::NumberSwap], Made::WouldBeTrue);

        let i = insight(
            "Reuters reports revenue of $4.2 billion, which could come in higher or lower at the report.",
            "据路透社报道，苹果营收42亿美元，财报数字可能高于或低于这一水平。",
        );
        assert_eq!(
            made_kinds(&pack(), &i)[&Kind::DirectionFlip],
            Made::WouldBeTrue
        );
    }

    /// The final run's lines that passed the checks (2026-11-13), as stored.
    fn final_run() -> Vec<Record> {
        include_str!("../testdata/final_run_lines.jsonl")
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The four-digit years written in `t`, in order.
    fn years(t: &str) -> Vec<String> {
        t.split(|c: char| !c.is_ascii_digit())
            .filter(|w| w.len() == 4 && (w.starts_with("19") || w.starts_with("20")))
            .map(str::to_string)
            .collect()
    }

    /// Final run: number swaps exchanged a year with a money amount and
    /// garbled the sentence ("guides to 2026 of $28.00 adjusted EPS",
    /// "revenue to approximately 2026", "forecast to 2026-$3.63 billion").
    /// Now a swap is like for like, in both languages, never a year; and a
    /// line without such a pair is skipped.
    #[test]
    fn number_swaps_on_the_final_run_are_like_for_like() {
        let records = final_run();
        assert!(records.len() >= 9);
        let mut made = 0;
        for r in &records {
            let i = r.insight.as_ref().unwrap();
            match number_swap(&r.pack, i) {
                Made::Wrong(v, change) => {
                    made += 1;
                    for garbled in [
                        "to 2026 of",
                        "approximately 2026",
                        "2026-$",
                        "7.2, down",
                        "7/24",
                    ] {
                        assert!(
                            !v.en.watch.contains(garbled),
                            "{} {}: {}",
                            r.post,
                            r.ticker,
                            v.en.watch
                        );
                    }
                    // Years stay where they were, in both languages.
                    assert_eq!(years(&v.en.watch), years(&i.en.watch), "{}", r.ticker);
                    assert_eq!(years(&v.zh.watch), years(&i.zh.watch), "{}", r.ticker);
                    // Both languages say the same wrong thing.
                    assert_ne!(v.en.watch, i.en.watch);
                    assert_ne!(v.zh.watch, i.zh.watch, "{} {}", r.ticker, v.zh.watch);
                    assert!(change.contains('↔') || change.contains('→'), "{change}");
                    let sides: Vec<&str> = change.split(['↔', '→']).map(str::trim).collect();
                    for side in sides {
                        assert!(side.contains(['$', '%']), "{} {change}", r.ticker);
                    }
                }
                Made::WouldBeTrue | Made::NotApplicable => {}
            }
        }
        assert!(made > 0);
        // The four the lead named: no year is ever swapped.
        let get = |post: &str, t: &str| {
            records
                .iter()
                .find(|r| r.post == post && r.ticker == t)
                .unwrap()
        };
        let cpay = get("recap_2026-11-05", "CPAY");
        let Made::Wrong(v, change) = number_swap(&cpay.pack, cpay.insight.as_ref().unwrap()) else {
            panic!()
        };
        assert_eq!(change, "$28.00 ↔ $7.54");
        // Every place a figure is written changes, so the line stays
        // consistent with itself.
        assert_eq!(
            v.en.watch,
            "TIKR reports Corpay now guides to $7.54 of 2026 adjusted EPS after posting $28.00 for the quarter, so whether later results keep pace with that $7.54 full-year figure is what would confirm the raise."
        );
        assert_eq!(
            v.zh.watch,
            "据TIKR报道，Corpay本季调整后每股收益为28.00美元，并将2026年调整后每股收益指引上调至7.54美元，后续业绩能否跟上这一全年数字是验证的关键。"
        );
        // The ones whose only swaps were a year, a range or percentage
        // points are skipped: AXON, ANET, FICO, CHTR premarket.
        for (post, t) in [
            ("recap_2026-11-06", "AXON"),
            ("recap_2026-11-12", "ANET"),
            ("premarket_2026-11-05", "FICO"),
            ("premarket_2026-11-06", "CHTR"),
        ] {
            let r = get(post, t);
            assert_eq!(
                number_swap(&r.pack, r.insight.as_ref().unwrap()),
                Made::WouldBeTrue,
                "{t}"
            );
        }
    }

    /// Each kind records what it changed, for seeded.jsonl and the report.
    #[test]
    fn every_seed_says_what_it_changed() {
        for (k, made) in attempts(&pack(), &two_outlets(), 0) {
            if let Made::Wrong(_, change) = made {
                assert!(
                    !change.is_empty() && change.contains(['→', '↔']),
                    "{k:?}: {change}"
                );
            }
        }
        let b = base();
        let m: BTreeMap<Kind, Made> = attempts(&pack(), &b, 0).into_iter().collect();
        let Made::Wrong(_, flip) = &m[&Kind::DirectionFlip] else {
            panic!()
        };
        assert_eq!(flip, "raised → cut");
        let Made::Wrong(_, swap) = &m[&Kind::NumberSwap] else {
            panic!()
        };
        assert_eq!(swap, "$4.2 billion ↔ $1.0 billion");
        let Made::Wrong(_, zh) = &m[&Kind::ZhOnlyNumber] else {
            panic!()
        };
        assert_eq!(zh, "42亿 → 10亿 (Chinese only)");
    }

    /// Round 4, L8: on the final run's lines the advice clause replaces the
    /// closing clause, so the seeds are no longer refused for length.
    #[test]
    fn advice_seeds_fit_the_length_limits() {
        let records = final_run();
        let mut fitted = 0;
        for (k, r) in records.iter().enumerate() {
            let i = r.insight.as_ref().unwrap();
            for pick in 0..ADVICE.len() {
                let v = formal_advice(i, k + pick);
                let f = check::check(&r.pack, &v);
                assert!(
                    !f.iter()
                        .any(|e| e.contains("words (at most") || e.contains("characters (at most")),
                    "{} {}: {f:?}",
                    r.ticker,
                    v.en.watch
                );
                if f.is_empty() {
                    fitted += 1;
                }
            }
        }
        assert!(fitted >= 40, "{fitted}");
    }
}
