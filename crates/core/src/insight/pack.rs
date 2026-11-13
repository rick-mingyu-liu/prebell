//! The evidence pack: what the writer and the judge may use for one mover,
//! and which movers have enough of it to get an insight.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{Evidence, EvidenceKind, Pack};
use crate::clock::{trading_day, DayKind};
use crate::cover::Session;
use crate::earnings::draft::fmt_value;
use crate::earnings::facts::{EarningsFacts, Metric};
use crate::facts::{Field, Mover, NewsReason, Theme};
use crate::ipo::news::{host_of, visible, OUTLETS};
use crate::Lang;

/// An earnings check belongs to a move when it was filed this many calendar
/// days before the trading date, or on it.
const CHECK_DAYS: i64 = 3;
/// Calendar rows this many days either side of the trading date.
const REPORT_DAYS: i64 = 100;
/// Display names for the hosts mover news most often comes from, beyond
/// `ipo::news::OUTLETS` (whose names win). Built from the hosts of the
/// stored mover reasons of 2026-11-04 to 2026-11-12, plus a few common ones.
pub const MOVER_OUTLETS: &[(&str, &str)] = &[
    ("seekingalpha.com", "Seeking Alpha"),
    ("benzinga.com", "Benzinga"),
    ("tipranks.com", "TipRanks"),
    ("tradingview.com", "TradingView"),
    ("stocktitan.net", "Stock Titan"),
    ("stockstory.org", "StockStory"),
    ("financefeeds.com", "FinanceFeeds"),
    ("quiverquant.com", "Quiver Quantitative"),
    ("247wallst.com", "24/7 Wall St."),
    ("tikr.com", "TIKR"),
    ("fool.com", "The Motley Fool"),
    ("tradingkey.com", "TradingKey"),
    ("chartmill.com", "ChartMill"),
    ("zacks.com", "Zacks"),
    ("nasdaq.com", "Nasdaq"),
    ("investing.com", "Investing.com"),
    ("thestreet.com", "TheStreet"),
    ("morningstar.com", "Morningstar"),
];

/// Search, aggregator and social hosts: not outlets. A reason from one
/// goes into the pack with no outlet, a line may not cite it, and it does
/// not make a mover eligible ("Google reports Axon lowered its guidance" and
/// "Stocktwits reports …" must not be possible).
pub const AGGREGATORS: &[&str] = &[
    "google.com",
    "news.google.com",
    "msn.com",
    "finance.yahoo.com",
    "yahoo.com",
    "bing.com",
    "stocktwits.com",
    "reddit.com",
    "x.com",
    "twitter.com",
    "facebook.com",
    "youtube.com",
];

/// An earnings calendar row's status (`earnings_calendar.status`; quarantined
/// rows are never read).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Reported,
    Confirmed,
    Unconfirmed,
}

impl ReportStatus {
    /// The D1 value; `None` for a quarantined row or anything unknown.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reported" => Some(ReportStatus::Reported),
            "confirmed" => Some(ReportStatus::Confirmed),
            "unconfirmed" => Some(ReportStatus::Unconfirmed),
            _ => None,
        }
    }
}

pub struct PackInputs<'a> {
    pub session: Session,
    pub trading_date: NaiveDate,
    /// When the post's facts were built: a premarket brief only uses an
    /// earnings check accepted before it.
    pub as_of_utc: DateTime<Utc>,
    /// Gainers then losers, as listed.
    pub movers: &'a [Mover],
    pub themes: &'a [Theme],
    /// PreBell's published earnings checks, any ticker.
    pub checks: &'a [EarningsFacts],
    /// (ticker, report date, status) rows near `trading_date`.
    pub calendar: &'a [(String, NaiveDate, ReportStatus)],
}

/// Packs for the eligible movers only, in mover order.
pub fn packs(i: &PackInputs) -> Vec<Pack> {
    i.movers.iter().filter_map(|m| pack(i, m)).collect()
}

fn long_date(d: NaiveDate) -> String {
    d.format("%B %-d, %Y").to_string()
}

/// The regular session's close on `d` (16:00 ET, 13:00 on an early close).
fn close_utc(d: NaiveDate) -> DateTime<Utc> {
    let (h, m) = match trading_day(d) {
        DayKind::EarlyClose => (13, 0),
        _ => (16, 0),
    };
    New_York
        .from_local_datetime(&d.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("valid time")))
        .earliest()
        .expect("the close is never in a DST gap")
        .with_timezone(&Utc)
}

/// Whether a guidance quote itself names the period it applies to: a
/// quarter, a half, a fiscal or calendar year, or the full year.
/// "Revenue $18.0 billion - $18.2 billion" does not, so a model could not
/// tell the quarter's range from the year's.
pub fn names_period(quote: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:Q[1-4]|[1-4]Q|H[12]|FY\s?\d{2,4}|fiscal\s+(?:year\s+)?(?:20\d{2}|\d{2})|(?:first|second|third|fourth)[- ](?:fiscal[- ])?quarter|(?:first|second)[- ]half|full[- ](?:fiscal[- ])?year|(?:for|in|during|through)\s+(?:calendar\s+|fiscal\s+)?(?:year\s+)?20\d{2})\b",
        )
        .expect("valid built-in regex")
    });
    re.is_match(&visible(quote))
}

/// "Q1 of fiscal 2027" / "fiscal 2027", from the check's verified period.
fn period_text(c: &EarningsFacts) -> Option<String> {
    let Field::Value(p) = &c.fiscal_period else {
        return None;
    };
    Some(match p.quarter {
        Some(q) => format!("Q{q} of fiscal {}", p.fiscal_year),
        None => format!("fiscal {}", p.fiscal_year),
    })
}

/// `text` without Markdown links, bare URLs or bold marks: "agreed to
/// [sell](https://seekingalpha.com/pr/…) six assets" reads "agreed to sell
/// six assets". A model is never sent a URL, and a URL's digits never
/// become allowed numbers.
pub fn plain(text: &str) -> String {
    static R: OnceLock<[Regex; 2]> = OnceLock::new();
    let [link, url] = R.get_or_init(|| {
        [
            Regex::new(r#"\[([^\]]*)\]\([^)\s]*(?:\s+"[^"]*")?\)"#).expect("valid built-in regex"),
            Regex::new(r"\(?https?://[^\s)]+\)?").expect("valid built-in regex"),
        ]
    });
    let text = link.replace_all(text, "$1");
    let text = url.replace_all(&text, "");
    let text = text.replace("**", "");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Adds an item to `ev` with the next id, without invisible characters. The
/// same sentence twice from one source (one quote holding two figures) is
/// one item; two outlets carrying it are two.
fn push<'a>(
    ev: &'a mut Vec<Evidence>,
    kind: EvidenceKind,
    text: &str,
    outlet: Option<String>,
    url: Option<String>,
    date: Option<NaiveDate>,
) -> Option<&'a mut Evidence> {
    let text = plain(&visible(text));
    let text = text.trim();
    if text.is_empty() || ev.iter().any(|e| e.text == text && e.outlet == outlet) {
        return None;
    }
    ev.push(Evidence {
        id: format!("e{}", ev.len() + 1),
        kind,
        text: text.to_string(),
        outlet,
        url,
        date,
        quote: None,
        unconfirmed: false,
    });
    ev.last_mut()
}

fn pack(i: &PackInputs, m: &Mover) -> Option<Pack> {
    let ticker = m.quote.symbol.clone();
    let td = i.trading_date;
    let mut ev: Vec<Evidence> = Vec::new();
    let verb = if m.quote.change_pct < 0.0 {
        "fell"
    } else {
        "rose"
    };
    let session = match i.session {
        Session::Premarket => "in premarket trading",
        Session::Recap => "in the regular session",
    };
    push(
        &mut ev,
        EvidenceKind::Move,
        &format!(
            "{} ({ticker}) {verb} {:.2}% {session} on {}.",
            m.name,
            m.quote.change_pct.abs(),
            long_date(td)
        ),
        None,
        None,
        Some(td),
    );

    for r in &m.reasons {
        let kind = if analyst_action(r, &m.name) {
            EvidenceKind::AnalystAction
        } else {
            EvidenceKind::News
        };
        push(
            &mut ev,
            kind,
            &r.quote,
            outlet_name(&r.url),
            Some(r.url.clone()),
            None,
        );
    }

    // Results out before the post was built: before the brief's facts, or
    // before the close the recap describes.
    let cutoff = match i.session {
        Session::Premarket => i.as_of_utc,
        Session::Recap => i.as_of_utc.min(close_utc(td)),
    };
    let check = i
        .checks
        .iter()
        .filter(|c| {
            c.ticker.eq_ignore_ascii_case(&ticker)
                && c.filing_date <= td
                && c.filing_date >= td - Duration::days(CHECK_DAYS)
                && c.accepted_at < cutoff
        })
        .max_by_key(|c| c.accepted_at);
    if let Some(c) = check {
        let url = match &c.exhibit_url {
            Field::Value(u) => u.clone(),
            Field::Missing { .. } => c.filing_url.clone(),
        };
        let period = period_text(c)
            .unwrap_or_else(|| format!("the results filed on {}", long_date(c.filing_date)));
        // Written by code from the verified value and period: a table row's
        // quote ("Total revenue 18,517 14,883") holds other periods' cells.
        for metric in Metric::ALL {
            if let Some(f) = c.metrics.value(metric) {
                let text = format!(
                    "{}: {}, reported for {period}.",
                    metric.label_en(),
                    fmt_value(f.value, metric.unit(), Lang::En)
                );
                if let Some(e) = push(
                    &mut ev,
                    EvidenceKind::CompanyFigure,
                    &text,
                    None,
                    Some(url.clone()),
                    None,
                ) {
                    e.quote = Some(visible(&f.quote));
                }
            }
        }
        if let Field::Value(notes) = &c.guidance {
            for n in notes.iter().filter(|n| names_period(&n.quote)) {
                push(
                    &mut ev,
                    EvidenceKind::CompanyGuidance,
                    &n.quote,
                    None,
                    Some(url.clone()),
                    None,
                );
            }
        }
    }

    for t in i
        .themes
        .iter()
        .filter(|t| t.members.iter().any(|x| x.ticker == ticker))
    {
        let dir = match t.direction {
            crate::facts::Direction::Up => "up",
            crate::facts::Direction::Down => "down",
        };
        push(
            &mut ev,
            EvidenceKind::Theme,
            &format!(
                "{} moved with the {} group ({} members {dir}, median {:+.2}%).",
                m.name,
                t.group,
                t.members.len() + t.more,
                t.median_pct
            ),
            None,
            None,
            None,
        );
    }

    let mut reports: Vec<(NaiveDate, ReportStatus)> = i
        .calendar
        .iter()
        .filter(|(t, d, _)| {
            t.eq_ignore_ascii_case(&ticker) && (*d - td).num_days().abs() <= REPORT_DAYS
        })
        .map(|(_, d, s)| (*d, *s))
        .collect();
    reports.sort_by_key(|(d, _)| *d);
    reports.dedup_by_key(|(d, _)| *d);
    for (d, status) in reports {
        let date = long_date(d);
        let text = match status {
            ReportStatus::Reported => {
                format!("{} reported results on {date} (earnings calendar).", m.name)
            }
            ReportStatus::Confirmed => format!(
                "{}'s report date is confirmed for {date} (earnings calendar).",
                m.name
            ),
            ReportStatus::Unconfirmed => {
                format!("Expected report date, not confirmed by the company: {date}.")
            }
        };
        if let Some(e) = push(
            &mut ev,
            EvidenceKind::ReportDate,
            &text,
            None,
            None,
            Some(d),
        ) {
            e.unconfirmed = status == ReportStatus::Unconfirmed;
        }
    }

    // News from an aggregator (no outlet) does not count.
    let eligible = ev.iter().any(|e| match e.kind {
        EvidenceKind::News => e.outlet.is_some(),
        EvidenceKind::CompanyFigure | EvidenceKind::CompanyGuidance => true,
        _ => false,
    });
    eligible.then(|| Pack {
        ticker,
        name: m.name.clone(),
        evidence: ev,
    })
}

/// The outlet's display name: from `ipo::news::OUTLETS`, then
/// `MOVER_OUTLETS`, else the host without "www.". `None` for an aggregator
/// or search host (`AGGREGATORS`), which is not an outlet.
pub fn outlet_name(url: &str) -> Option<String> {
    let host = host_of(url.trim());
    if host.is_empty() || AGGREGATORS.contains(&host.as_str()) {
        return None;
    }
    Some(
        OUTLETS
            .iter()
            .chain(MOVER_OUTLETS)
            .find(|(h, _)| *h == host)
            .map_or(host.clone(), |(_, name)| name.to_string()),
    )
}

/// The host an outlet name stands for, when it is a listed or
/// host-named outlet ("Seeking Alpha" -> "seekingalpha.com").
pub fn outlet_host(outlet: &str) -> Option<String> {
    OUTLETS
        .iter()
        .chain(MOVER_OUTLETS)
        .find(|(_, name)| *name == outlet)
        .map(|(h, _)| h.to_string())
        .or_else(|| outlet.contains('.').then(|| outlet.to_ascii_lowercase()))
}

/// Brokers and research firms whose name makes a rating word or a target an
/// analyst action. Whole words, with their capitals.
const BROKERS: &[&str] = &[
    "Morgan Stanley",
    "Goldman Sachs",
    "Goldman",
    "JPMorgan",
    "J.P. Morgan",
    "Citi",
    "Citigroup",
    "Bank of America",
    "BofA",
    "Jefferies",
    "Wells Fargo",
    "Barclays",
    "UBS",
    "Deutsche Bank",
    "Bernstein",
    "Evercore",
    "Evercore ISI",
    "Piper Sandler",
    "KeyBanc",
    "Mizuho",
    "Needham",
    "Oppenheimer",
    "Raymond James",
    "RBC",
    "TD Cowen",
    "Cowen",
    "Wedbush",
    "BMO",
    "Truist",
    "Stifel",
    "Baird",
    "Cantor Fitzgerald",
    "Loop Capital",
    "Rosenblatt",
    "HSBC",
    "Macquarie",
    "Nomura",
    "Redburn",
    "Argus",
    "Morningstar",
    "Guggenheim",
    "Susquehanna",
    "Daiwa",
    "William Blair",
    "Melius",
    "D.A. Davidson",
    "Craig-Hallum",
    "Northland",
    "Roth",
    "Seaport",
    "Wolfe Research",
    "BTIG",
    "Telsey",
    "Bernstein SocGen",
    "Exane",
    "Jefferies Group",
    "Arete",
    "New Street",
    "MoffettNathanson",
    "Lake Street",
    "Canaccord",
    "Erste",
    "CFRA",
    "Zacks",
    "Barrington",
    "Ladenburg",
    "Leerink",
    "Leerink Partners",
    "Bank of Montreal",
];

/// Wording that is an analyst action on its own.
fn analyst_strong_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:price\s+target|price\s+objective|target\s+price|price\s+tgt|(?:initiat|start|resum|launch|assum)\w*\s+(?:its\s+|their\s+)?coverage|coverage\s+(?:with|at)\s+an?\s+[a-z-]+(?:\s+[a-z-]+)?\s+rating|to\s+(?:buy|sell|hold|neutral|overweight|underweight|equal[- ]weight|outperform|underperform|market\s+perform|sector\s+perform|sector\s+weight|peer\s+perform|accumulate|reduce|strong\s+buy)\s+from|(?:overweight|underweight|equal[- ]weight|market[- ]perform|sector[- ]perform|sector[- ]weight|peer[- ]perform)|(?:buy|sell|hold|neutral|outperform|underperform|overweight|underweight)[- ]rat(?:ed|ing)|(?:upgrad|downgrad)\w*\s+(?:the\s+)?(?:stock|shares|company)\s+to)\b",
        )
        .expect("valid built-in regex")
    })
}

/// Wording that is an analyst action when a broker or an analyst is named.
fn analyst_weak_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:upgrad\w*|downgrad\w*|outperform|underperform|targets?|upside|downside|ratings?|reiterat\w*|rated)\b",
        )
        .expect("valid built-in regex")
    })
}

fn analyst_subject_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)\b(?:analysts?|brokerages?|brokers?|research\s+firm)\b")
            .expect("valid built-in regex")
    })
}

/// Whether `text` names a broker (as a whole word) or an analyst. A broker
/// whose name is in `mover` is the company itself, not its analysts.
fn names_broker(text: &str, mover: &str) -> bool {
    let joined = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    analyst_subject_re().is_match(text)
        || BROKERS.iter().filter(|b| !mover.contains(**b)).any(|b| {
            text.match_indices(b).any(|(at, m)| {
                !joined(text[..at].chars().next_back())
                    && !joined(text[at + m.len()..].chars().next())
            })
        })
}

/// True when a news reason is an analyst rating action: an upgrade, a
/// downgrade, an initiation, a rating or a price target. Rating wording
/// that is unambiguous counts alone ("price target", "to buy from hold",
/// "sector weight"); words with other meanings ("upgrade", "outperformed",
/// "target", "upside") count only beside a broker or an analyst. The quote
/// (and its summary) decide, not the headline: "Arista Gains as Strong Q3
/// Results and Fresh Analyst Target Hikes" can carry the company's own
/// outlook as its quote.
pub fn is_analyst_action(r: &NewsReason) -> bool {
    analyst_action(r, "")
}

/// `is_analyst_action` for a reason about the mover named `mover`.
fn analyst_action(r: &NewsReason, mover: &str) -> bool {
    [&r.quote, &r.summary_en].iter().any(|t| {
        let t = visible(t);
        analyst_strong_re().is_match(&t)
            || analyst_weak_re().is_match(&t) && names_broker(&t, mover)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::Session;
    use crate::earnings::facts::tests::{facts as check, fig};
    use crate::earnings::facts::{FiscalPeriod, Note, Unit};
    use crate::facts::{Direction, Mover, NewsReason, Quote, Theme, ThemeMember};
    use crate::insight::{Insight, Line};
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn reason(url: &str, title: &str, quote: &str) -> NewsReason {
        NewsReason {
            url: url.into(),
            title: title.into(),
            quote: quote.into(),
            summary_en: "s".into(),
        }
    }

    fn news() -> NewsReason {
        reason(
            "https://www.reuters.com/business/apple-results-2026-10-31/",
            "Apple shares rise after iPhone sales beat",
            "Apple said it expects revenue to grow 10 to 12 percent in the December quarter.",
        )
    }

    fn upgrade() -> NewsReason {
        reason(
            "https://www.cnbc.com/2026/10/31/apple-upgrade.html",
            "Morgan Stanley upgrades Apple to overweight",
            "Morgan Stanley raised its rating on Apple.",
        )
    }

    fn mover(sym: &str, pct: f64, reasons: Vec<NewsReason>) -> Mover {
        Mover {
            quote: Quote {
                symbol: sym.into(),
                last: 100.0,
                prev_close: 100.0 / (1.0 + pct / 100.0),
                change_pct: pct,
                volume: None,
            },
            name: format!("{sym} Inc"),
            reasons,
            search_failed: false,
            fallback: None,
        }
    }

    fn theme(member: &str) -> Theme {
        Theme {
            group: "networking & optical".into(),
            group_zh: "网络与光通信".into(),
            direction: Direction::Down,
            members: vec![ThemeMember {
                ticker: member.into(),
                name: format!("{member} Inc"),
                change_pct: -5.1,
            }],
            more: 2,
            median_pct: -4.63,
            reasons: vec![],
        }
    }

    fn note(quote: &str) -> Note {
        Note {
            quote: quote.into(),
            summary_en: "Written by a model; never read.".into(),
            cause: false,
            scale: 1.0,
            current: None,
        }
    }

    /// The fixture check (Apple, filed 2026-10-30 at 20:30 UTC, fiscal Q4
    /// 2026), with a guidance note that names its period beside the one
    /// that does not.
    fn apple_check() -> crate::earnings::facts::EarningsFacts {
        let mut c = check();
        if let Field::Value(notes) = &mut c.guidance {
            notes.push(note(
                "the Company expects revenue of $110 billion for the first quarter of fiscal 2027",
            ));
        }
        c
    }

    struct Fixture {
        movers: Vec<Mover>,
        themes: Vec<Theme>,
        checks: Vec<crate::earnings::facts::EarningsFacts>,
        calendar: Vec<(String, NaiveDate, ReportStatus)>,
        session: Session,
    }

    impl Fixture {
        fn new(movers: Vec<Mover>) -> Self {
            Fixture {
                movers,
                themes: vec![],
                checks: vec![],
                calendar: vec![],
                session: Session::Premarket,
            }
        }
        /// Packs as built at 12:00 UTC (premarket) or 21:30 UTC (recap).
        fn packs(&self, trading_date: &str) -> Vec<Pack> {
            let hour = match self.session {
                Session::Premarket => "12:00:00",
                Session::Recap => "21:30:00",
            };
            packs(&PackInputs {
                session: self.session,
                trading_date: d(trading_date),
                as_of_utc: format!("{trading_date}T{hour}Z").parse().unwrap(),
                movers: &self.movers,
                themes: &self.themes,
                checks: &self.checks,
                calendar: &self.calendar,
            })
        }
    }

    fn kinds(p: &Pack) -> Vec<EvidenceKind> {
        p.evidence.iter().map(|e| e.kind).collect()
    }

    #[test]
    fn eligible_with_a_news_reason() {
        let f = Fixture::new(vec![mover("AAPL", 4.2, vec![news()])]);
        let p = f.packs("2026-10-31");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].ticker, "AAPL");
        assert_eq!(p[0].name, "AAPL Inc");
        assert_eq!(kinds(&p[0]), [EvidenceKind::Move, EvidenceKind::News]);
        let n = &p[0].evidence[1];
        assert_eq!(n.outlet.as_deref(), Some("Reuters"));
        assert_eq!(n.text, news().quote);
        assert_eq!(n.url.as_deref(), Some(news().url.as_str()));
        assert_eq!(n.quote, None);
        let m = &p[0].evidence[0];
        assert!(m.text.contains("4.20%"), "{}", m.text);
        assert_eq!(m.date, Some(d("2026-10-31")));
    }

    #[test]
    fn not_eligible_with_only_a_theme_fallback() {
        let mut f = Fixture::new(vec![mover("ANET", -5.1, vec![])]);
        f.themes = vec![theme("ANET")];
        f.calendar = vec![("ANET".into(), d("2026-11-04"), ReportStatus::Unconfirmed)];
        assert!(f.packs("2026-10-31").is_empty());
    }

    #[test]
    fn not_eligible_with_only_an_analyst_action() {
        let f = Fixture::new(vec![mover("AAPL", 4.2, vec![upgrade()])]);
        assert!(f.packs("2026-10-31").is_empty());
    }

    #[test]
    fn a_mover_with_news_and_a_report_gets_its_check_figures_and_guidance() {
        let mut f = Fixture::new(vec![mover("AAPL", 4.2, vec![news(), upgrade()])]);
        // The fixture check was filed 2026-10-30, the day before.
        f.checks = vec![apple_check()];
        f.themes = vec![theme("AAPL")];
        f.calendar = vec![
            ("AAPL".into(), d("2026-10-30"), ReportStatus::Reported),
            ("MSFT".into(), d("2026-10-29"), ReportStatus::Reported),
        ];
        let p = f.packs("2026-10-31");
        assert_eq!(p.len(), 1);
        use EvidenceKind::*;
        assert_eq!(
            kinds(&p[0]),
            [
                Move,
                News,
                AnalystAction,
                CompanyFigure,
                CompanyFigure,
                CompanyFigure,
                CompanyGuidance,
                Theme,
                ReportDate
            ]
        );
        let texts: Vec<&str> = p[0].evidence.iter().map(|e| e.text.as_str()).collect();
        assert!(texts.contains(&"Revenue: $102.5 billion, reported for Q4 of fiscal 2026."));
        assert!(texts.contains(&"Diluted EPS (GAAP): $1.85, reported for Q4 of fiscal 2026."));
        assert!(texts.contains(&"Gross margin: 47.2%, reported for Q4 of fiscal 2026."));
        // The guidance note without a period ("grow 10 to 12 percent year
        // over year") is left out; the one naming its quarter is in.
        assert!(texts.contains(
            &"the Company expects revenue of $110 billion for the first quarter of fiscal 2027"
        ));
        assert!(!texts
            .iter()
            .any(|t| t.contains("10 to 12 percent year over year")));
        assert_eq!(p[0].evidence[2].outlet.as_deref(), Some("CNBC"));
        assert_eq!(p[0].evidence[3].outlet, None);
        assert_eq!(
            p[0].evidence[3].quote.as_deref(),
            Some("The Company posted quarterly revenue of $102.5 billion, up 8 percent year over year")
        );
        let report = &p[0].evidence[8];
        assert_eq!(report.date, Some(d("2026-10-30")));
        assert!(
            report.text.contains("reported results on October 30, 2026"),
            "{}",
            report.text
        );
    }

    /// Review H1, with Cisco's stored check (2026-11-12): a table row's
    /// quote holds the prior year's cell, and two guidance quotes read
    /// "Revenue $X billion - $Y billion" with no period.
    fn cisco_check() -> crate::earnings::facts::EarningsFacts {
        let mut c = check();
        c.ticker = "CSCO".into();
        c.company = "Cisco".into();
        c.filing_date = d("2026-11-12");
        c.accepted_at = "2026-11-12T21:06:00Z".parse().unwrap();
        c.fiscal_period = Field::Value(FiscalPeriod {
            quarter: Some(1),
            fiscal_year: 2027,
            quote: "Q1 FY 2027 Results:".into(),
        });
        let mut revenue = fig(18517e6, Unit::Usd, "18,517", "Total revenue 18,517 14,883");
        if let Field::Value(f) = &mut revenue {
            f.scale = 1e6;
        }
        c.metrics = crate::earnings::facts::Metrics::all_missing("x");
        c.metrics.revenue = revenue;
        c.guidance = Field::Value(vec![
            note("Revenue $18.0 billion - $18.2 billion"),
            note("Cisco estimates that GAAP EPS will be $1.07 to $1.09 for the second quarter of fiscal 2027."),
            note("Revenue $74.8 billion - $75.8 billion"),
            note("Cisco estimates that GAAP EPS will be $4.38 to $4.44 for fiscal 2027."),
        ]);
        c
    }

    #[test]
    fn company_figures_are_written_from_verified_fields() {
        let mut f = Fixture::new(vec![mover("CSCO", 6.0, vec![])]);
        f.checks = vec![cisco_check()];
        let p = f.packs("2026-11-13");
        let figure = &p[0].evidence[1];
        assert_eq!(figure.kind, EvidenceKind::CompanyFigure);
        assert_eq!(
            figure.text,
            "Revenue: $18.52 billion, reported for Q1 of fiscal 2027."
        );
        assert_eq!(figure.quote.as_deref(), Some("Total revenue 18,517 14,883"));
        // The prior-year column the reviewer cited is no longer allowed.
        let l = |watch: &str| Line {
            hook: "Revenue base sets the test".into(),
            watch: watch.into(),
            cites: vec![figure.id.clone()],
        };
        let zh = Line {
            hook: "营收基数成为考验".into(),
            watch: "公司公布2027财年第一季度营收185.2亿美元，后续财报将检验这一水平。".into(),
            cites: vec![figure.id.clone()],
        };
        let insight = |watch: &str| Insight {
            ticker: "CSCO".into(),
            en: l(watch),
            zh: zh.clone(),
        };
        let bad = crate::insight::check::check(
            &p[0],
            &insight("The company reported total revenue of 14,883, a level its next filing will update."),
        );
        assert!(bad.iter().any(|e| e.contains("14,883")), "{bad:?}");
        let ok = crate::insight::check::check(
            &p[0],
            &insight("The company reported Q1 revenue of $18.52 billion for fiscal 2027, a level to track."),
        );
        assert!(ok.is_empty(), "{ok:?}");
    }

    #[test]
    fn guidance_without_a_period_is_left_out() {
        let mut f = Fixture::new(vec![mover("CSCO", 6.0, vec![])]);
        f.checks = vec![cisco_check()];
        let p = f.packs("2026-11-13");
        let guidance: Vec<&str> = p[0]
            .evidence
            .iter()
            .filter(|e| e.kind == EvidenceKind::CompanyGuidance)
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(
            guidance,
            [
                "Cisco estimates that GAAP EPS will be $1.07 to $1.09 for the second quarter of fiscal 2027.",
                "Cisco estimates that GAAP EPS will be $4.38 to $4.44 for fiscal 2027."
            ]
        );
        for (quote, period) in [
            ("Revenue $18.0 billion - $18.2 billion", false),
            (
                "the Company expects total revenue to grow 10 to 12 percent year over year",
                false,
            ),
            ("revenue growth compared with 2019 levels", false),
            ("for the second quarter of fiscal 2027", true),
            ("for fiscal 2027", true),
            ("full-year revenue of $4 billion", true),
            ("Q4 revenue of $2 billion", true),
            ("expects 2027 revenue growth for 2027 of 5%", true),
            ("FY27 adjusted EPS of $3", true),
            ("second-half margin of 30%", true),
        ] {
            assert_eq!(names_period(quote), period, "{quote}");
        }
    }

    #[test]
    fn a_check_older_than_three_days_is_left_out() {
        let mut f = Fixture::new(vec![mover("AAPL", 4.2, vec![news()])]);
        f.checks = vec![check()];
        // Filed 2026-10-30: four days before 2026-11-03.
        let p = f.packs("2026-11-03");
        assert_eq!(kinds(&p[0]), [EvidenceKind::Move, EvidenceKind::News]);
        // Three days before is still in.
        let p = f.packs("2026-11-02");
        assert!(kinds(&p[0]).contains(&EvidenceKind::CompanyFigure));
        // A check filed after the trading date is not this move's report.
        let p = f.packs("2026-10-29");
        assert_eq!(kinds(&p[0]), [EvidenceKind::Move, EvidenceKind::News]);
    }

    #[test]
    fn results_after_the_close_are_not_in_that_days_recap() {
        // Accepted 2026-10-30 20:30 UTC, 4:30 PM ET: after that day's close.
        let mut f = Fixture::new(vec![mover("AAPL", 4.2, vec![news()])]);
        f.checks = vec![check()];
        f.session = Session::Recap;
        assert_eq!(
            kinds(&f.packs("2026-10-30")[0]),
            [EvidenceKind::Move, EvidenceKind::News]
        );
        assert!(kinds(&f.packs("2026-10-31")[0]).contains(&EvidenceKind::CompanyFigure));
        // Nor in a premarket brief built before it was accepted.
        f.session = Session::Premarket;
        assert_eq!(
            kinds(&f.packs("2026-10-30")[0]),
            [EvidenceKind::Move, EvidenceKind::News]
        );
        assert!(kinds(&f.packs("2026-10-31")[0]).contains(&EvidenceKind::CompanyFigure));
        // Cisco: 21:06 UTC on 2026-11-12 is after that day's close.
        let mut f = Fixture::new(vec![mover("CSCO", 6.0, vec![news()])]);
        f.checks = vec![cisco_check()];
        f.session = Session::Recap;
        assert_eq!(
            kinds(&f.packs("2026-11-12")[0]),
            [EvidenceKind::Move, EvidenceKind::News]
        );
    }

    #[test]
    fn report_dates_say_their_status() {
        let mut f = Fixture::new(vec![mover("AAPL", 4.2, vec![news()])]);
        f.calendar = vec![
            ("AAPL".into(), d("2026-08-01"), ReportStatus::Reported),
            ("AAPL".into(), d("2026-11-19"), ReportStatus::Confirmed),
            ("AAPL".into(), d("2027-01-29"), ReportStatus::Unconfirmed),
        ];
        let p = f.packs("2026-10-31");
        let reports: Vec<(&str, bool)> = p[0]
            .evidence
            .iter()
            .filter(|e| e.kind == EvidenceKind::ReportDate)
            .map(|e| (e.text.as_str(), e.unconfirmed))
            .collect();
        assert_eq!(
            reports,
            [
                ("AAPL Inc reported results on August 1, 2026 (earnings calendar).", false),
                ("AAPL Inc's report date is confirmed for November 19, 2026 (earnings calendar).", false),
                ("Expected report date, not confirmed by the company: January 29, 2027.", true),
            ]
        );
        assert_eq!(
            ReportStatus::parse("unconfirmed"),
            Some(ReportStatus::Unconfirmed)
        );
        assert_eq!(ReportStatus::parse("quarantined"), None);
    }

    #[test]
    fn a_report_alone_makes_a_mover_eligible() {
        let mut f = Fixture::new(vec![mover("AAPL", 4.2, vec![]), mover("MSFT", 3.0, vec![])]);
        f.checks = vec![check()];
        let p = f.packs("2026-10-31");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].ticker, "AAPL");
    }

    #[test]
    fn outlet_name_uses_the_list_then_the_host() {
        assert_eq!(outlet_name("reuters.com").as_deref(), Some("Reuters"));
        assert_eq!(
            outlet_name("https://www.reuters.com/markets/x").as_deref(),
            Some("Reuters")
        );
        assert_eq!(
            outlet_name("https://www.wsj.com/a").as_deref(),
            Some("The Wall Street Journal")
        );
        assert_eq!(
            outlet_name("https://www.marketscreener.com/x").as_deref(),
            Some("marketscreener.com")
        );
        assert_eq!(outlet_name(""), None);
    }

    #[test]
    fn analyst_actions_are_recognised() {
        let r = |title: &str, quote: &str| reason("https://x.test", title, quote);
        for (title, quote, want) in [
            // The review's cases (M3).
            (
                "Apple shares rise",
                "Apple shares outperformed the Nasdaq after the launch.",
                false,
            ),
            (
                "Apple unveils network upgrade",
                "Apple announced an upgrade to its data center network.",
                false,
            ),
            (
                "Apple rises",
                "Jefferies raised the stock to buy from hold.",
                true,
            ),
            (
                "Apple rises",
                "Goldman Sachs started coverage with a buy rating.",
                true,
            ),
            (
                "Apple rises",
                "Citi lifted its price objective to $300.",
                true,
            ),
            (
                "Apple rises",
                "KeyBanc moved the stock to sector weight.",
                true,
            ),
            ("Apple rises", "Analysts at Bernstein see 20% upside.", true),
            (
                "Apple rises",
                "Morgan Stanley raised its target on the shares to $250.",
                true,
            ),
            // Earlier cases.
            (
                "Morgan Stanley upgrades Apple to overweight",
                "Morgan Stanley raised its rating on Apple.",
                true,
            ),
            (
                "Shares fall",
                "Jefferies initiated coverage of the stock",
                true,
            ),
            ("Shares fall", "Citi cuts target on the company", true),
            (
                "Apple beats on earnings, raises dividend",
                "Revenue rose 8 percent.",
                false,
            ),
            (
                "Apple sets a 2030 emissions target",
                "The company raised its target for recycled materials.",
                false,
            ),
        ] {
            assert_eq!(is_analyst_action(&r(title, quote)), want, "{quote}");
        }
        assert!(!is_analyst_action(&news()));
        // A broker that is the mover itself is the company, not its analysts.
        let gs = reason(
            "https://x.test",
            "Goldman Sachs shares rise",
            "Goldman Sachs raised its return target to 16%.",
        );
        assert!(is_analyst_action(&gs));
        assert!(!analyst_action(&gs, "Goldman Sachs Group Inc"));
    }

    #[test]
    fn evidence_ids_are_unique_and_ordered() {
        let mut f = Fixture::new(vec![
            mover("AAPL", 4.2, vec![news(), upgrade()]),
            mover("NVDA", -3.0, vec![]),
            mover("MSFT", -2.0, vec![news()]),
        ]);
        f.checks = vec![check()];
        let p = f.packs("2026-10-31");
        assert_eq!(
            p.iter().map(|p| p.ticker.as_str()).collect::<Vec<_>>(),
            ["AAPL", "MSFT"]
        );
        for pack in &p {
            let ids: Vec<&str> = pack.evidence.iter().map(|e| e.id.as_str()).collect();
            let want: Vec<String> = (1..=ids.len()).map(|i| format!("e{i}")).collect();
            assert_eq!(ids, want);
        }
    }

    #[test]
    fn invisible_characters_are_stripped() {
        let f = Fixture::new(vec![mover(
            "AAPL",
            4.2,
            vec![reason(
                "https://www.reuters.com/x",
                "Apple lists",
                "on \u{200b}the Nasdaq",
            )],
        )]);
        let p = f.packs("2026-10-31");
        assert_eq!(p[0].evidence[1].text, "on the Nasdaq");
    }

    #[test]
    fn two_outlets_with_the_same_text_are_both_kept() {
        let same = "Apple said it expects revenue of $4.2 billion.";
        let f = Fixture::new(vec![mover(
            "AAPL",
            4.2,
            vec![
                reason("https://www.reuters.com/a", "Apple", same),
                reason("https://www.reuters.com/b", "Apple", same),
                reason("https://www.cnbc.com/c", "Apple", same),
            ],
        )]);
        let p = f.packs("2026-10-31");
        let outlets: Vec<Option<&str>> = p[0].evidence[1..]
            .iter()
            .map(|e| e.outlet.as_deref())
            .collect();
        assert_eq!(outlets, [Some("Reuters"), Some("CNBC")]);
    }

    /// Round 2, L4: real quotes carry Markdown links; the pack keeps the
    /// link text and never a URL.
    #[test]
    fn links_are_reduced_to_their_text() {
        assert_eq!(
            plain("after saying it agreed to [sell](https://seekingalpha.com/pr/20718237-nrg-energy-announces-4-billion-asset-sale \"sell\") six natural gas generation assets"),
            "after saying it agreed to sell six natural gas generation assets"
        );
        assert_eq!(
            plain("Fox Advisors downgraded [SNDK](https://financefeeds.com/sandisk-sndk-stock-1238/) from Overweight"),
            "Fox Advisors downgraded SNDK from Overweight"
        );
        assert_eq!(plain("see https://example.com/a1 now"), "see now");
        assert_eq!(
            plain("CEO **Strauss Zelnick** said"),
            "CEO Strauss Zelnick said"
        );
        let f = Fixture::new(vec![mover(
            "NRG",
            3.0,
            vec![reason(
                "https://seekingalpha.com/news/1",
                "NRG",
                "agreed to [sell](https://seekingalpha.com/pr/20718237-x) six assets for $4B",
            )],
        )]);
        let p = f.packs("2026-11-09");
        assert_eq!(p[0].evidence[1].text, "agreed to sell six assets for $4B");
        assert!(!serde_json::to_string(&p[0].evidence[1].text)
            .unwrap()
            .contains("http"));
    }

    /// Round 2, L3: the move names its day, so a line may name it too.
    #[test]
    fn the_move_names_its_trading_date() {
        let f = Fixture::new(vec![mover("AAPL", -4.2, vec![news()])]);
        let p = f.packs("2026-11-12");
        assert_eq!(
            p[0].evidence[0].text,
            "AAPL Inc (AAPL) fell 4.20% in premarket trading on November 12, 2026."
        );
        assert_eq!(p[0].evidence[0].date, Some(d("2026-11-12")));
    }

    /// Trial run 1: brands for the hosts in the stored reasons, and no
    /// outlet (and no eligibility) for an aggregator.
    #[test]
    fn mover_news_hosts_have_brands_and_aggregators_none() {
        for (host, name) in [
            ("seekingalpha.com", "Seeking Alpha"),
            ("benzinga.com", "Benzinga"),
            ("tipranks.com", "TipRanks"),
            ("tradingview.com", "TradingView"),
            ("stocktitan.net", "Stock Titan"),
            ("stockstory.org", "StockStory"),
            ("financefeeds.com", "FinanceFeeds"),
            ("investors.com", "Investor's Business Daily"),
            ("247wallst.com", "24/7 Wall St."),
            ("marketwatch.com", "MarketWatch"),
        ] {
            assert_eq!(
                outlet_name(&format!("https://www.{host}/a")).as_deref(),
                Some(name),
                "{host}"
            );
        }
        for host in [
            "google.com",
            "news.google.com",
            "msn.com",
            "finance.yahoo.com",
            "yahoo.com",
            "bing.com",
            // Social platforms, after trial run 2 (SMCI's only reason).
            "stocktwits.com",
            "reddit.com",
            "x.com",
            "twitter.com",
            "facebook.com",
            "youtube.com",
        ] {
            assert_eq!(outlet_name(&format!("https://{host}/a")), None, "{host}");
        }
        assert_eq!(
            outlet_host("Seeking Alpha").as_deref(),
            Some("seekingalpha.com")
        );
        assert_eq!(
            outlet_host("marketscreener.com").as_deref(),
            Some("marketscreener.com")
        );
        let f = Fixture::new(vec![mover(
            "AXON",
            -12.86,
            vec![
                reason(
                    "https://www.google.com/x",
                    "Axon",
                    "lowering its 2026 revenue guidance amid drone deployment delays",
                ),
                reason(
                    "https://finance.yahoo.com/y",
                    "Axon",
                    "reported third-quarter results",
                ),
            ],
        )]);
        assert!(f.packs("2026-11-06").is_empty());
        let smci = Fixture::new(vec![mover(
            "SMCI",
            -11.3,
            vec![reason(
                "https://stocktwits.com/news-articles/markets/equity/x",
                "Supermicro",
                "the DOJ is investigating whether entities linked to Alice Huang were involved in efforts to smuggle Supermicro products into China",
            )],
        )]);
        assert!(smci.packs("2026-11-12").is_empty());
    }
}
