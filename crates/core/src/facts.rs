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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mover {
    pub quote: Quote,
    pub name: String,
    pub reasons: Vec<NewsReason>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PremarketFacts {
    pub trading_date: NaiveDate,
    pub as_of_utc: DateTime<Utc>,
    pub futures: Vec<(String, Field<Quote>)>,
    pub gainers: Vec<Mover>,
    pub losers: Vec<Mover>,
    pub macro_items: Field<Vec<NewsReason>>,
    pub watch_list: Field<Vec<String>>,
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

/// Every number a draft may cite: quote fields, numbers inside source quotes,
/// and the trading year and its neighbours ("full-year 2026 guidance").
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
}
