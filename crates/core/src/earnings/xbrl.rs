//! GAAP quarterly history from XBRL `companyfacts`. Only discrete quarters
//! a company actually reported in a 10-Q or 10-K are used; a quarter the
//! company never tagged (often fiscal Q4, which appears only inside the
//! annual total) stays a gap rather than being derived.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::earnings::facts::{QuarterPoint, Trend};
use crate::edgar::{CompanyFacts, Concept, FactEntry};

/// Quarters kept per series.
pub const QUARTERS: usize = 8;
/// A release is filed within ~10 weeks of its quarter's end.
const RELEASE_QUARTER_MAX_DAYS: i64 = 75;

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

fn all_quarters(c: &Option<Concept>) -> Vec<QuarterPoint> {
    c.as_ref()
        .map(|c| quarters(&c.units.usd))
        .unwrap_or_default()
}

/// Revenue, diluted EPS and net income series, the last `QUARTERS` each.
pub fn trend(cf: &CompanyFacts) -> Trend {
    let g = &cf.facts.us_gaap;
    // Companies switch revenue tags over time: merge the candidates by
    // period end, the first-listed tag winning a quarter both report.
    let mut merged: BTreeMap<NaiveDate, QuarterPoint> = BTreeMap::new();
    for c in [
        &g.revenues,
        &g.revenue_from_contracts,
        &g.revenues_net_of_interest,
        &g.sales_revenue_net,
    ] {
        for p in all_quarters(c) {
            merged.entry(p.period_end).or_insert(p);
        }
    }
    let revenue = last_n(merged.into_values().collect());
    Trend {
        revenue,
        eps_diluted: last_n(
            g.eps_diluted
                .as_ref()
                .map(|c| quarters(&c.units.usd_per_share))
                .unwrap_or_default(),
        ),
        net_income: last_n(all_quarters(&g.net_income)),
    }
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
mod tests {
    use super::*;
    use crate::edgar::tests::COMPANYFACTS;
    use crate::edgar::{FactsRoot, Units, UsGaap};

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    fn fixture() -> CompanyFacts {
        serde_json::from_str(COMPANYFACTS).unwrap()
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
    fn concept(e: Vec<FactEntry>) -> Option<Concept> {
        Some(Concept {
            units: Units {
                usd: e,
                usd_per_share: vec![],
            },
        })
    }

    #[test]
    fn fixture_trend() {
        let t = trend(&fixture());
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
        let cf = CompanyFacts {
            facts: FactsRoot {
                us_gaap: UsGaap {
                    revenues: concept(e),
                    ..Default::default()
                },
            },
        };
        let t = trend(&cf);
        assert_eq!(t.revenue.len(), QUARTERS);
        assert_eq!(t.revenue[0].period_end, d("2022-03-31"));
    }

    #[test]
    fn revenue_tags_are_merged() {
        let old = vec![entry("2019-01-01", "2019-03-31", 5.0, "10-Q", "2019-05-01")];
        let new = vec![entry("2026-04-01", "2026-06-30", 9.0, "10-Q", "2026-08-01")];
        let cf = CompanyFacts {
            facts: FactsRoot {
                us_gaap: UsGaap {
                    revenues: concept(old),
                    revenue_from_contracts: concept(new),
                    ..Default::default()
                },
            },
        };
        let r = trend(&cf).revenue;
        assert_eq!(r.iter().map(|p| p.value).collect::<Vec<_>>(), [5.0, 9.0]);
    }

    #[test]
    fn same_day_periodic_report() {
        let t = trend(&fixture());
        // Q3 ended 2026-06-27; its 10-Q was filed 2026-07-31.
        assert!(!release_already_in_trend(&t.revenue, d("2026-10-30")));
        assert!(release_already_in_trend(&t.revenue, d("2026-07-31")));
        assert!(!release_already_in_trend(&[], d("2026-07-31")));
        // A 10-Q/A for an old quarter filed on the 8-K date is not the release quarter.
        let mut late = t.revenue.clone();
        late.last_mut().unwrap().filed = d("2026-12-01");
        assert!(!release_already_in_trend(&late, d("2026-12-01")));
    }
}
