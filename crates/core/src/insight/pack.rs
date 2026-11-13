//! The evidence pack: what the writer and the judge may use for one mover,
//! and which movers have enough of it to get an insight.

use std::sync::OnceLock;

use chrono::{Duration, NaiveDate};
use regex::Regex;

use super::{Evidence, EvidenceKind, Pack};
use crate::cover::Session;
use crate::earnings::facts::{EarningsFacts, Metric};
use crate::facts::{Field, Mover, NewsReason, Theme};
use crate::ipo::news::{host_of, visible, OUTLETS};

/// An earnings check belongs to a move when it was filed this many calendar
/// days before the trading date, or on it.
const CHECK_DAYS: i64 = 3;
/// Calendar rows this many days either side of the trading date.
const REPORT_DAYS: i64 = 100;
/// Macro releases this many days ahead.
const MACRO_DAYS: i64 = 7;

pub struct PackInputs<'a> {
    pub session: Session,
    pub trading_date: NaiveDate,
    /// Gainers then losers, as listed.
    pub movers: &'a [Mover],
    pub themes: &'a [Theme],
    /// PreBell's published earnings checks, any ticker.
    pub checks: &'a [EarningsFacts],
    /// (ticker, report date) rows near `trading_date`.
    pub calendar: &'a [(String, NaiveDate)],
    /// (date, event name), the next seven days.
    pub macro_events: &'a [(NaiveDate, String)],
}

/// Packs for the eligible movers only, in mover order.
pub fn packs(i: &PackInputs) -> Vec<Pack> {
    i.movers.iter().filter_map(|m| pack(i, m)).collect()
}

fn long_date(d: NaiveDate) -> String {
    d.format("%B %-d, %Y").to_string()
}

fn pack(i: &PackInputs, m: &Mover) -> Option<Pack> {
    let ticker = m.quote.symbol.clone();
    let td = i.trading_date;
    let mut ev: Vec<Evidence> = Vec::new();
    let mut push = |kind, text: &str, outlet: Option<String>, url: Option<String>, date| {
        let text = visible(text);
        let text = text.trim();
        // The same sentence twice (one quote holding two figures) is one item.
        if text.is_empty() || ev.iter().any(|e| e.text == text) {
            return;
        }
        ev.push(Evidence {
            id: format!("e{}", ev.len() + 1),
            kind,
            text: text.to_string(),
            outlet,
            url,
            date,
        });
    };

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
        EvidenceKind::Move,
        &format!(
            "{} ({ticker}) {verb} {:.2}% {session}.",
            m.name,
            m.quote.change_pct.abs()
        ),
        None,
        None,
        Some(td),
    );

    for r in &m.reasons {
        let kind = if is_analyst_action(r) {
            EvidenceKind::AnalystAction
        } else {
            EvidenceKind::News
        };
        push(
            kind,
            &r.quote,
            outlet_name(&r.url),
            Some(r.url.clone()),
            None,
        );
    }

    let check = i
        .checks
        .iter()
        .filter(|c| {
            c.ticker.eq_ignore_ascii_case(&ticker)
                && c.filing_date <= td
                && c.filing_date >= td - Duration::days(CHECK_DAYS)
        })
        .max_by_key(|c| c.accepted_at);
    if let Some(c) = check {
        let url = match &c.exhibit_url {
            Field::Value(u) => u.clone(),
            Field::Missing { .. } => c.filing_url.clone(),
        };
        for metric in Metric::ALL {
            if let Some(f) = c.metrics.value(metric) {
                push(
                    EvidenceKind::CompanyFigure,
                    &f.quote,
                    None,
                    Some(url.clone()),
                    None,
                );
            }
        }
        if let Field::Value(notes) = &c.guidance {
            for n in notes {
                push(
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

    let mut reports: Vec<NaiveDate> = i
        .calendar
        .iter()
        .filter(|(t, d)| {
            t.eq_ignore_ascii_case(&ticker) && (*d - td).num_days().abs() <= REPORT_DAYS
        })
        .map(|(_, d)| *d)
        .collect();
    reports.sort();
    reports.dedup();
    for d in reports {
        let text = if d < td {
            format!(
                "{} reported results on {} (earnings calendar).",
                m.name,
                long_date(d)
            )
        } else {
            format!(
                "{} is scheduled to report results on {} (earnings calendar).",
                m.name,
                long_date(d)
            )
        };
        push(EvidenceKind::ReportDate, &text, None, None, Some(d));
    }

    // A recap is written after the close: today's releases are behind it.
    let first = match i.session {
        Session::Premarket => td,
        Session::Recap => td + Duration::days(1),
    };
    for (d, name) in i
        .macro_events
        .iter()
        .filter(|(d, _)| *d >= first && *d <= td + Duration::days(MACRO_DAYS))
    {
        push(
            EvidenceKind::MacroEvent,
            &format!("{} on {}.", name.trim(), long_date(*d)),
            None,
            None,
            Some(*d),
        );
    }

    let eligible = ev.iter().any(|e| {
        matches!(
            e.kind,
            EvidenceKind::News | EvidenceKind::CompanyFigure | EvidenceKind::CompanyGuidance
        )
    });
    eligible.then(|| Pack {
        ticker,
        name: m.name.clone(),
        evidence: ev,
    })
}

/// The outlet's English name: from `ipo::news::OUTLETS` when the host is
/// listed, else the host without "www.".
pub fn outlet_name(url: &str) -> Option<String> {
    let host = host_of(url.trim());
    if host.is_empty() {
        return None;
    }
    Some(
        OUTLETS
            .iter()
            .find(|(h, _)| *h == host)
            .map_or(host.clone(), |(_, name)| name.to_string()),
    )
}

/// True when a news reason is an analyst rating action (upgrade, downgrade,
/// initiation, price target).
pub fn is_analyst_action(r: &NewsReason) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(
            r"(?i)upgrad|downgrad|initiat(?:es|ed|ing) coverage|price target|raises target|cuts target|overweight|underweight|outperform|underperform",
        )
        .expect("valid built-in regex")
    });
    [&r.title, &r.quote, &r.summary_en]
        .iter()
        .any(|t| re.is_match(&visible(t)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::Session;
    use crate::earnings::facts::tests::facts as check;
    use crate::facts::{Direction, Mover, NewsReason, Quote, Theme, ThemeMember};
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

    struct Fixture {
        movers: Vec<Mover>,
        themes: Vec<Theme>,
        checks: Vec<crate::earnings::facts::EarningsFacts>,
        calendar: Vec<(String, NaiveDate)>,
        macro_events: Vec<(NaiveDate, String)>,
    }

    impl Fixture {
        fn new(movers: Vec<Mover>) -> Self {
            Fixture {
                movers,
                themes: vec![],
                checks: vec![],
                calendar: vec![],
                macro_events: vec![],
            }
        }
        fn packs(&self, trading_date: &str) -> Vec<Pack> {
            packs(&PackInputs {
                session: Session::Premarket,
                trading_date: d(trading_date),
                movers: &self.movers,
                themes: &self.themes,
                checks: &self.checks,
                calendar: &self.calendar,
                macro_events: &self.macro_events,
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
        let m = &p[0].evidence[0];
        assert!(m.text.contains("4.20%"), "{}", m.text);
        assert_eq!(m.date, Some(d("2026-10-31")));
    }

    #[test]
    fn not_eligible_with_only_a_theme_fallback() {
        let mut f = Fixture::new(vec![mover("ANET", -5.1, vec![])]);
        f.themes = vec![theme("ANET")];
        f.calendar = vec![("ANET".into(), d("2026-11-04"))];
        f.macro_events = vec![(d("2026-11-03"), "October jobs report".into())];
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
        f.checks = vec![check()];
        f.themes = vec![theme("AAPL")];
        f.calendar = vec![
            ("AAPL".into(), d("2026-10-30")),
            ("MSFT".into(), d("2026-10-29")),
        ];
        f.macro_events = vec![(d("2026-11-06"), "October jobs report".into())];
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
                ReportDate,
                MacroEvent
            ]
        );
        let texts: Vec<&str> = p[0].evidence.iter().map(|e| e.text.as_str()).collect();
        assert!(texts.contains(
            &"The Company posted quarterly revenue of $102.5 billion, up 8 percent year over year"
        ));
        assert!(texts.contains(
            &"the Company expects total revenue to grow 10 to 12 percent year over year"
        ));
        assert_eq!(p[0].evidence[2].outlet.as_deref(), Some("CNBC"));
        assert_eq!(p[0].evidence[3].outlet, None);
        let report = &p[0].evidence[8];
        assert_eq!(report.date, Some(d("2026-10-30")));
        assert!(report.text.contains("October 30, 2026"), "{}", report.text);
        let event = &p[0].evidence[9];
        assert_eq!(event.date, Some(d("2026-11-06")));
        assert!(event.text.contains("October jobs report"));
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
        assert!(is_analyst_action(&upgrade()));
        assert!(is_analyst_action(&reason(
            "https://x.test",
            "Shares fall",
            "Jefferies initiated coverage of the stock",
        )));
        assert!(is_analyst_action(&reason(
            "https://x.test",
            "Shares fall",
            "Citi cuts target on the company",
        )));
        assert!(!is_analyst_action(&news()));
        assert!(!is_analyst_action(&reason(
            "https://x.test",
            "Apple beats on earnings, raises dividend",
            "Revenue rose 8 percent.",
        )));
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
}
