//! IPO watch (design `docs/superpowers/specs/2026-11-11-ipo-watch-design.md`,
//! storage contract `2026-11-11-ipo-watch-contract.md`): a hand-kept list of
//! big upcoming listings (`data/ipo-watch.tsv`), their SEC filings and cover
//! facts (`check`, `cover`), dated "as reported" news items (`news`) and,
//! after listing, the recap's closes (`trading`). The date and status rules
//! are in `resolve`.
//!
//! No new cron: an earnings poll asks for the `ipo:check` and `ipo:sweep`
//! messages (`follow_ups`), only while `IPO_WATCH_ENABLED` is exactly `true`.

pub mod check;
pub mod cover;
pub mod news;
pub mod resolve;
pub mod store;
pub mod trading;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, SecondsFormat, Utc};

use crate::clock::{et, trading_day, DayKind};
use crate::earnings::store::EarningsStore;
use crate::schedule::EarningsStep;

const DATA: &str = include_str!("../../data/ipo-watch.tsv");

/// Rows the list may hold: each costs one EDGAR request per hourly check
/// and up to two Tavily searches a trading day.
pub const MAX_TRACKED: usize = 10;
/// The filing check runs at most this often (it rides the earnings poll).
pub const CHECK_EVERY_MINUTES: i64 = 60;
/// kv: when the last `ipo:check` started (RFC 3339).
pub const KV_CHECKED: &str = "ipo_checked_at";
/// kv: JSON `{slug: "YYYY-MM-DD:am|pm"}`, the last sweep slot per IPO.
pub const KV_SWEEPS: &str = "ipo_sweeps";
/// kv: `YYYY-MM N`, Tavily searches the sweeps made this month (ET).
pub const KV_SEARCHES: &str = "ipo_searches";

/// One tracked IPO, from `data/ipo-watch.tsv`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
    pub slug: String,
    pub cik: u64,
    pub name: String,
    pub name_zh: Option<String>,
    /// Other names a news quote may use for the company (`ipo::news`).
    pub aliases: Vec<String>,
}

impl Tracked {
    /// The display name and the aliases.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.aliases.iter().map(String::as_str))
    }
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 60
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// The list's rows; an error names the first bad line.
pub fn parse_list(data: &str) -> Result<Vec<Tracked>, String> {
    let mut out: Vec<Tracked> = Vec::new();
    for line in data.lines().map(str::trim_end) {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').map(str::trim).collect();
        let (Some(slug), Some(cik), Some(name), Some(zh)) =
            (c.first(), c.get(1), c.get(2), c.get(3))
        else {
            return Err(format!("ipo-watch.tsv: too few columns: {line}"));
        };
        let cik: u64 = cik
            .parse()
            .map_err(|_| format!("ipo-watch.tsv: bad cik: {line}"))?;
        if !slug_ok(slug) || name.is_empty() || cik == 0 {
            return Err(format!("ipo-watch.tsv: bad row: {line}"));
        }
        if out.iter().any(|t| t.slug == *slug || t.cik == cik) {
            return Err(format!("ipo-watch.tsv: duplicate slug or cik: {line}"));
        }
        out.push(Tracked {
            slug: slug.to_string(),
            cik,
            name: name.to_string(),
            name_zh: (!zh.is_empty() && *zh != "-").then(|| zh.to_string()),
            aliases: c
                .get(4)
                .filter(|a| !a.is_empty() && **a != "-")
                .map(|a| {
                    a.split(',')
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    if out.len() > MAX_TRACKED {
        return Err(format!(
            "ipo-watch.tsv: {} rows, at most {MAX_TRACKED}",
            out.len()
        ));
    }
    Ok(out)
}

/// The tracked IPOs (a test keeps the file parsing).
pub fn tracked() -> &'static [Tracked] {
    static T: OnceLock<Vec<Tracked>> = OnceLock::new();
    T.get_or_init(|| parse_list(DATA).unwrap_or_default())
}

/// `(form, EN, ZH)`: the forms the timeline keeps, with their labels. The
/// contract (`2026-11-11-ipo-watch-contract.md`) carries the same table.
pub const FORMS: &[(&str, &str, &str)] = &[
    (
        "DRS",
        "Draft registration statement (confidential)",
        "保密提交的注册草案",
    ),
    (
        "DRS/A",
        "Amended draft registration statement",
        "注册草案修订",
    ),
    ("S-1", "Registration statement (S-1)", "注册声明（S-1）"),
    (
        "S-1/A",
        "Amended registration statement (S-1/A)",
        "注册声明修订（S-1/A）",
    ),
    ("F-1", "Registration statement (F-1)", "注册声明（F-1）"),
    (
        "F-1/A",
        "Amended registration statement (F-1/A)",
        "注册声明修订（F-1/A）",
    ),
    ("424B1", "Final prospectus", "最终招股说明书"),
    ("424B4", "Final prospectus", "最终招股说明书"),
    ("8-A12B", "Exchange registration (8-A)", "交易所注册（8-A）"),
    (
        "EFFECT",
        "Declared effective by the SEC",
        "SEC 宣布注册生效",
    ),
    ("CERT", "Exchange certified the listing", "交易所批准上市"),
    ("RW", "Registration withdrawn", "撤回注册"),
];

pub fn is_kept(form: &str) -> bool {
    FORMS.iter().any(|(f, _, _)| *f == form)
}
/// A registration statement whose cover may state the offering.
pub fn is_registration(form: &str) -> bool {
    matches!(form, "S-1" | "S-1/A" | "F-1" | "F-1/A")
}
/// A final prospectus: the offering is priced.
pub fn is_final(form: &str) -> bool {
    matches!(form, "424B1" | "424B4")
}
pub fn is_draft(form: &str) -> bool {
    matches!(form, "DRS" | "DRS/A")
}

/// `IPO_WATCH_ENABLED`: on only when it is exactly `true` (no spaces, no
/// other case); unset or anything else is off.
pub fn enabled(raw: Option<&str>) -> bool {
    raw == Some("true")
}

pub(crate) fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// The first trading day after `d` (`clock`'s NYSE table; an early close
/// is a trading day).
pub fn next_trading_day(d: NaiveDate) -> NaiveDate {
    let mut n = d;
    loop {
        n += Duration::days(1);
        if trading_day(n) != DayKind::Closed {
            return n;
        }
    }
}

/// The `n`th trading day after `d`.
pub fn trading_days_after(d: NaiveDate, n: usize) -> NaiveDate {
    (0..n).fold(d, |x, _| next_trading_day(x))
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid built-in time")
}

/// The news sweep's slot at `now`: one a trading day, `YYYY-MM-DD:pm`
/// from 16:30 ET (after the close); `None` before then and on closed
/// days. One sweep a day, not two, keeps the Tavily allowance for the
/// briefs (lead's ruling 2026-11-11; `docs/decisions.md`).
pub fn slot(now: DateTime<Utc>) -> Option<String> {
    let l = et(now);
    let day = l.date_naive();
    if trading_day(day) == DayKind::Closed || l.time() < hm(16, 30) {
        return None;
    }
    Some(format!("{}:pm", ymd(day)))
}

/// slug -> the last slot swept.
pub type Sweeps = BTreeMap<String, String>;

/// The sweep marks; unreadable JSON reads as none swept.
pub async fn read_sweeps(es: &dyn EarningsStore) -> Result<Sweeps, String> {
    Ok(es
        .kv_get(KV_SWEEPS)
        .await?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

/// The first tracked IPO not yet swept in `slot`.
pub fn due_sweep<'a>(list: &'a [Tracked], sweeps: &Sweeps, slot: &str) -> Option<&'a Tracked> {
    list.iter()
        .find(|t| sweeps.get(&t.slug).map(String::as_str) != Some(slot))
}

/// Whether the filing check is due: never run, unreadable, or an hour old.
pub async fn check_due(es: &dyn EarningsStore, now: DateTime<Utc>) -> Result<bool, String> {
    Ok(match es.kv_get(KV_CHECKED).await? {
        Some(t) => DateTime::parse_from_rfc3339(&t)
            .map(|t| now - t.with_timezone(&Utc) >= Duration::minutes(CHECK_EVERY_MINUTES))
            .unwrap_or(true),
        None => true,
    })
}

/// The IPO messages an earnings poll at `now` queues, on trading days only:
/// `ipo:check` once an hour, and one `ipo:sweep` while a tracked IPO is not
/// swept in this slot.
/// Off (`raw` not exactly `true`) is nothing, without touching the store.
pub async fn follow_ups(
    raw: Option<&str>,
    es: &dyn EarningsStore,
    now: DateTime<Utc>,
) -> Result<Vec<EarningsStep>, String> {
    follow_ups_for(raw, tracked(), es, now).await
}

pub async fn follow_ups_for(
    raw: Option<&str>,
    list: &[Tracked],
    es: &dyn EarningsStore,
    now: DateTime<Utc>,
) -> Result<Vec<EarningsStep>, String> {
    let trading = trading_day(et(now).date_naive()) != DayKind::Closed;
    if !enabled(raw) || list.is_empty() || !crate::earnings::in_window(now) || !trading {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    if check_due(es, now).await? {
        out.push(EarningsStep::IpoCheck);
    }
    if let Some(s) = slot(now) {
        if due_sweep(list, &read_sweeps(es).await?, &s).is_some() {
            out.push(EarningsStep::IpoSweep);
        }
    }
    Ok(out)
}

/// The error a job returns before migration 0009 is applied.
pub fn missing_table(err: &str) -> bool {
    err.contains("no such table: ipo_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::store::MemEarningsStore;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn the_list_parses_with_anthropic() {
        let list = parse_list(DATA).unwrap();
        assert_eq!(tracked(), list.as_slice());
        assert!(!list.is_empty() && list.len() <= MAX_TRACKED);
        let a = &list[0];
        assert_eq!(
            (
                a.slug.as_str(),
                a.cik,
                a.name.as_str(),
                a.name_zh.as_deref()
            ),
            ("anthropic", 2130357, "Anthropic", None)
        );
        assert!(a.aliases.is_empty());
    }

    #[test]
    fn bad_list_rows_are_errors() {
        for bad in [
            "Anthropic\t2130357\tAnthropic\t-",
            "anthropic\tx\tAnthropic\t-",
            "anthropic\t2130357",
            "anthropic\t0\tAnthropic\t-",
            "a\t1\tA\t-\nb\t1\tB\t-",
            "a\t1\tA\t-\na\t2\tB\t-",
        ] {
            assert!(parse_list(bad).is_err(), "{bad}");
        }
        let eleven: String = (1..=11).map(|i| format!("c{i}\t{i}\tC\t-\n")).collect();
        assert!(parse_list(&eleven).is_err());
        let ok =
            parse_list("# x\n\nnw\t9\tNorthwind\t北风\tNWR, Northwind Robotics\tnote\n").unwrap();
        assert_eq!(ok[0].name_zh.as_deref(), Some("北风"));
        assert_eq!(
            ok[0].names().collect::<Vec<_>>(),
            ["Northwind", "NWR", "Northwind Robotics"]
        );
    }

    #[test]
    fn the_switch_is_exactly_true() {
        assert!(enabled(Some("true")));
        for off in [
            None,
            Some(""),
            Some(" true"),
            Some("true "),
            Some("TRUE"),
            Some("True"),
            Some("1"),
            Some("false"),
        ] {
            assert!(!enabled(off), "{off:?}");
        }
    }

    #[test]
    fn the_generator_config_turns_it_on_exactly() {
        const WRANGLER: &str = include_str!("../../../generator/wrangler.toml");
        let line = WRANGLER
            .lines()
            .find(|l| l.starts_with("IPO_WATCH_ENABLED"))
            .expect("IPO_WATCH_ENABLED in wrangler.toml");
        assert_eq!(line, "IPO_WATCH_ENABLED = \"true\"");
    }

    #[test]
    fn forms_and_their_kinds() {
        for f in ["S-1", "S-1/A", "F-1", "F-1/A"] {
            assert!(is_registration(f) && is_kept(f));
        }
        assert!(is_final("424B4") && is_final("424B1") && !is_final("424B3"));
        assert!(!is_kept("8-K") && !is_kept("424B3") && is_kept("RW"));
        let mut seen = std::collections::HashSet::new();
        assert!(FORMS.iter().all(|(f, _, _)| seen.insert(*f)));
    }

    #[test]
    fn next_trading_day_skips_weekends_and_holidays() {
        let d = |s: &str| s.parse::<NaiveDate>().unwrap();
        assert_eq!(next_trading_day(d("2026-11-12")), d("2026-11-13"));
        assert_eq!(next_trading_day(d("2026-11-13")), d("2026-11-16"));
        // Thanksgiving; the early close after it trades.
        assert_eq!(next_trading_day(d("2026-11-25")), d("2026-11-27"));
    }

    #[test]
    fn one_sweep_slot_after_the_close_on_trading_days() {
        // Nothing in the morning or before 16:30 ET (EST); 16:30 pm.
        assert_eq!(slot(utc("2026-11-11T12:00:00Z")), None);
        assert_eq!(slot(utc("2026-11-11T21:29:00Z")), None);
        assert_eq!(
            slot(utc("2026-11-11T21:30:00Z")).as_deref(),
            Some("2026-11-11:pm")
        );
        // 21:00 ET the same day is still that day's pm.
        assert_eq!(
            slot(utc("2026-11-12T02:00:00Z")).as_deref(),
            Some("2026-11-11:pm")
        );
        // Thanksgiving: closed, no sweep.
        assert_eq!(slot(utc("2026-11-26T22:00:00Z")), None);
    }

    fn list() -> Vec<Tracked> {
        parse_list("anthropic\t2130357\tAnthropic\t-\nnorthwind\t9\tNorthwind\t-\n").unwrap()
    }

    #[tokio::test]
    async fn off_queues_nothing_and_reads_nothing() {
        let now = utc("2026-11-11T15:00:00Z");
        for raw in [None, Some(" true"), Some("TRUE"), Some("false"), Some("")] {
            let es = MemEarningsStore::default();
            assert_eq!(
                follow_ups_for(raw, &list(), &es, now).await,
                Ok(vec![]),
                "{raw:?}"
            );
            assert_eq!(es.calls.get(), 0, "{raw:?}");
        }
    }

    #[tokio::test]
    async fn on_queues_the_hourly_check_and_one_sweep_per_poll() {
        let es = MemEarningsStore::default();
        let on = Some("true");
        // 10:00 ET: the check, no sweep before 16:30.
        let morning = utc("2026-11-11T15:00:00Z");
        assert_eq!(
            follow_ups_for(on, &list(), &es, morning).await.unwrap(),
            [EarningsStep::IpoCheck]
        );
        es.kv_set(KV_CHECKED, "2026-11-11T15:00:00Z", "x")
            .await
            .unwrap();
        assert!(follow_ups_for(on, &list(), &es, morning)
            .await
            .unwrap()
            .is_empty());
        // 16:30 ET: an hour later, the check again and the day's sweep.
        let close = utc("2026-11-11T21:30:00Z");
        assert_eq!(
            follow_ups_for(on, &list(), &es, close).await.unwrap(),
            [EarningsStep::IpoCheck, EarningsStep::IpoSweep]
        );
        es.kv_set(KV_CHECKED, "2026-11-11T21:30:00Z", "x")
            .await
            .unwrap();
        let mut sw = Sweeps::new();
        sw.insert("anthropic".into(), "2026-11-11:pm".into());
        es.kv_set(KV_SWEEPS, &serde_json::to_string(&sw).unwrap(), "x")
            .await
            .unwrap();
        // Northwind is still due.
        assert_eq!(
            follow_ups_for(on, &list(), &es, close).await.unwrap(),
            [EarningsStep::IpoSweep]
        );
        sw.insert("northwind".into(), "2026-11-11:pm".into());
        es.kv_set(KV_SWEEPS, &serde_json::to_string(&sw).unwrap(), "x")
            .await
            .unwrap();
        assert!(follow_ups_for(on, &list(), &es, close)
            .await
            .unwrap()
            .is_empty());
        // Outside the poll window, and on a market holiday: nothing.
        for t in ["2026-11-11T09:00:00Z", "2026-11-26T15:00:00Z"] {
            let fresh = MemEarningsStore::default();
            assert!(follow_ups_for(on, &list(), &fresh, utc(t))
                .await
                .unwrap()
                .is_empty());
            assert_eq!(fresh.calls.get(), 0, "{t}");
        }
    }

    #[test]
    fn the_missing_table_error_is_recognised() {
        assert!(missing_table(
            "D1_ERROR: no such table: ipo_watch: SQLITE_ERROR"
        ));
        assert!(!missing_table("D1_ERROR: no such table: macro_events"));
    }
}
