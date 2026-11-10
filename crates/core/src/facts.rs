//! The single stored record both language drafts are written from, plus the
//! publish gate and the number allow-list.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::numbers::{self, Lang};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Field<T> {
    Value(T),
    Missing { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Quote {
    pub symbol: String,
    pub last: f64,
    pub prev_close: f64,
    /// Percent units: 0.45 means +0.45%.
    pub change_pct: f64,
    pub volume: Option<u64>,
}

/// Two decimals, as every `Quote` price and percent is stored: provider
/// prices such as 2805.876 would otherwise reach the facts and the prose.
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NewsReason {
    pub url: String,
    pub title: String,
    /// Verbatim source text.
    pub quote: String,
    pub summary_en: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum Direction {
    Up,
    Down,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ThemeMember {
    pub ticker: String,
    pub name: String,
    /// Percent units, signed.
    pub change_pct: f64,
}

/// An industry group moving together: several members beyond the rule's
/// threshold in one direction.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Theme {
    pub group: String,
    pub group_zh: String,
    pub direction: Direction,
    /// Qualifying members by |move| descending; at most the listed few.
    pub members: Vec<ThemeMember>,
    /// Qualifying members not listed in `members`.
    pub more: usize,
    /// Signed median move of the qualifying members, two decimals.
    pub median_pct: f64,
    pub reasons: Vec<NewsReason>,
}

/// Breadth of one group's strong movers, even when no theme formed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ThemeStat {
    pub group: String,
    pub present: usize,
    pub up: usize,
    pub down: usize,
    pub median_up: Option<f64>,
    pub median_down: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mover {
    pub quote: Quote,
    pub name: String,
    pub reasons: Vec<NewsReason>,
    /// What code can prove about a mover with no verified reason (`why`).
    /// Absent when it has a reason or nothing is known, and in facts stored
    /// before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FallbackKind {
    /// The earnings calendar marks the company's report as out, at a time
    /// that precedes this session's move.
    Results,
    /// The mover is in the group of one of the facts' themes and moved the
    /// same way. Co-movement, never a cause.
    Theme,
}

/// The Why of a mover without a verified news reason, rendered by code
/// (`why::fill`) from the calendar or the themes, never by the LLM.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Fallback {
    pub kind: FallbackKind,
    /// "Reported results after Monday's close", "Fell with the networking
    /// & optical group (median −4.63%); no company news found".
    pub text_en: String,
    pub text_zh: String,
    /// This site's earnings check for the report: `/{lang}/p/{slug}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_slug: Option<String>,
    /// The SEC filing of the report, when there is no earnings check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filing_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PremarketFacts {
    pub trading_date: NaiveDate,
    pub as_of_utc: DateTime<Utc>,
    pub futures: Vec<(String, Field<Quote>)>,
    pub gainers: Vec<Mover>,
    pub losers: Vec<Mover>,
    pub macro_items: Field<Vec<NewsReason>>,
    /// Today's macro events and after-close write-up earnings
    /// (`events::watch`). Facts stored before the events calendar hold
    /// `Missing`.
    pub watch_list: Field<Vec<crate::events::watch::WatchItem>>,
    /// Industry groups moving together, best first. Absent in facts stored
    /// before themes existed.
    #[serde(default)]
    pub themes: Vec<Theme>,
    /// Breadth of every group with two strong movers, for tuning the theme
    /// thresholds. Never sent to the LLM.
    #[serde(default)]
    pub theme_candidates: Vec<ThemeStat>,
    /// Short counters from the build ("quotes ok=480 http429=0 ..."), for
    /// logs and `last_error`. Never sent to the LLM.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

/// Names of required sections that are unavailable. Empty means publishable.
pub fn missing_required(f: &PremarketFacts) -> Vec<&'static str> {
    let mut out = Vec::new();
    if f.futures
        .iter()
        .all(|(_, q)| matches!(q, Field::Missing { .. }))
    {
        out.push("futures");
    }
    if !f
        .gainers
        .iter()
        .chain(&f.losers)
        .any(|m| !m.reasons.is_empty())
    {
        out.push("movers");
    }
    out
}

fn push_quote(out: &mut Vec<f64>, q: &Quote) {
    out.extend([q.last, q.prev_close, q.change_pct]);
    if let Some(v) = q.volume {
        out.push(v as f64);
    }
}

/// Numbers a source quote allows, in base units. Without an explicit sign
/// marker the quote carries no direction, so both signs are allowed.
pub(crate) fn quote_numbers(quote: &str) -> Vec<f64> {
    let mut out = Vec::new();
    // `Num.value` is already scaled to base units.
    for n in numbers::extract(quote, Lang::En) {
        out.push(n.value);
        if !n.signed {
            out.push(-n.value);
        }
    }
    out
}

fn push_reason(out: &mut Vec<f64>, r: &NewsReason) {
    out.extend(quote_numbers(&r.quote));
}

/// Each theme's member moves and median, computed in code so the draft may
/// cite them, and the numbers in its reasons' quotes.
pub(crate) fn push_themes(out: &mut Vec<f64>, themes: &[Theme]) {
    for t in themes {
        out.push(t.median_pct);
        out.extend(t.members.iter().map(|m| m.change_pct));
        for r in &t.reasons {
            push_reason(out, r);
        }
    }
}

/// Every number a draft may cite: quote fields, numbers inside source quotes,
/// a released report's figures on the watch list and the numbers in their
/// release sentences, and the trading year and its neighbours ("full-year
/// 2026 guidance").
pub fn numbers_in_facts(f: &PremarketFacts) -> Vec<f64> {
    let year = f64::from(f.trading_date.year());
    let mut out = vec![year - 1.0, year, year + 1.0];
    for (_, fq) in &f.futures {
        if let Field::Value(q) = fq {
            push_quote(&mut out, q);
        }
    }
    for m in f.gainers.iter().chain(&f.losers) {
        push_quote(&mut out, &m.quote);
        for r in &m.reasons {
            push_reason(&mut out, r);
        }
    }
    if let Field::Value(items) = &f.macro_items {
        for r in items {
            push_reason(&mut out, r);
        }
    }
    push_themes(&mut out, &f.themes);
    if let Field::Value(items) = &f.watch_list {
        for g in items.iter().flat_map(|i| &i.figures) {
            out.push(g.value);
            // The release's sentence ("after rising 0.6 percent in September").
            out.extend(quote_numbers(&g.quote));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(sym: &str) -> Quote {
        Quote {
            symbol: sym.into(),
            last: 5800.25,
            prev_close: 5790.0,
            change_pct: 0.45,
            volume: Some(1200),
        }
    }
    fn reason(text: &str) -> NewsReason {
        NewsReason {
            url: "https://x.test/a".into(),
            title: "t".into(),
            quote: text.into(),
            summary_en: "s".into(),
        }
    }
    fn mover(reasons: Vec<NewsReason>) -> Mover {
        Mover {
            quote: quote("ABC"),
            name: "ABC Inc".into(),
            reasons,
            fallback: None,
        }
    }
    fn missing() -> Field<Quote> {
        Field::Missing {
            reason: "feed down".into(),
        }
    }
    fn facts() -> PremarketFacts {
        PremarketFacts {
            trading_date: NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
            as_of_utc: "2026-10-29T12:00:00Z".parse().unwrap(),
            futures: vec![
                ("ES".into(), Field::Value(quote("ES"))),
                ("NQ".into(), missing()),
            ],
            gainers: vec![mover(vec![reason("shares rose")])],
            losers: vec![],
            macro_items: Field::Value(vec![reason("CPI was 3.1%")]),
            watch_list: Field::Missing {
                reason: "none".into(),
            },
            themes: vec![],
            theme_candidates: vec![],
            diagnostics: vec![],
        }
    }

    #[test]
    fn all_futures_missing_gates_futures() {
        let mut f = facts();
        f.futures = ["ES", "NQ", "YM", "RTY"]
            .iter()
            .map(|l| (l.to_string(), missing()))
            .collect();
        assert_eq!(missing_required(&f), vec!["futures"]);
    }

    #[test]
    fn movers_without_reasons_gate_movers() {
        let mut f = facts();
        f.gainers = vec![mover(vec![])];
        f.losers = vec![mover(vec![])];
        assert_eq!(missing_required(&f), vec!["movers"]);
    }

    #[test]
    fn publishable_when_one_future_and_one_reason() {
        assert!(missing_required(&facts()).is_empty());
    }

    #[test]
    fn json_round_trip() {
        let f = facts();
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<PremarketFacts>(&s).unwrap(), f);
    }

    #[test]
    fn diagnostics_round_trip_and_default_when_absent() {
        let mut f = facts();
        f.diagnostics = vec!["quotes ok=1".into()];
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<PremarketFacts>(&s).unwrap(), f);
        // Facts stored before this field existed still parse.
        let mut v: serde_json::Value = serde_json::to_value(&f).unwrap();
        v.as_object_mut().unwrap().remove("diagnostics");
        let old: PremarketFacts = serde_json::from_value(v).unwrap();
        assert!(old.diagnostics.is_empty());
    }

    #[test]
    fn mover_fallback_round_trips_and_is_absent_from_old_facts() {
        let mut f = facts();
        // No fallback: the stored JSON has no key, as before it existed.
        let s = serde_json::to_string(&f).unwrap();
        assert!(!s.contains("fallback"), "{s}");
        f.losers = vec![Mover {
            fallback: Some(Fallback {
                kind: FallbackKind::Results,
                text_en: "Reported results after Monday's close".into(),
                text_zh: "已于周一盘后公布财报".into(),
                post_slug: None,
                filing_url: Some("https://www.sec.gov/Archives/edgar/data/1/x-index.htm".into()),
            }),
            ..mover(vec![])
        }];
        let s = serde_json::to_string(&f).unwrap();
        assert!(
            s.contains(r#""kind":"results""#) && !s.contains("post_slug"),
            "{s}"
        );
        assert_eq!(serde_json::from_str::<PremarketFacts>(&s).unwrap(), f);
        // A mover stored before the field existed reads as having none.
        let old = r#"{"quote":{"symbol":"ABC","last":1.0,"prev_close":1.0,"change_pct":0.0,"volume":null},"name":"ABC Inc","reasons":[]}"#;
        assert_eq!(serde_json::from_str::<Mover>(old).unwrap().fallback, None);
    }

    #[test]
    fn quote_text_numbers_are_collected_in_base_units() {
        let mut f = facts();
        f.gainers = vec![mover(vec![reason("rose 12% to $1.2 billion")])];
        let n = numbers_in_facts(&f);
        assert!(n.contains(&12.0), "{n:?}");
        assert!(n.iter().any(|v| (v - 1.2e9).abs() < 1.0), "{n:?}");
        assert!(n.contains(&5800.25) && n.contains(&1200.0));
    }

    #[test]
    fn unsigned_quote_numbers_allow_both_directions() {
        let mut f = facts();
        f.gainers = vec![mover(vec![reason("shares fell 3% in premarket trading")])];
        let n = numbers_in_facts(&f);
        assert!(numbers::check("下跌3%", Lang::Zh, &n).is_ok());
        assert!(numbers::check("上涨3%", Lang::Zh, &n).is_ok());
    }

    #[test]
    fn trading_year_and_neighbours_are_allowed() {
        let n = numbers_in_facts(&facts());
        for y in [2025.0, 2026.0, 2027.0] {
            assert!(n.contains(&y), "{y} missing from {n:?}");
        }
        assert!(numbers::check("full-year 2026 guidance", Lang::En, &n).is_ok());
        assert!(numbers::check("a 2027 target", Lang::En, &n).is_ok());
        assert_eq!(
            numbers::check("Russell futures at 2045", Lang::En, &n),
            Err(vec!["2045".to_string()])
        );
    }

    #[test]
    fn macro_quote_numbers_collected() {
        assert!(numbers_in_facts(&facts()).contains(&3.1));
    }

    fn theme() -> Theme {
        Theme {
            group: "Memory & storage".into(),
            group_zh: "存储".into(),
            direction: Direction::Down,
            members: vec![
                ThemeMember {
                    ticker: "MU".into(),
                    name: "Micron Technology".into(),
                    change_pct: -6.12,
                },
                ThemeMember {
                    ticker: "WDC".into(),
                    name: "Western Digital".into(),
                    change_pct: -4.37,
                },
            ],
            more: 0,
            median_pct: -5.25,
            reasons: vec![reason("memory prices fell 7% in October")],
        }
    }

    #[test]
    fn numbers_in_facts_include_theme_moves_and_medians() {
        let mut f = facts();
        f.themes = vec![theme()];
        let n = numbers_in_facts(&f);
        for v in [-6.12, -4.37, -5.25, 7.0] {
            assert!(n.contains(&v), "{v} missing from {n:?}");
        }
        assert!(numbers::check("Micron fell 6.12%", Lang::En, &n).is_ok());
        assert!(numbers::check("a median drop of 5.25%", Lang::En, &n).is_ok());
    }

    #[test]
    fn facts_without_theme_fields_deserialize() {
        let mut f = facts();
        f.themes = vec![theme()];
        f.theme_candidates = vec![ThemeStat {
            group: "Memory & storage".into(),
            present: 4,
            up: 0,
            down: 4,
            median_up: None,
            median_down: Some(-5.25),
        }];
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<PremarketFacts>(&s).unwrap(), f);
        // Facts stored before themes existed still parse, with none.
        let mut v: serde_json::Value = serde_json::to_value(&f).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("themes");
        o.remove("theme_candidates");
        let old: PremarketFacts = serde_json::from_value(v).unwrap();
        assert!(old.themes.is_empty() && old.theme_candidates.is_empty());
    }
}
