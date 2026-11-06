//! Parsers for the official schedule pages the daily check re-reads: the
//! Fed's FOMC calendar and the BLS release schedules. They mirror the ones
//! in `scripts/build-events.mjs` (which also reads the Fed month pages for
//! minutes); a test runs both against the same fixtures' dates.

use std::sync::OnceLock;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use regex::Regex;

use super::{Planned, TIER_MAJOR};

pub const FOMC_URL: &str = "https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm";
pub const BLS_JOBS_URL: &str = "https://www.bls.gov/schedule/news_release/empsit.htm";
pub const BLS_CPI_URL: &str = "https://www.bls.gov/schedule/news_release/cpi.htm";

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

/// 1-12 for an English month name or abbreviation ("Sept", "Apr").
fn month(name: &str) -> Option<u32> {
    const M: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let key = name.get(..3)?.to_ascii_lowercase();
    M.iter().position(|m| *m == key).map(|i| i as u32 + 1)
}

/// The UTC instant of an ET wall-clock time (`None` in a DST gap).
pub fn et_instant(date: NaiveDate, h: u32, m: u32) -> Option<DateTime<Utc>> {
    let t = NaiveTime::from_hms_opt(h, m, 0)?;
    New_York
        .from_local_datetime(&date.and_time(t))
        .single()
        .map(|t| t.with_timezone(&Utc))
}

/// What the FOMC calendar lists: the meetings and the years it has panels for.
#[derive(Debug, Clone, PartialEq)]
pub struct FomcPage {
    pub meetings: Vec<Planned>,
    pub years: Vec<i32>,
}

/// The FOMC calendar: one panel per year, one row per meeting ("27-28",
/// "8-9*" with projections, "Apr/May 30-1" across months). The decision is
/// on the second day at 2:00 PM ET. Rows such as "22 (notation vote)" are
/// not meetings. `Err` when no meeting parses (a changed page).
pub fn parse_fomc(html: &str) -> Result<FomcPage, String> {
    static PANEL: OnceLock<Regex> = OnceLock::new();
    static ROW: OnceLock<Regex> = OnceLock::new();
    let panel = PANEL.get_or_init(|| re(r"<h4><a id=\x22\d+\x22>(\d{4}) FOMC Meetings"));
    let row = ROW.get_or_init(|| {
        re(r#"fomc-meeting__month[^>]*><strong>([^<]+)</strong></div>\s*<div class="fomc-meeting__date[^>]*>([^<]+)</div>"#)
    });
    let days_re = re(r"^(\d{1,2})-(\d{1,2})(\*?)$");
    let starts: Vec<(usize, i32)> = panel
        .captures_iter(html)
        .filter_map(|c| Some((c.get(0)?.start(), c[1].parse().ok()?)))
        .collect();
    let mut out = FomcPage {
        meetings: Vec::new(),
        years: starts.iter().map(|(_, y)| *y).collect(),
    };
    for (i, (start, year)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(html.len(), |(s, _)| *s);
        for c in row.captures_iter(&html[*start..end]) {
            let Some(days) = days_re.captures(c[2].trim()) else {
                continue;
            };
            let months: Vec<&str> = c[1].trim().split('/').collect();
            let (Some(first), Some(last)) = (
                months.first().and_then(|m| month(m)),
                months.last().and_then(|m| month(m)),
            ) else {
                continue;
            };
            let y = if last < first { year + 1 } else { *year };
            let Some(date) = days[2]
                .parse()
                .ok()
                .and_then(|d| NaiveDate::from_ymd_opt(y, last, d))
            else {
                continue;
            };
            let period = date.format("%Y-%m").to_string();
            out.meetings.push(Planned {
                id: format!("fed:fomc:{period}"),
                series: "fomc".into(),
                ref_period: Some(period),
                variant: (&days[3] == "*").then(|| "sep".to_string()),
                starts_at: et_instant(date, 14, 0),
                et_date: date,
                tier: TIER_MAJOR,
                proof_url: FOMC_URL.into(),
            });
        }
    }
    if out.meetings.is_empty() {
        return Err("fomc: no meetings parsed".into());
    }
    Ok(out)
}

/// A BLS release schedule ("October 2026 | Nov. 06, 2026 | 08:30 AM"),
/// `series` `jobs` or `cpi`. Past releases drop off the page. `Err` when no
/// row parses.
pub fn parse_bls(html: &str, series: &str, url: &str) -> Result<Vec<Planned>, String> {
    static ROW: OnceLock<Regex> = OnceLock::new();
    let row = ROW.get_or_init(|| {
        re(r"<tr[^>]*>\s*<td>([A-Za-z]+) (\d{4})</td>\s*<td>([A-Za-z]+)\.? (\d{1,2}), (\d{4})</td>\s*<td>(\d{1,2}):(\d{2}) ([AP])M</td>")
    });
    let mut out = Vec::new();
    for c in row.captures_iter(html) {
        let parsed = (|| {
            let ref_month = month(&c[1])?;
            let ref_year: i32 = c[2].parse().ok()?;
            let date =
                NaiveDate::from_ymd_opt(c[5].parse().ok()?, month(&c[3])?, c[4].parse().ok()?)?;
            let h: u32 = c[6].parse().ok()?;
            let h = h % 12 + if &c[8] == "P" { 12 } else { 0 };
            let period = format!("{ref_year}-{ref_month:02}");
            Some(Planned {
                id: format!("bls:{series}:{period}"),
                series: series.into(),
                ref_period: Some(period),
                variant: None,
                starts_at: Some(et_instant(date, h, c[7].parse().ok()?)?),
                et_date: date,
                tier: TIER_MAJOR,
                proof_url: url.into(),
            })
        })();
        out.extend(parsed);
    }
    if out.is_empty() {
        return Err(format!("{series}: no releases parsed"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::snapshot;

    const FOMC: &str = include_str!("../../tests/fixtures/fed_fomccalendars.htm");
    const JOBS: &str = include_str!("../../tests/fixtures/bls_empsit_schedule.htm");
    const CPI: &str = include_str!("../../tests/fixtures/bls_cpi_schedule.htm");

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn fomc_page_gives_decision_days_at_2pm_with_the_projections_flag() {
        let p = parse_fomc(FOMC).unwrap();
        assert_eq!(p.years, [2026, 2027]);
        assert_eq!(p.meetings.len(), 16);
        let dec = p
            .meetings
            .iter()
            .find(|m| m.id == "fed:fomc:2026-12")
            .unwrap();
        assert_eq!(dec.et_date, d("2026-12-09"));
        assert_eq!(dec.starts_at, Some(utc("2026-12-09T19:00:00Z")));
        assert_eq!(dec.variant.as_deref(), Some("sep"));
        let jan = p
            .meetings
            .iter()
            .find(|m| m.id == "fed:fomc:2027-01")
            .unwrap();
        assert_eq!(
            (jan.et_date, jan.variant.as_deref()),
            (d("2027-01-27"), None)
        );
        // EDT: 2:00 PM is 18:00Z.
        let oct = p
            .meetings
            .iter()
            .find(|m| m.id == "fed:fomc:2026-10")
            .unwrap();
        assert_eq!(oct.starts_at, Some(utc("2026-10-28T18:00:00Z")));
    }

    #[test]
    fn fomc_rows_across_months_and_notation_votes() {
        let html = r#"<h4><a id="1">2024 FOMC Meetings</a></h4>
            <div class="fomc-meeting__month col"><strong>Apr/May</strong></div>
            <div class="fomc-meeting__date col">30-1</div>
            <div class="fomc-meeting__month col"><strong>August</strong></div>
            <div class="fomc-meeting__date col">22 (notation vote)</div>"#;
        let p = parse_fomc(html).unwrap();
        assert_eq!(p.meetings.len(), 1);
        assert_eq!(p.meetings[0].et_date, d("2024-05-01"));
        assert_eq!(p.meetings[0].id, "fed:fomc:2024-05");
        assert!(parse_fomc("<html>moved</html>").is_err());
    }

    #[test]
    fn bls_schedules_give_reference_month_date_and_time() {
        let jobs = parse_bls(JOBS, "jobs", BLS_JOBS_URL).unwrap();
        assert_eq!(jobs[0].id, "bls:jobs:2026-10");
        assert_eq!(jobs[0].et_date, d("2026-11-06"));
        assert_eq!(jobs[0].starts_at, Some(utc("2026-11-06T13:30:00Z")));
        let jan = jobs.iter().find(|j| j.id == "bls:jobs:2026-12").unwrap();
        assert_eq!(jan.et_date, d("2027-01-08"));
        let cpi = parse_bls(CPI, "cpi", BLS_CPI_URL).unwrap();
        assert_eq!(cpi[0].id, "bls:cpi:2026-10");
        assert_eq!(cpi[0].et_date, d("2026-11-10"));
        assert!(cpi.iter().all(|c| c.proof_url == BLS_CPI_URL));
        let sept = "<tr><td>August 2026</td><td>Sept. 04, 2026</td><td>08:30 AM</td></tr>";
        assert_eq!(
            parse_bls(sept, "jobs", BLS_JOBS_URL).unwrap()[0].et_date,
            d("2026-09-04")
        );
        assert!(parse_bls("<table></table>", "cpi", BLS_CPI_URL).is_err());
    }

    /// The script's parser and this one agree: every fixture event inside the
    /// snapshot's window is in the snapshot on the same day and time.
    #[test]
    fn rust_and_script_parsers_agree_on_the_fixtures() {
        let (from, to) = crate::events::snapshot_covers().unwrap();
        let mut all = parse_fomc(FOMC).unwrap().meetings;
        all.extend(parse_bls(JOBS, "jobs", BLS_JOBS_URL).unwrap());
        all.extend(parse_bls(CPI, "cpi", BLS_CPI_URL).unwrap());
        let mut checked = 0;
        for p in all.iter().filter(|p| p.et_date >= from && p.et_date <= to) {
            let s = snapshot().iter().find(|s| s.id == p.id);
            let s = s.unwrap_or_else(|| panic!("{} not in events.tsv", p.id));
            assert_eq!(
                (s.et_date, s.starts_at, &s.variant),
                (p.et_date, p.starts_at, &p.variant),
                "{}",
                p.id
            );
            checked += 1;
        }
        assert!(checked >= 20, "{checked}");
    }
}
