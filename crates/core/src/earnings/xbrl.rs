//! GAAP quarterly history from XBRL, one `companyconcept` document per
//! concept (never the multi-megabyte `companyfacts`, which would not fit the
//! Free plan's CPU budget). Only discrete quarters a company actually
//! reported in a 10-Q or 10-K are used; a quarter the company never tagged
//! (often fiscal Q4, which appears only inside the annual total) stays a gap
//! rather than being derived.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::earnings::facts::{QuarterPoint, Trend};
use crate::edgar::{Concept, FactEntry, Filings};
use crate::providers::ProviderError;

/// Quarters kept per series.
pub const QUARTERS: usize = 8;
/// A release is filed within ~10 weeks of its quarter's end.
const RELEASE_QUARTER_MAX_DAYS: i64 = 75;
/// Revenue tags in order of preference; companies switch tags over time.
pub const REVENUE_TAGS: [&str; 3] = [
    "Revenues",
    "RevenueFromContractWithCustomerExcludingAssessedTax",
    "RevenuesNetOfInterestExpense",
];
pub const EPS_TAG: &str = "EarningsPerShareDiluted";
pub const NET_INCOME_TAG: &str = "NetIncomeLoss";
/// A revenue tag whose latest quarter ended this close before the filing is
/// the company's current tag: later fallbacks are not fetched.
const CURRENT_TAG_DAYS: i64 = 200;

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn is_periodic_report(form: &str) -> bool {
    matches!(form, "10-Q" | "10-K" | "10-Q/A" | "10-K/A")
}

/// Discrete quarters (80-100 day durations) from periodic reports, one per
/// period end (the latest `filed` wins, so restatements replace originals),
/// oldest first.
fn quarters(entries: &[FactEntry]) -> Vec<QuarterPoint> {
    let mut by_end: BTreeMap<NaiveDate, QuarterPoint> = BTreeMap::new();
    for e in entries.iter().filter(|e| is_periodic_report(&e.form)) {
        let (Some(start), Some(end), Some(filed)) = (
            e.start.as_deref().and_then(date),
            date(&e.end),
            date(&e.filed),
        ) else {
            continue;
        };
        let days = (end - start).num_days();
        if !(80..=100).contains(&days) || !e.val.is_finite() {
            continue;
        }
        let p = QuarterPoint {
            period_end: end,
            value: e.val,
            form: e.form.clone(),
            filed,
        };
        match by_end.get(&end) {
            Some(old) if old.filed >= filed => {}
            _ => {
                by_end.insert(end, p);
            }
        }
    }
    by_end.into_values().collect()
}

fn last_n(all: Vec<QuarterPoint>) -> Vec<QuarterPoint> {
    let skip = all.len().saturating_sub(QUARTERS);
    all.into_iter().skip(skip).collect()
}

/// Revenue (merged across `revenue` tags by period end, the first-listed
/// tag winning a quarter both report), diluted EPS and net income series,
/// the last `QUARTERS` each.
pub fn trend(revenue: &[Concept], eps: Option<&Concept>, net_income: Option<&Concept>) -> Trend {
    let mut merged: BTreeMap<NaiveDate, QuarterPoint> = BTreeMap::new();
    for c in revenue {
        for p in quarters(&c.units.usd) {
            merged.entry(p.period_end).or_insert(p);
        }
    }
    Trend {
        revenue: last_n(merged.into_values().collect()),
        eps_diluted: last_n(
            eps.map(|c| quarters(&c.units.usd_per_share))
                .unwrap_or_default(),
        ),
        net_income: last_n(
            net_income
                .map(|c| quarters(&c.units.usd))
                .unwrap_or_default(),
        ),
    }
}

/// Fetches the concepts the trend needs: revenue tags in order until one is
/// current for `filing_date` (at most 3), then EPS and net income: at most 5
/// requests. A concept the company never tagged is skipped; any other error
/// fails the trend.
pub async fn fetch_trend(
    filings: &dyn Filings,
    cik: u64,
    filing_date: NaiveDate,
) -> Result<Trend, ProviderError> {
    let mut revenue = Vec::new();
    for tag in REVENUE_TAGS {
        let Some(c) = filings.company_concept(cik, tag).await? else {
            continue;
        };
        let current = quarters(&c.units.usd)
            .last()
            .is_some_and(|p| (filing_date - p.period_end).num_days() <= CURRENT_TAG_DAYS);
        revenue.push(c);
        if current {
            break;
        }
    }
    let eps = filings.company_concept(cik, EPS_TAG).await?;
    let net_income = filings.company_concept(cik, NET_INCOME_TAG).await?;
    Ok(trend(&revenue, eps.as_ref(), net_income.as_ref()))
}

/// The series' latest XBRL quarter is the quarter this press release
/// reports: its 10-Q/10-K was filed no earlier than the 8-K, and the quarter
/// ended recently enough to be the release quarter (a late amendment of an
/// older quarter does not count).
pub fn release_already_in_trend(points: &[QuarterPoint], filing_date: NaiveDate) -> bool {
    points.last().is_some_and(|p| {
        p.filed >= filing_date
            && (filing_date - p.period_end).num_days() <= RELEASE_QUARTER_MAX_DAYS
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::edgar::Units;
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub(crate) const REVENUE: &str = include_str!(
        "../../tests/fixtures/edgar_concept_AAPL_RevenueFromContractWithCustomerExcludingAssessedTax.json"
    );
    pub(crate) const EPS: &str =
        include_str!("../../tests/fixtures/edgar_concept_AAPL_EarningsPerShareDiluted.json");
    pub(crate) const NET_INCOME: &str =
        include_str!("../../tests/fixtures/edgar_concept_AAPL_NetIncomeLoss.json");

    /// The AAPL concept fixtures by tag.
    pub(crate) fn aapl_concept(tag: &str) -> Option<Concept> {
        let body = match tag {
            "RevenueFromContractWithCustomerExcludingAssessedTax" => REVENUE,
            EPS_TAG => EPS,
            NET_INCOME_TAG => NET_INCOME,
            _ => return None,
        };
        Some(serde_json::from_str(body).unwrap())
    }

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    fn fixture() -> Trend {
        let rev = aapl_concept(REVENUE_TAGS[1]).unwrap();
        trend(
            &[rev],
            aapl_concept(EPS_TAG).as_ref(),
            aapl_concept(NET_INCOME_TAG).as_ref(),
        )
    }
    fn entry(start: &str, end: &str, val: f64, form: &str, filed: &str) -> FactEntry {
        FactEntry {
            start: Some(start.into()),
            end: end.into(),
            val,
            form: form.into(),
            filed: filed.into(),
        }
    }
    fn concept(e: Vec<FactEntry>) -> Concept {
        Concept {
            units: Units {
                usd: e,
                usd_per_share: vec![],
            },
        }
    }

    #[test]
    fn fixture_trend() {
        let t = fixture();
        let ends: Vec<_> = t.revenue.iter().map(|p| p.period_end.to_string()).collect();
        // Fiscal Q4s (annual-only), the 9-month YTD, the annual total and the
        // 8-K fact are not quarters; no Q4 is derived.
        assert_eq!(
            ends,
            [
                "2024-12-28",
                "2025-03-29",
                "2025-06-28",
                "2025-12-27",
                "2026-03-28",
                "2026-06-27"
            ]
        );
        // The 2025-06-28 quarter was re-reported later with a revised value.
        assert_eq!(t.revenue[2].value, 94_100_000_000.0);
        assert_eq!(t.revenue[2].filed, d("2026-07-31"));
        assert_eq!(t.eps_diluted.len(), 6);
        assert_eq!(t.eps_diluted.last().unwrap().value, 1.71);
        assert_eq!(t.net_income.len(), 6);
    }

    #[test]
    fn keeps_last_eight() {
        let mut e = Vec::new();
        for y in 2020..2024 {
            for (s, en) in [
                ("01-01", "03-31"),
                ("04-01", "06-30"),
                ("07-01", "09-30"),
                ("10-01", "12-31"),
            ] {
                e.push(entry(
                    &format!("{y}-{s}"),
                    &format!("{y}-{en}"),
                    1.0,
                    "10-Q",
                    &format!("{}-01-15", y + 1),
                ));
            }
        }
        let t = trend(&[concept(e)], None, None);
        assert_eq!(t.revenue.len(), QUARTERS);
        assert_eq!(t.revenue[0].period_end, d("2022-03-31"));
    }

    #[test]
    fn revenue_tags_are_merged() {
        let old = vec![entry("2019-01-01", "2019-03-31", 5.0, "10-Q", "2019-05-01")];
        let new = vec![entry("2026-04-01", "2026-06-30", 9.0, "10-Q", "2026-08-01")];
        let r = trend(&[concept(old), concept(new)], None, None).revenue;
        assert_eq!(r.iter().map(|p| p.value).collect::<Vec<_>>(), [5.0, 9.0]);
    }

    #[test]
    fn same_day_periodic_report() {
        let t = fixture();
        // Q3 ended 2026-06-27; its 10-Q was filed 2026-07-31.
        assert!(!release_already_in_trend(&t.revenue, d("2026-10-30")));
        assert!(release_already_in_trend(&t.revenue, d("2026-07-31")));
        assert!(!release_already_in_trend(&[], d("2026-07-31")));
        // A 10-Q/A for an old quarter filed on the 8-K date is not the release quarter.
        let mut late = t.revenue.clone();
        late.last_mut().unwrap().filed = d("2026-12-01");
        assert!(!release_already_in_trend(&late, d("2026-12-01")));
    }

    /// Serves concepts by tag; records each requested tag.
    pub(crate) struct ConceptFilings {
        pub concepts: HashMap<&'static str, Result<Concept, ProviderError>>,
        pub asked: RefCell<Vec<String>>,
    }
    #[async_trait(?Send)]
    impl Filings for ConceptFilings {
        async fn current_filings(
            &self,
            _: usize,
        ) -> Result<Vec<crate::edgar::FeedEntry>, ProviderError> {
            unreachable!()
        }
        async fn filing_docs(
            &self,
            _: u64,
            _: &str,
        ) -> Result<Vec<crate::edgar::FilingDoc>, ProviderError> {
            unreachable!()
        }
        async fn document_text(&self, _: &str) -> Result<String, ProviderError> {
            unreachable!()
        }
        async fn company_concept(
            &self,
            _: u64,
            tag: &str,
        ) -> Result<Option<Concept>, ProviderError> {
            self.asked.borrow_mut().push(tag.into());
            self.concepts.get(tag).cloned().transpose()
        }
    }

    #[tokio::test]
    async fn fetch_stops_at_the_current_revenue_tag() {
        // AAPL: no `Revenues`, current `RevenueFromContract...`: the third
        // tag is never asked for; 4 requests.
        let f = ConceptFilings {
            concepts: [REVENUE_TAGS[1], EPS_TAG, NET_INCOME_TAG]
                .into_iter()
                .map(|t| (t, Ok(aapl_concept(t).unwrap())))
                .collect(),
            asked: RefCell::new(vec![]),
        };
        let t = fetch_trend(&f, 320193, d("2026-10-30")).await.unwrap();
        assert_eq!(t, fixture());
        assert_eq!(
            *f.asked.borrow(),
            [REVENUE_TAGS[0], REVENUE_TAGS[1], EPS_TAG, NET_INCOME_TAG]
        );
    }

    #[tokio::test]
    async fn fetch_falls_back_through_stale_tags_with_at_most_five_requests() {
        // A bank: an old `Revenues` series, then `RevenuesNetOfInterestExpense`.
        let old = concept(vec![entry(
            "2019-01-01",
            "2019-03-31",
            5.0,
            "10-Q",
            "2019-05-01",
        )]);
        let new = concept(vec![entry(
            "2026-04-01",
            "2026-06-30",
            9.0,
            "10-Q",
            "2026-08-01",
        )]);
        let f = ConceptFilings {
            concepts: [(REVENUE_TAGS[0], Ok(old)), (REVENUE_TAGS[2], Ok(new))]
                .into_iter()
                .collect(),
            asked: RefCell::new(vec![]),
        };
        let t = fetch_trend(&f, 1, d("2026-10-15")).await.unwrap();
        assert_eq!(t.revenue.len(), 2);
        assert!(t.eps_diluted.is_empty() && t.net_income.is_empty());
        assert_eq!(f.asked.borrow().len(), 5);
    }

    #[tokio::test]
    async fn fetch_error_fails_the_trend() {
        let f = ConceptFilings {
            concepts: [(REVENUE_TAGS[0], Err(ProviderError::TooLarge(1)))]
                .into_iter()
                .collect(),
            asked: RefCell::new(vec![]),
        };
        assert_eq!(
            fetch_trend(&f, 1, d("2026-10-15")).await,
            Err(ProviderError::TooLarge(1))
        );
    }
}
