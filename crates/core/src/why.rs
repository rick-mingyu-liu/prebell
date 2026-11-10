//! The Why of a mover without a verified news reason (decision 2026-11-10).
//! Instead of "No clear news", code states what the facts prove: that the
//! company reported results before this session's move (the earnings
//! calendar), else that it moved with one of the brief's themes. Co-movement
//! is stated, never a cause. The text and its one figure (the theme's
//! median, already in the facts) come from code, never from the LLM.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc, Weekday};

use crate::clock::{et, trading_day, DayKind};
use crate::cover::{fmt_signed_pct, Session};
use crate::facts::{Direction, Fallback, FallbackKind, Mover, Theme};
use crate::premarket::group;
use crate::store::{ReportRow, Store};

/// The last trading day before `date`.
pub fn previous_trading_day(date: NaiveDate) -> NaiveDate {
    let mut d = date;
    loop {
        d = d.pred_opt().expect("date in range");
        if trading_day(d) != DayKind::Closed {
            return d;
        }
    }
}

/// The reported calendar rows that can explain `date`'s session (from the
/// previous trading day through `date`), in one store read, or none and no
/// read when `needed` is false (every mover has a verified reason). A store
/// error reads as none, so the movers can still fall back to a theme; the
/// second value counts it for the diagnostics.
pub async fn read_reports(
    store: &dyn Store,
    date: NaiveDate,
    needed: bool,
) -> (Vec<ReportRow>, usize) {
    if !needed {
        return (vec![], 0);
    }
    let from = previous_trading_day(date).format("%Y-%m-%d").to_string();
    let to = date.format("%Y-%m-%d").to_string();
    match store.reported_between(&from, &to).await {
        Ok(rows) => (rows, 0),
        Err(_) => (vec![], 1),
    }
}

/// When a report came out, relative to the session's move.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Landed {
    BeforeOpen,
    /// Earlier today, at a time not proven (the recap only).
    Today,
    /// After the close of this trading day (the previous one).
    AfterClose(NaiveDate),
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid built-in time")
}

/// The ET close of trading day `d`: 13:00 on an early-close day, else 16:00.
fn close_of(d: NaiveDate) -> NaiveTime {
    match trading_day(d) {
        DayKind::EarlyClose => hm(13, 0),
        _ => hm(16, 0),
    }
}

/// The ET time of the report's first filing: the earliest acceptance of the
/// ticker's earnings filings on the report's ET date. `None` without one (a
/// foreign 6-K filer's report has no filing row).
fn filed_at(r: &ReportRow, day: NaiveDate) -> Option<NaiveTime> {
    r.accepted_at
        .iter()
        .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| et(t.with_timezone(&Utc)))
        .filter(|t| t.date_naive() == day)
        .map(|t| t.time())
        .min()
}

/// When `r` came out, if that is proven to precede the move `session` covers
/// on `date`. The 8-K is furnished at or after the results release, so its
/// acceptance time bounds the release from above only: it proves "before"
/// a time, never "after" one.
/// - today: the premarket brief is built before the open, so a report the
///   calendar already marks out today came before it, timed or not. The
///   recap needs the filing time: before 09:30 ET is before the open; during
///   the session proves only that results came out today; after the close
///   explains nothing;
/// - the previous trading day, filed at or after its close, or a day in
///   between (a holiday or weekend) with a filing that day: "after that
///   close" also needs the calendar's slot not to be `bmo`, since a release
///   before that day's open with a late 8-K was already traded on.
///
/// A report without a filing time gets no results fallback outside the
/// premarket's today: a reported row keeps no record of whether its slot was
/// ever confirmed.
fn landed(r: &ReportRow, date: NaiveDate, session: Session) -> Option<Landed> {
    let day: NaiveDate = r.report_date.parse().ok()?;
    let prev = previous_trading_day(date);
    let at = filed_at(r, day);
    if day == date {
        if session == Session::Premarket {
            return Some(Landed::BeforeOpen);
        }
        let t = at?;
        if t < hm(9, 30) {
            Some(Landed::BeforeOpen)
        } else if t < close_of(date) {
            Some(Landed::Today)
        } else {
            None
        }
    } else if day >= prev && day < date {
        let t = at?;
        let after = day > prev || t >= close_of(prev);
        (after && r.time_slot != "bmo").then_some(Landed::AfterClose(prev))
    } else {
        None
    }
}

fn weekday_zh(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

/// "Reported results before the open" / "... today" / "... after Monday's
/// close", and the ZH.
fn results(r: &ReportRow, landed: Landed) -> Fallback {
    let (text_en, text_zh) = match landed {
        Landed::BeforeOpen => (
            "Reported results before the open".to_string(),
            "已于盘前公布财报".to_string(),
        ),
        Landed::Today => (
            "Reported results today".to_string(),
            "已于今日公布财报".to_string(),
        ),
        Landed::AfterClose(d) => (
            format!("Reported results after {}'s close", d.format("%A")),
            format!("已于{}盘后公布财报", weekday_zh(d.weekday())),
        ),
    };
    // The earnings check when the site has one, else the SEC filing.
    let post_slug = r.post_slug.clone().filter(|s| !s.is_empty());
    let filing_url = r
        .proof_url
        .clone()
        .filter(|u| post_slug.is_none() && u.starts_with("https://www.sec.gov/"));
    Fallback {
        kind: FallbackKind::Results,
        text_en,
        text_zh,
        post_slug,
        filing_url,
    }
}

/// A group name inside a sentence: "Networking & optical" becomes
/// "networking & optical"; a leading acronym stays ("AI infrastructure &
/// data centers", "REITs & real estate"). Only the first letter of a group
/// name is ever capitalised for the sentence's sake.
fn in_sentence(group: &str) -> String {
    let first = group.split([' ', '-']).next().unwrap_or("");
    if first.chars().filter(|c| c.is_uppercase()).count() > 1 {
        return group.to_string();
    }
    let mut chars = group.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "Fell with the networking & optical group (median −4.63%); no company
/// news found", when the mover is in a theme's group (any member, not only
/// the listed ones) and moved the theme's way. The median is the theme's,
/// as its list line prints it. The "no company news found" tail only when
/// the mover's news search completed (`Mover::search_failed`).
fn theme(m: &Mover, themes: &[Theme]) -> Option<Fallback> {
    let g = group(&m.quote.symbol)?;
    let t = themes.iter().find(|t| t.group == g)?;
    let with = match t.direction {
        Direction::Up => m.quote.change_pct > 0.0,
        Direction::Down => m.quote.change_pct < 0.0,
    };
    if !with {
        return None;
    }
    let (en, zh) = match t.direction {
        Direction::Up => ("Rose", "上涨"),
        Direction::Down => ("Fell", "下跌"),
    };
    let median = fmt_signed_pct(t.median_pct);
    let (tail_en, tail_zh) = if m.search_failed {
        ("", "")
    } else {
        ("; no company news found", "，暂无个股消息")
    };
    Some(Fallback {
        kind: FallbackKind::Theme,
        text_en: format!(
            "{en} with the {} group (median {median}){tail_en}",
            in_sentence(&t.group)
        ),
        text_zh: format!("随{}板块{zh}（中位数 {median}）{tail_zh}", t.group_zh),
        post_slug: None,
        filing_url: None,
    })
}

/// Sets each mover's `fallback`: none with a verified reason, else reported
/// results from `reports` (the latest row of its ticker that came out before
/// the move), else its theme, else none. Returns how many got results and
/// how many a theme, for the diagnostics.
pub fn fill<'a>(
    movers: impl IntoIterator<Item = &'a mut Mover>,
    themes: &[Theme],
    reports: &[ReportRow],
    date: NaiveDate,
    session: Session,
) -> (usize, usize) {
    let (mut by_results, mut by_theme) = (0, 0);
    for m in movers {
        m.fallback = if !m.reasons.is_empty() {
            None
        } else if let Some((r, l)) = reports
            .iter()
            .rev()
            .filter(|r| r.ticker == m.quote.symbol)
            .find_map(|r| Some((r, landed(r, date, session)?)))
        {
            by_results += 1;
            Some(results(r, l))
        } else {
            let t = theme(m, themes);
            by_theme += usize::from(t.is_some());
            t
        };
    }
    (by_results, by_theme)
}

/// The diagnostics line for `fill`'s counts and `read_reports`' error, or
/// none when neither fallback was set and the read did not fail, so a quiet
/// build's diagnostics stay as they were.
pub fn diagnostics(counts: (usize, usize), reports_err: usize) -> Option<String> {
    (counts != (0, 0) || reports_err > 0).then(|| {
        format!(
            "why results={} theme={} reports_err={reports_err}",
            counts.0, counts.1
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{NewsReason, Quote, ThemeMember};
    use crate::numbers::{check, Lang};
    use crate::store::MemStore;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    /// Tuesday 2026-11-10; the previous trading day is Monday the 9th.
    fn tue() -> NaiveDate {
        d("2026-11-10")
    }
    fn mover(sym: &str, pct: f64) -> Mover {
        Mover {
            quote: Quote {
                symbol: sym.into(),
                last: 100.0,
                prev_close: 100.0 / (1.0 + pct / 100.0),
                change_pct: pct,
                volume: None,
            },
            name: sym.into(),
            reasons: vec![],
            search_failed: false,
            fallback: None,
        }
    }
    fn reason() -> NewsReason {
        NewsReason {
            url: "https://news.test/a".into(),
            title: "t".into(),
            quote: "Shares fell after the company cut its outlook".into(),
            summary_en: "Cut its outlook.".into(),
        }
    }
    /// Networking & optical down, median −4.63%, listing only COHR and CIEN.
    fn optical() -> Theme {
        let member = |t: &str, p: f64| ThemeMember {
            ticker: t.into(),
            name: t.into(),
            change_pct: p,
        };
        Theme {
            group: "Networking & optical".into(),
            group_zh: "网络与光通信".into(),
            direction: Direction::Down,
            members: vec![member("COHR", -5.45), member("CIEN", -4.63)],
            more: 0,
            median_pct: -4.63,
            reasons: vec![],
        }
    }
    /// Filing times, in UTC (ET is UTC−5 in these November weeks).
    const MON_0700: &str = "2026-11-09T12:00:00Z";
    const MON_1605: &str = "2026-11-09T21:05:00Z";
    const TUE_0700: &str = "2026-11-10T12:00:00Z";
    const TUE_1200: &str = "2026-11-10T17:00:00Z";
    const TUE_1610: &str = "2026-11-10T21:10:00Z";
    /// A reported row with these filing acceptance times, expected after
    /// the close.
    fn row(ticker: &str, date: &str, accepted: &[&str]) -> ReportRow {
        slotted(ticker, date, "amc", accepted)
    }
    fn slotted(ticker: &str, date: &str, slot: &str, accepted: &[&str]) -> ReportRow {
        ReportRow {
            ticker: ticker.into(),
            report_date: date.into(),
            time_slot: slot.into(),
            accepted_at: accepted.iter().map(|a| a.to_string()).collect(),
            proof_url: Some(
                "https://www.sec.gov/Archives/edgar/data/1633978/000163397826000041/0001633978-26-000041-index.htm"
                    .into(),
            ),
            post_slug: None,
        }
    }
    fn one(
        m: Mover,
        themes: &[Theme],
        reports: &[ReportRow],
        session: Session,
    ) -> Option<Fallback> {
        let mut ms = [m];
        fill(ms.iter_mut(), themes, reports, tue(), session);
        let [m] = ms;
        m.fallback
    }
    fn text(f: &Option<Fallback>) -> Option<&str> {
        f.as_ref().map(|f| f.text_en.as_str())
    }

    #[test]
    fn previous_trading_day_skips_weekends_and_holidays() {
        assert_eq!(previous_trading_day(tue()), d("2026-11-09"));
        assert_eq!(previous_trading_day(d("2026-11-09")), d("2026-11-06"));
        // Thanksgiving, Thursday 2026-11-26, is closed.
        assert_eq!(previous_trading_day(d("2026-11-27")), d("2026-11-25"));
    }

    #[test]
    fn news_beats_results_beats_theme_beats_none() {
        let themes = [optical()];
        let reports = [row("LITE", "2026-11-09", &[MON_1605])];
        let mut with_news = mover("LITE", -5.55);
        with_news.reasons = vec![reason()];
        // A stale fallback from an earlier fill goes once a reason is in.
        with_news.fallback = theme(&mover("LITE", -5.55), &themes);
        assert_eq!(one(with_news, &themes, &reports, Session::Premarket), None);
        let f = one(mover("LITE", -5.55), &themes, &reports, Session::Premarket);
        assert_eq!(f.as_ref().unwrap().kind, FallbackKind::Results);
        assert_eq!(text(&f), Some("Reported results after Monday's close"));
        let f = one(mover("LITE", -5.55), &themes, &[], Session::Premarket);
        assert_eq!(f.as_ref().unwrap().kind, FallbackKind::Theme);
        assert_eq!(
            one(mover("LITE", -5.55), &[], &[], Session::Premarket),
            None
        );
    }

    #[test]
    fn theme_text_in_both_languages_for_a_member_outside_the_listed_ones() {
        // LITE is in the group but not among the listed members.
        assert!(!optical().members.iter().any(|m| m.ticker == "LITE"));
        let f = one(mover("LITE", -5.55), &[optical()], &[], Session::Premarket).unwrap();
        assert_eq!(
            f.text_en,
            "Fell with the networking & optical group (median −4.63%); no company news found"
        );
        assert_eq!(
            f.text_zh,
            "随网络与光通信板块下跌（中位数 −4.63%），暂无个股消息"
        );
        assert_eq!((f.post_slug, f.filing_url), (None, None));
        // A rising theme reads "Rose".
        let mut up = optical();
        up.direction = Direction::Up;
        up.median_pct = 3.1;
        let f = one(mover("LITE", 4.2), &[up], &[], Session::Recap).unwrap();
        assert_eq!(
            f.text_en,
            "Rose with the networking & optical group (median +3.10%); no company news found"
        );
        assert_eq!(
            f.text_zh,
            "随网络与光通信板块上涨（中位数 +3.10%），暂无个股消息"
        );
    }

    #[test]
    fn theme_text_drops_the_no_news_tail_when_the_search_failed() {
        let mut m = mover("LITE", -5.55);
        m.search_failed = true;
        let f = one(m, &[optical()], &[], Session::Premarket).unwrap();
        assert_eq!(
            f.text_en,
            "Fell with the networking & optical group (median −4.63%)"
        );
        assert_eq!(f.text_zh, "随网络与光通信板块下跌（中位数 −4.63%）");
        assert_eq!(check(&f.text_en, Lang::En, &[-4.63]), Ok(()));
        assert_eq!(check(&f.text_zh, Lang::Zh, &[-4.63]), Ok(()));
    }

    #[test]
    fn a_mover_against_its_theme_or_outside_every_group_gets_none() {
        // Rose while its group's theme fell.
        assert_eq!(
            one(mover("LITE", 3.0), &[optical()], &[], Session::Premarket),
            None
        );
        // Another group's theme says nothing about it.
        assert_eq!(
            one(mover("NVDA", -5.0), &[optical()], &[], Session::Premarket),
            None
        );
        // A ticker outside the universe has no group.
        assert_eq!(
            one(mover("ZZZZ", -5.0), &[optical()], &[], Session::Premarket),
            None
        );
    }

    #[test]
    fn theme_median_passes_the_number_check() {
        let f = one(mover("LITE", -5.55), &[optical()], &[], Session::Recap).unwrap();
        let allowed = [-4.63];
        assert_eq!(check(&f.text_en, Lang::En, &allowed), Ok(()));
        assert_eq!(check(&f.text_zh, Lang::Zh, &allowed), Ok(()));
    }

    #[test]
    fn group_names_read_naturally_in_a_sentence() {
        for (g, want) in [
            ("Semiconductors", "semiconductors"),
            ("Networking & optical", "networking & optical"),
            (
                "Electrical & grid infrastructure",
                "electrical & grid infrastructure",
            ),
            ("Mega-cap tech platforms", "mega-cap tech platforms"),
            ("Autos & EV", "autos & EV"),
            (
                "AI infrastructure & data centers",
                "AI infrastructure & data centers",
            ),
            ("IT services & hardware", "IT services & hardware"),
            ("REITs & real estate", "REITs & real estate"),
            ("Tower & data center REITs", "tower & data center REITs"),
        ] {
            assert_eq!(in_sentence(g), want);
        }
    }

    #[test]
    fn premarket_results_today_timed_or_not_or_after_the_previous_close() {
        let p = Session::Premarket;
        // The brief is built before the open: a report out today preceded it.
        for accepted in [&[TUE_0700][..], &[]] {
            let f = one(
                mover("LITE", -5.0),
                &[],
                &[row("LITE", "2026-11-10", accepted)],
                p,
            );
            assert_eq!(text(&f), Some("Reported results before the open"));
            assert_eq!(f.unwrap().text_zh, "已于盘前公布财报");
        }
        let f = one(
            mover("LITE", -5.0),
            &[],
            &[row("LITE", "2026-11-09", &[MON_1605])],
            p,
        );
        assert_eq!(text(&f), Some("Reported results after Monday's close"));
        assert_eq!(f.unwrap().text_zh, "已于周一盘后公布财报");
        // Filed before Monday's close (whatever slot an aggregator listed),
        // or with no filing time: Monday's session may already have had it.
        for accepted in [&[MON_0700][..], &[]] {
            let r = [row("LITE", "2026-11-09", accepted)];
            assert_eq!(one(mover("LITE", -5.0), &[], &r, p), None, "{accepted:?}");
        }
        // The earliest filing of the day decides.
        let r = [row("LITE", "2026-11-09", &[MON_1605, MON_0700])];
        assert_eq!(one(mover("LITE", -5.0), &[], &r, p), None);
        // A filing time on another ET date is not this report's.
        let r = [row("LITE", "2026-11-09", &["2026-11-09T03:00:00Z"])];
        assert_eq!(one(mover("LITE", -5.0), &[], &r, p), None);
        // Friday's brief after Thanksgiving: a holiday filing came after
        // Wednesday's close.
        let mut ms = [mover("LITE", -5.0)];
        let r = [row("LITE", "2026-11-26", &["2026-11-26T15:00:00Z"])];
        fill(ms.iter_mut(), &[], &r, d("2026-11-27"), p);
        assert_eq!(
            text(&ms[0].fallback),
            Some("Reported results after Wednesday's close")
        );
        assert_eq!(
            ms[0].fallback.as_ref().unwrap().text_zh,
            "已于周三盘后公布财报"
        );
        // Untimed, the holiday row proves nothing.
        let mut ms = [mover("LITE", -5.0)];
        fill(
            ms.iter_mut(),
            &[],
            &[row("LITE", "2026-11-26", &[])],
            d("2026-11-27"),
            p,
        );
        assert_eq!(ms[0].fallback, None);
    }

    #[test]
    fn recap_results_follow_the_filing_time_today() {
        let r = Session::Recap;
        let today = |a: &[&str]| one(mover("LITE", -5.0), &[], &[row("LITE", "2026-11-10", a)], r);
        assert_eq!(
            text(&today(&[TUE_0700])),
            Some("Reported results before the open")
        );
        // Filed during the session proves only that results are out today:
        // a 06:45 ET release with its 8-K accepted at 10:20 ET was out
        // before the open.
        for a in [TUE_1200, "2026-11-10T15:20:00Z"] {
            let f = today(&[a]);
            assert_eq!(text(&f), Some("Reported results today"), "{a}");
            assert_eq!(f.unwrap().text_zh, "已于今日公布财报");
        }
        // After today's close, or untimed: not shown to precede the move.
        assert_eq!(today(&[TUE_1610]), None);
        assert_eq!(today(&[]), None);
        let f = one(
            mover("LITE", -5.0),
            &[],
            &[row("LITE", "2026-11-09", &[MON_1605])],
            r,
        );
        assert_eq!(text(&f), Some("Reported results after Monday's close"));
        // Another ticker's report is not this mover's.
        let rows = [row("COHR", "2026-11-10", &[TUE_0700])];
        assert_eq!(one(mover("LITE", -5.0), &[], &rows, r), None);
        // The early close (Friday 2026-11-27, 13:00 ET) ends the session.
        let mut ms = [mover("LITE", -5.0)];
        let rows = [row("LITE", "2026-11-27", &["2026-11-27T18:30:00Z"])];
        fill(ms.iter_mut(), &[], &rows, d("2026-11-27"), r);
        assert_eq!(ms[0].fallback, None);
    }

    #[test]
    fn after_the_close_needs_a_late_filing_and_a_slot_that_is_not_bmo() {
        let p = Session::Premarket;
        // A Monday 07:00 ET release with its 8-K accepted at 16:30 ET: the
        // filing is after the close, but the slot says before the open, so
        // Monday's session already traded on it.
        let monday_1630 = "2026-11-09T21:30:00Z";
        let r = [slotted("LITE", "2026-11-09", "bmo", &[monday_1630])];
        assert_eq!(one(mover("LITE", -5.0), &[], &r, p), None);
        for slot in ["amc", "unknown"] {
            let r = [slotted("LITE", "2026-11-09", slot, &[monday_1630])];
            let f = one(mover("LITE", -5.0), &[], &r, p);
            assert_eq!(
                text(&f),
                Some("Reported results after Monday's close"),
                "{slot}"
            );
        }
        // A weekend filing before Monday's brief: after Friday's close unless
        // the slot was bmo.
        let saturday = "2026-11-07T15:00:00Z";
        for (slot, want) in [
            ("amc", Some("Reported results after Friday's close")),
            ("unknown", Some("Reported results after Friday's close")),
            ("bmo", None),
        ] {
            let mut ms = [mover("LITE", -5.0)];
            let r = [slotted("LITE", "2026-11-07", slot, &[saturday])];
            fill(ms.iter_mut(), &[], &r, d("2026-11-09"), p);
            assert_eq!(text(&ms[0].fallback), want, "{slot}");
        }
        // Early-close Friday 2026-11-27 (13:00 ET), Monday's brief: filed at
        // 13:30 ET is after that close, at 12:30 ET it is not.
        for (accepted, want) in [
            (
                "2026-11-27T18:30:00Z",
                Some("Reported results after Friday's close"),
            ),
            ("2026-11-27T17:30:00Z", None),
        ] {
            let mut ms = [mover("LITE", -5.0)];
            let r = [row("LITE", "2026-11-27", &[accepted])];
            fill(ms.iter_mut(), &[], &r, d("2026-11-30"), p);
            assert_eq!(text(&ms[0].fallback), want, "{accepted}");
        }
    }

    #[test]
    fn results_link_the_earnings_check_else_the_sec_filing() {
        let p = Session::Premarket;
        let mut r = row("LITE", "2026-11-09", &[MON_1605]);
        r.post_slug = Some("lite-q1-fy2027-earnings".into());
        let f = one(mover("LITE", -5.0), &[], &[r.clone()], p).unwrap();
        assert_eq!(f.post_slug.as_deref(), Some("lite-q1-fy2027-earnings"));
        assert_eq!(f.filing_url, None);
        r.post_slug = None;
        let f = one(mover("LITE", -5.0), &[], &[r.clone()], p).unwrap();
        assert_eq!(f.filing_url, r.proof_url);
        // Only an SEC link is a filing link.
        r.proof_url = Some("https://ir.example.com/news".into());
        let f = one(mover("LITE", -5.0), &[], &[r.clone()], p).unwrap();
        assert_eq!((f.post_slug, f.filing_url), (None, None));
        r.proof_url = None;
        let f = one(mover("LITE", -5.0), &[], &[r], p).unwrap();
        assert_eq!(f.filing_url, None);
    }

    #[test]
    fn fill_counts_each_kind() {
        let mut news = mover("COHR", -5.45);
        news.reasons = vec![reason()];
        let mut ms = [
            mover("LITE", -5.55),
            mover("CIEN", -4.63),
            news,
            mover("NVDA", 6.0),
        ];
        let counts = fill(
            ms.iter_mut(),
            &[optical()],
            &[row("LITE", "2026-11-10", &[TUE_0700])],
            tue(),
            Session::Premarket,
        );
        assert_eq!(counts, (1, 1));
        assert_eq!(
            diagnostics(counts, 0).as_deref(),
            Some("why results=1 theme=1 reports_err=0")
        );
        assert_eq!(
            diagnostics((0, 0), 1).as_deref(),
            Some("why results=0 theme=0 reports_err=1")
        );
        assert_eq!(diagnostics((0, 0), 0), None);
    }

    #[tokio::test]
    async fn reports_are_one_read_over_the_previous_trading_day_and_today() {
        let s = MemStore::default();
        *s.reports.borrow_mut() = vec![
            row("LITE", "2026-11-06", &[]),
            row("LITE", "2026-11-09", &[MON_1605]),
            row("COHR", "2026-11-10", &[TUE_0700]),
        ];
        let (rows, err) = read_reports(&s, tue(), true).await;
        let got: Vec<_> = rows.iter().map(|r| r.report_date.as_str()).collect();
        assert_eq!((got, err), (vec!["2026-11-09", "2026-11-10"], 0));
        // Nothing to explain: no read.
        s.fail_reports.set(true);
        assert_eq!(read_reports(&s, tue(), false).await, (vec![], 0));
        // A store error reads as none.
        assert_eq!(read_reports(&s, tue(), true).await, (vec![], 1));
    }
}
