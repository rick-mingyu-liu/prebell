//! A check's fiscal period, as the site's search titles write it
//! (`apps/web/src/lib/searchtitle.ts`, Rick's rule of 2026-11-19): "Q3 2026"
//! only when that calendar quarter ended within `CALENDAR_WINDOW_DAYS` before
//! the check's date, else "Q3 fiscal 2027". A Rust port of `calendarAligned`
//! and `periodFromSlug` (`earningsfacts.ts`), with their tests' cases.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::numbers::Lang;

/// How long after a calendar quarter ends its results can still be "Q3
/// 2026" rather than a fiscal quarter (`searchtitle.ts`).
pub const CALENDAR_WINDOW_DAYS: i64 = 120;

/// The period a check's slug was named from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Period {
    /// 1-4, or `None` for a full fiscal year.
    pub quarter: Option<u32>,
    pub fiscal_year: i32,
    /// The quarter read as a calendar quarter ended within the window
    /// before the check's date (`calendar_aligned`).
    pub calendar: bool,
}

/// `periodFromSlug`: `{ticker}-q4-fy2026-earnings` or `-fy2026-earnings`,
/// either with a `-{6 digits}` accession suffix; `None` for a slug named by
/// filing date (`aapl-2026-10-30-earnings`).
pub fn period_from_slug(slug: &str) -> Option<(Option<u32>, i32)> {
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let mut rest = slug;
    if let Some((head, tail)) = rest.rsplit_once('-') {
        if digits(tail, 6) {
            rest = head;
        }
    }
    let rest = rest.strip_suffix("-earnings")?;
    let (head, fy) = rest.rsplit_once("-fy")?;
    if !digits(fy, 4) {
        return None;
    }
    let year: i32 = fy.parse().ok()?;
    let quarter = head.rsplit_once("-q").and_then(|(_, q)| match q {
        "1" | "2" | "3" | "4" => q.parse().ok(),
        _ => None,
    });
    Some((quarter, year))
}

/// `calendarAligned`: "Q{quarter} {year}" read as a calendar quarter ended
/// on or before `trading_date`, at most `CALENDAR_WINDOW_DAYS` days before.
pub fn calendar_aligned(quarter: u32, year: i32, trading_date: NaiveDate) -> bool {
    // Day 0 of the next month: the quarter's last day.
    let next = if quarter == 4 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, quarter * 3 + 1, 1)
    };
    let Some(end) = next.and_then(|d| d.pred_opt()) else {
        return false;
    };
    let days = (trading_date - end).num_days();
    (0..=CALENDAR_WINDOW_DAYS).contains(&days)
}

/// The period of the check `slug` reported on `trading_date`.
pub fn period(slug: &str, trading_date: NaiveDate) -> Option<Period> {
    let (quarter, fiscal_year) = period_from_slug(slug)?;
    Some(Period {
        quarter,
        fiscal_year,
        calendar: quarter.is_some_and(|q| calendar_aligned(q, fiscal_year, trading_date)),
    })
}

const ZH_QUARTERS: [&str; 4] = ["一", "二", "三", "四"];

/// "Q3 2026", "Q3 fiscal 2027", "fiscal 2026"; "2026年第三季度",
/// "2027财年第三季度", "2026财年全年" (as `earningsSearchTitle` words them).
pub fn label(p: &Period, lang: Lang) -> String {
    let y = p.fiscal_year;
    match (lang, p.quarter) {
        (Lang::En, Some(q)) if p.calendar => format!("Q{q} {y}"),
        (Lang::En, Some(q)) => format!("Q{q} fiscal {y}"),
        (Lang::En, None) => format!("fiscal {y}"),
        (Lang::Zh, Some(q)) => {
            let unit = if p.calendar { "年" } else { "财年" };
            let i = usize::try_from(q.clamp(1, 4) - 1).unwrap_or(0);
            format!("{y}{unit}第{}季度", ZH_QUARTERS[i])
        }
        (Lang::Zh, None) => format!("{y}财年全年"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// `searchtitle.test.ts` and `earningsfacts.test.ts`, case for case.
    #[test]
    fn the_site_rules_case_for_case() {
        assert_eq!(
            period_from_slug("aapl-q4-fy2026-earnings"),
            Some((Some(4), 2026))
        );
        assert_eq!(
            period_from_slug("brk-b-fy2026-earnings"),
            Some((None, 2026))
        );
        assert_eq!(
            period_from_slug("aapl-q4-fy2026-earnings-000070"),
            Some((Some(4), 2026))
        );
        assert_eq!(period_from_slug("aapl-2026-10-30-earnings"), None);
        assert!(calendar_aligned(4, 2025, d("2026-02-05")));
        assert!(calendar_aligned(3, 2026, d("2026-09-30")));
        assert!(!calendar_aligned(4, 2026, d("2026-11-17")));
        assert!(!calendar_aligned(3, 2026, d("2027-03-01")));
        assert!(!calendar_aligned(3, 2026, d("2026-09-29")));
        assert!(calendar_aligned(3, 2026, d("2027-01-28")));
        assert!(!calendar_aligned(3, 2026, d("2027-01-29")));
        let l = |slug: &str, date: &str, lang| period(slug, d(date)).map(|p| label(&p, lang));
        // ConocoPhillips: a calendar quarter; Nvidia and Disney: fiscal.
        assert_eq!(
            l("cop-q3-fy2026-earnings", "2026-11-06", Lang::En).unwrap(),
            "Q3 2026"
        );
        assert_eq!(
            l("nvda-q3-fy2027-earnings", "2026-11-17", Lang::En).unwrap(),
            "Q3 fiscal 2027"
        );
        assert_eq!(
            l("dis-q4-fy2026-earnings", "2026-11-17", Lang::En).unwrap(),
            "Q4 fiscal 2026"
        );
        assert_eq!(
            l("brk-b-q3-fy2026-earnings-123456", "2026-11-03", Lang::En).unwrap(),
            "Q3 2026"
        );
        assert_eq!(
            l("cop-fy2026-earnings", "2027-02-05", Lang::En).unwrap(),
            "fiscal 2026"
        );
        assert_eq!(
            l("cop-q3-fy2026-earnings", "2026-11-06", Lang::Zh).unwrap(),
            "2026年第三季度"
        );
        assert_eq!(
            l("nvda-q3-fy2027-earnings", "2026-11-17", Lang::Zh).unwrap(),
            "2027财年第三季度"
        );
        assert_eq!(
            l("cop-fy2026-earnings", "2027-02-05", Lang::Zh).unwrap(),
            "2026财年全年"
        );
        // A slug named by date has no period.
        assert_eq!(l("qcom-2026-11-04-earnings", "2026-11-04", Lang::En), None);
    }
}
