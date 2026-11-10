//! The stock's move when an earnings check is built: one Yahoo v7 quote
//! (`Yahoo::session_quote`), read in the session the build runs in. Best
//! effort only: a closed market, a stale or incomplete quote, a Yahoo error
//! or a spent subrequest budget leaves the move `Missing`, and the check
//! publishes without it. Nothing is retried.

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;

use crate::budget::allows;
use crate::clock::{et, previous_close, trading_day, DayKind};
use crate::earnings::facts::{ShareMove, ShareSession};
use crate::events::et_time;
use crate::facts::{round2, Field};
use crate::numbers::Lang;
use crate::providers::{Http, ProviderError};
use crate::yahoo::{SessionQuote, Yahoo};

/// Fetches one read may use: the cookie, the crumb and the quote, and the
/// same three again when a 401 refreshes the crumb.
pub const SHARE_QUOTE_COST: usize = 6;

/// Yahoo's percent and the one the prices give may differ by this many
/// points before the fields count as anchored to another session.
const PCT_TOLERANCE: f64 = 0.5;

/// One symbol's quote for every session (Yahoo in production).
#[async_trait(?Send)]
pub trait ShareQuotes {
    async fn session_quote(&self, symbol: &str) -> Result<SessionQuote, ProviderError>;
}

#[async_trait(?Send)]
impl<H: Http> ShareQuotes for Yahoo<H> {
    async fn session_quote(&self, symbol: &str) -> Result<SessionQuote, ProviderError> {
        Yahoo::session_quote(self, symbol).await
    }
}

/// A quote source and the invocation's fetch count so far
/// (`crate::budget::allows` decides whether the read fits).
pub struct Shares<'a> {
    pub quotes: &'a dyn ShareQuotes,
    pub budget: &'a dyn Fn() -> usize,
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid built-in time")
}

/// The move for a build at `now` of a filing accepted at `accepted_at`, or
/// why there is none.
pub async fn read(
    shares: Option<&Shares<'_>>,
    ticker: &str,
    accepted_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Field<ShareMove> {
    let missing = |reason: String| Field::Missing { reason };
    let Some(s) = shares else {
        return missing("no quote source".into());
    };
    // No fetch on a weekend or holiday: there is no session to report.
    if trading_day(et(now).date_naive()) == DayKind::Closed {
        return missing("market closed".into());
    }
    if !allows((s.budget)(), SHARE_QUOTE_COST) {
        return missing("subrequest budget".into());
    }
    // Yahoo writes class shares with a dash; the watchlist with a dot.
    match s.quotes.session_quote(&ticker.replace('.', "-")).await {
        Ok(q) => share_move(&q, accepted_at, now).map_or_else(missing, Field::Value),
        Err(e) => missing(format!("yahoo: {e}")),
    }
}

/// The move in the session of `now` (ET):
/// - before 09:30, the premarket price against the previous close;
/// - from 09:30 to the close (16:00, 13:00 on an early-close day), the
///   regular-session price against the previous close;
/// - from the close, the after-hours price against today's close.
///
/// The quote's own time must fall in that session today; anything else is
/// left over from another session and is no move at all. It must also be no
/// earlier than the filing's acceptance (`accepted_at`): a trade from before
/// the report is not the shares' answer to it.
///
/// Premarket and after hours, the reference close must also come no later
/// than the acceptance, so the move is the reaction since the last close
/// before the report (a report at noon, built at 4:10 PM, has no after-hours
/// move: its reaction was in regular trading). Regular trading always
/// measures from the previous close, so part of the day's move may predate a
/// report filed during the session; the prompts never call it a reaction.
pub fn share_move(
    q: &SessionQuote,
    accepted_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<ShareMove, String> {
    let local = et(now);
    let day = local.date_naive();
    let close = match trading_day(day) {
        DayKind::Closed => return Err("market closed".into()),
        DayKind::EarlyClose => hm(13, 0),
        DayKind::Full => hm(16, 0),
    };
    let open = hm(9, 30);
    let today = |t: Option<DateTime<Utc>>| t.map(et).filter(|t| t.date_naive() == day);
    let (session, last, ref_close, pct, at) = if local.time() < open {
        let at = today(q.pre_time)
            .filter(|t| t.time() < open)
            .ok_or("stale: no premarket trade today")?;
        // Before the open, the regular price is still the last close.
        let ref_close = q.regular_price;
        (
            ShareSession::Premarket,
            q.pre_price,
            ref_close,
            q.pre_change_pct,
            at,
        )
    } else if local.time() < close {
        let at = today(q.regular_time)
            .filter(|t| t.time() >= open)
            .ok_or("stale: no regular-session trade today")?;
        let ref_close = q.regular_prev_close;
        (
            ShareSession::Regular,
            q.regular_price,
            ref_close,
            q.regular_change_pct,
            at,
        )
    } else {
        let at = today(q.post_time)
            .filter(|t| t.time() >= close)
            .ok_or("stale: no after-hours trade today")?;
        // The reference is today's close: the regular session's final print
        // (one minute of slack, as for the recap's closes).
        today(q.regular_time)
            .filter(|t| t.time() >= close - Duration::minutes(1))
            .ok_or("stale: no close today")?;
        (
            ShareSession::AfterHours,
            q.post_price,
            q.regular_price,
            q.post_change_pct,
            at,
        )
    };
    if at.with_timezone(&Utc) < accepted_at {
        return Err("stale: no trade since the report".into());
    }
    let ref_time = match session {
        ShareSession::Premarket => Some(previous_close(now)),
        ShareSession::AfterHours => New_York
            .from_local_datetime(&day.and_time(close))
            .earliest()
            .map(|t| t.with_timezone(&Utc)),
        ShareSession::Regular => None,
    };
    if ref_time.is_some_and(|t| t > accepted_at) {
        return Err("stale: the report came before the reference close".into());
    }
    let last = last.filter(|x| x.is_finite() && *x > 0.0);
    let ref_close = ref_close.filter(|x| x.is_finite() && *x > 0.0);
    let (Some(last), Some(ref_close)) = (last, ref_close) else {
        return Err("incomplete quote".into());
    };
    // The prices give the percent, so the three numbers always agree. It is
    // rounded once, here, to the one decimal the page prints.
    let change_pct = (last - ref_close) / ref_close * 100.0;
    if pct.is_some_and(|p| !p.is_finite() || (p - change_pct).abs() > PCT_TOLERANCE) {
        return Err("stale: the percent and the prices disagree".into());
    }
    Ok(ShareMove {
        session,
        change_pct: (change_pct * 10.0).round() / 10.0,
        last: round2(last),
        ref_close: round2(ref_close),
        as_of: at.with_timezone(&Utc),
    })
}

/// The line under the filing line: "Shares: -4.2% in premarket trading (as
/// of 8:12 AM ET)" / "股价：盘前下跌4.2%（截至美东时间上午8:12）". The move
/// is stored to one decimal; zero is "unchanged" / "持平". The sign is an
/// ASCII "-", as in the key-figures table below it (`fmt_value`).
pub fn share_line(m: &ShareMove, lang: Lang) -> String {
    let pct = format!("{:.1}%", m.change_pct.abs());
    match lang {
        Lang::En => {
            let change = match m.change_pct {
                t if t > 0.0 => format!("+{pct}"),
                t if t < 0.0 => format!("-{pct}"),
                _ => "unchanged".into(),
            };
            let session = match m.session {
                ShareSession::Premarket => "in premarket trading",
                ShareSession::Regular => "in regular trading",
                ShareSession::AfterHours => "after hours",
            };
            format!(
                "Shares: {change} {session} (as of {})",
                et_time(m.as_of, Lang::En)
            )
        }
        Lang::Zh => {
            let change = match m.change_pct {
                t if t > 0.0 => format!("上涨{pct}"),
                t if t < 0.0 => format!("下跌{pct}"),
                _ => "持平".into(),
            };
            let session = match m.session {
                ShareSession::Premarket => "盘前",
                ShareSession::Regular => "盘中",
                ShareSession::AfterHours => "盘后",
            };
            // "上午8:12（美东）" without its zone note, which leads here.
            let clock = et_time(m.as_of, Lang::Zh);
            let clock = clock.trim_end_matches("（美东）");
            format!("股价：{session}{change}（截至美东时间{clock}）")
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::numbers::check;
    use std::cell::{Cell, RefCell};

    fn t(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse().unwrap())
    }
    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    /// A filing accepted at midnight ET on Tue 2026-11-10, before every
    /// trade that day and after Monday's close.
    fn early() -> DateTime<Utc> {
        utc("2026-11-10T05:00:00Z")
    }
    /// A filing accepted at 4:05 PM ET on Tue 2026-11-10, after the close.
    fn amc() -> DateTime<Utc> {
        utc("2026-11-10T21:05:00Z")
    }

    /// CHTR on Tue 2026-11-10 (EST, UTC-5), as one snapshot per session:
    /// closed at $340.00 on Monday.
    fn premarket() -> SessionQuote {
        SessionQuote {
            pre_price: Some(325.72),
            pre_change_pct: Some(-4.2),
            pre_time: t("2026-11-10T13:12:00Z"),
            regular_price: Some(340.0),
            regular_change_pct: Some(0.4),
            regular_prev_close: Some(338.65),
            regular_time: t("2026-11-09T21:00:00Z"),
            ..SessionQuote::default()
        }
    }
    fn regular() -> SessionQuote {
        SessionQuote {
            regular_price: Some(350.54),
            regular_change_pct: Some(3.1),
            regular_prev_close: Some(340.0),
            regular_time: t("2026-11-10T17:36:00Z"),
            ..premarket()
        }
    }
    fn after_hours() -> SessionQuote {
        SessionQuote {
            regular_price: Some(330.12),
            regular_change_pct: Some(-2.91),
            regular_prev_close: Some(340.0),
            regular_time: t("2026-11-10T21:00:00Z"),
            post_price: Some(310.31),
            post_change_pct: Some(-6.0),
            post_time: t("2026-11-10T21:22:00Z"),
            ..premarket()
        }
    }

    #[test]
    fn each_session_against_its_reference() {
        let m = share_move(&premarket(), early(), utc("2026-11-10T13:15:00Z")).unwrap();
        assert_eq!(
            m,
            ShareMove {
                session: ShareSession::Premarket,
                change_pct: -4.2,
                last: 325.72,
                ref_close: 340.0,
                as_of: utc("2026-11-10T13:12:00Z"),
            }
        );
        let m = share_move(&regular(), early(), utc("2026-11-10T17:40:00Z")).unwrap();
        assert_eq!(
            (m.session, m.change_pct, m.ref_close),
            (ShareSession::Regular, 3.1, 340.0)
        );
        let m = share_move(&after_hours(), amc(), utc("2026-11-10T21:25:00Z")).unwrap();
        assert_eq!(
            (m.session, m.change_pct, m.last, m.ref_close),
            (ShareSession::AfterHours, -6.0, 310.31, 330.12)
        );
        assert_eq!(m.as_of, utc("2026-11-10T21:22:00Z"));
    }

    #[test]
    fn the_move_is_rounded_once_from_the_prices() {
        let mut q = premarket();
        // -4.249%: one decimal from the prices, never -4.25 then -4.3.
        q.pre_price = Some(325.5534);
        q.pre_change_pct = None;
        let m = share_move(&q, early(), utc("2026-11-10T13:15:00Z")).unwrap();
        assert_eq!((m.change_pct, m.last), (-4.2, 325.55));
        assert_eq!(
            share_line(&m, Lang::En),
            "Shares: -4.2% in premarket trading (as of 8:12 AM ET)"
        );
    }

    #[test]
    fn a_trade_before_the_report_is_no_move() {
        // (quote, its trade time, the build's time) for each session.
        for (q, trade, now) in [
            (premarket(), "2026-11-10T13:12:00Z", "2026-11-10T13:15:00Z"),
            (regular(), "2026-11-10T17:36:00Z", "2026-11-10T17:40:00Z"),
            (
                after_hours(),
                "2026-11-10T21:22:00Z",
                "2026-11-10T21:25:00Z",
            ),
        ] {
            let trade = utc(trade);
            let at = |d: i64| share_move(&q, trade + Duration::minutes(d), utc(now));
            // Accepted a minute after the last trade: that trade predates the report.
            assert_eq!(
                at(1),
                Err("stale: no trade since the report".into()),
                "{trade}"
            );
            // Accepted with the trade, or a minute before it.
            assert_eq!(at(0).unwrap().as_of, trade);
            assert_eq!(at(-1).unwrap().as_of, trade);
        }
        // The review's case: a 6:41 AM premarket print, the 8-K at 7:00 AM.
        let mut q = premarket();
        q.pre_time = t("2026-11-10T11:41:00Z");
        assert!(share_move(&q, utc("2026-11-10T12:00:00Z"), utc("2026-11-10T12:05:00Z")).is_err());
    }

    #[test]
    fn extended_hours_measure_from_the_last_close_before_the_report() {
        let gone = "stale: the report came before the reference close".to_string();
        // Filed at noon Tuesday, built after the close: the reaction was in
        // regular trading, and the after-hours move from Tuesday's close is
        // not it.
        let noon = utc("2026-11-10T17:00:00Z");
        assert_eq!(
            share_move(&after_hours(), noon, utc("2026-11-10T21:25:00Z")),
            Err(gone.clone())
        );
        // Filed at noon Monday, built Tuesday before the open: measured from
        // Monday's close, after the report.
        assert_eq!(
            share_move(
                &premarket(),
                utc("2026-11-09T17:00:00Z"),
                utc("2026-11-10T13:15:00Z")
            ),
            Err(gone)
        );
        // Filed after Monday's close (or at it), built Tuesday premarket.
        for accepted in ["2026-11-09T21:00:00Z", "2026-11-09T21:05:00Z"] {
            assert!(share_move(&premarket(), utc(accepted), utc("2026-11-10T13:15:00Z")).is_ok());
        }
        // Regular trading always measures from the previous close; a
        // mid-session report keeps its move (the prompts never call it a
        // reaction to the report).
        let m = share_move(&regular(), noon, utc("2026-11-10T17:40:00Z")).unwrap();
        assert_eq!(m.session, ShareSession::Regular);
    }

    #[test]
    fn stale_quotes_are_no_move() {
        let stale = |q: SessionQuote, now: &str| {
            let e = share_move(&q, early(), utc(now)).unwrap_err();
            assert!(e.starts_with("stale") || e == "market closed", "{e}");
        };
        // Premarket fields from yesterday.
        let mut q = premarket();
        q.pre_time = t("2026-11-09T13:12:00Z");
        stale(q, "2026-11-10T13:15:00Z");
        // At 12:40 ET the last regular trade is yesterday's close.
        stale(premarket(), "2026-11-10T17:40:00Z");
        // After the close with no after-hours trade yet: the 4:00 PM print
        // is the close, not an after-hours move.
        let mut q = after_hours();
        q.post_time = t("2026-11-09T21:22:00Z");
        stale(q, "2026-11-10T21:25:00Z");
        // After-hours fields measured from a close that is not today's.
        let mut q = after_hours();
        q.regular_time = t("2026-11-10T19:00:00Z");
        stale(q, "2026-11-10T21:25:00Z");
        // Yahoo's percent anchored to another session.
        let mut q = premarket();
        q.pre_change_pct = Some(1.5);
        stale(q, "2026-11-10T13:15:00Z");
        // Saturday, and Thanksgiving.
        stale(after_hours(), "2026-11-14T15:00:00Z");
        stale(after_hours(), "2026-11-26T15:00:00Z");
    }

    #[test]
    fn early_close_day_switches_to_after_hours_at_1_pm() {
        // Fri 2026-11-27 closes at 13:00 ET (18:00Z).
        let q = SessionQuote {
            regular_price: Some(100.0),
            regular_prev_close: Some(98.0),
            regular_time: t("2026-11-27T18:00:00Z"),
            post_price: Some(101.0),
            post_time: t("2026-11-27T18:30:00Z"),
            ..SessionQuote::default()
        };
        let m = share_move(&q, utc("2026-11-27T18:05:00Z"), utc("2026-11-27T18:45:00Z")).unwrap();
        assert_eq!((m.session, m.change_pct), (ShareSession::AfterHours, 1.0));
    }

    #[test]
    fn an_incomplete_quote_is_no_move() {
        let mut q = premarket();
        q.pre_price = None;
        assert_eq!(
            share_move(&q, early(), utc("2026-11-10T13:15:00Z")),
            Err("incomplete quote".into())
        );
    }

    #[test]
    fn share_lines() {
        let at = |s| share_move(&s, early(), utc("2026-11-10T13:15:00Z")).unwrap();
        let pre = at(premarket());
        let reg = share_move(&regular(), early(), utc("2026-11-10T17:40:00Z")).unwrap();
        let post = share_move(&after_hours(), amc(), utc("2026-11-10T21:25:00Z")).unwrap();
        for (m, en, zh) in [
            (
                &pre,
                "Shares: -4.2% in premarket trading (as of 8:12 AM ET)",
                "股价：盘前下跌4.2%（截至美东时间上午8:12）",
            ),
            (
                &reg,
                "Shares: +3.1% in regular trading (as of 12:36 PM ET)",
                "股价：盘中上涨3.1%（截至美东时间下午12:36）",
            ),
            (
                &post,
                "Shares: -6.0% after hours (as of 4:22 PM ET)",
                "股价：盘后下跌6.0%（截至美东时间下午4:22）",
            ),
        ] {
            assert_eq!(share_line(m, Lang::En), en);
            assert_eq!(share_line(m, Lang::Zh), zh);
            // The line's numbers are the move's own.
            let n = [m.change_pct, m.last, m.ref_close];
            assert_eq!(check(en, Lang::En, &n), Ok(()), "{en}");
            assert_eq!(check(zh, Lang::Zh, &n), Ok(()), "{zh}");
            assert!(check(en, Lang::En, &[]).is_err());
        }
        let mut flat = pre.clone();
        flat.change_pct = 0.0;
        assert_eq!(
            share_line(&flat, Lang::En),
            "Shares: unchanged in premarket trading (as of 8:12 AM ET)"
        );
        assert_eq!(
            share_line(&flat, Lang::Zh),
            "股价：盘前持平（截至美东时间上午8:12）"
        );
    }

    /// Answers every quote request with `reply`; counts them.
    pub(crate) struct FakeQuotes {
        pub reply: RefCell<Result<SessionQuote, ProviderError>>,
        pub calls: Cell<usize>,
    }
    impl FakeQuotes {
        pub(crate) fn new(reply: Result<SessionQuote, ProviderError>) -> Self {
            Self {
                reply: RefCell::new(reply),
                calls: Cell::new(0),
            }
        }
    }
    #[async_trait(?Send)]
    impl ShareQuotes for FakeQuotes {
        async fn session_quote(&self, symbol: &str) -> Result<SessionQuote, ProviderError> {
            assert!(!symbol.contains('.'), "{symbol}");
            self.calls.set(self.calls.get() + 1);
            self.reply.borrow().clone()
        }
    }

    #[tokio::test]
    async fn reads_are_best_effort() {
        let now = utc("2026-11-10T13:15:00Z");
        let q = FakeQuotes::new(Ok(premarket()));
        let s = Shares {
            quotes: &q,
            budget: &|| 0,
        };
        assert!(matches!(
            read(Some(&s), "CHTR", early(), now).await,
            Field::Value(_)
        ));
        // A Yahoo error.
        *q.reply.borrow_mut() = Err(ProviderError::Http(429));
        assert_eq!(
            read(Some(&s), "CHTR", early(), now).await,
            Field::Missing {
                reason: "yahoo: http status 429".into()
            }
        );
        assert_eq!(q.calls.get(), 2);
        // The budget would pass the soft limit: no request at all.
        let s = Shares {
            quotes: &q,
            budget: &|| 35,
        };
        assert_eq!(
            read(Some(&s), "CHTR", early(), now).await,
            Field::Missing {
                reason: "subrequest budget".into()
            }
        );
        // A weekend: no request either.
        let s = Shares {
            quotes: &q,
            budget: &|| 0,
        };
        assert!(matches!(
            read(Some(&s), "BRK.B", early(), utc("2026-11-14T15:00:00Z")).await,
            Field::Missing { .. }
        ));
        assert_eq!(q.calls.get(), 2);
        assert!(matches!(
            read(None, "CHTR", early(), now).await,
            Field::Missing { .. }
        ));
    }
}
