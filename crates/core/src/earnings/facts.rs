//! The stored record an earnings health check is written from: quote-anchored
//! press-release figures, GAAP trend from XBRL, and the filing links.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::facts::{quote_numbers, Field};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Usd,
    UsdPerShare,
    /// Percent units: 47.2 means 47.2%.
    Percent,
}

/// A press-release figure, verified against the exhibit text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Figure {
    /// Base units (dollars, dollars per share, or percent), signed.
    pub value: f64,
    pub unit: Unit,
    /// The amount exactly as written in the quote, e.g. "$102.5 billion" or "102,466".
    pub amount: String,
    /// Multiplier applied to a bare table amount ("(In millions)" -> 1e6); 1 otherwise.
    pub scale: f64,
    /// Verbatim exhibit text containing `amount`.
    pub quote: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Revenue,
    NetIncome,
    EpsGaap,
    EpsNonGaap,
    GrossMargin,
    OperatingMargin,
    OperatingCashFlow,
    FreeCashFlow,
}

impl Metric {
    pub const ALL: [Metric; 8] = [
        Metric::Revenue,
        Metric::NetIncome,
        Metric::EpsGaap,
        Metric::EpsNonGaap,
        Metric::GrossMargin,
        Metric::OperatingMargin,
        Metric::OperatingCashFlow,
        Metric::FreeCashFlow,
    ];

    /// The JSON key the extraction prompt uses.
    pub fn key(self) -> &'static str {
        match self {
            Metric::Revenue => "revenue",
            Metric::NetIncome => "net_income",
            Metric::EpsGaap => "eps_gaap_diluted",
            Metric::EpsNonGaap => "eps_non_gaap_diluted",
            Metric::GrossMargin => "gross_margin_pct",
            Metric::OperatingMargin => "operating_margin_pct",
            Metric::OperatingCashFlow => "operating_cash_flow",
            Metric::FreeCashFlow => "free_cash_flow",
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            Metric::EpsGaap | Metric::EpsNonGaap => Unit::UsdPerShare,
            Metric::GrossMargin | Metric::OperatingMargin => Unit::Percent,
            _ => Unit::Usd,
        }
    }

    pub fn label_en(self) -> &'static str {
        match self {
            Metric::Revenue => "Revenue",
            Metric::NetIncome => "Net income (GAAP)",
            Metric::EpsGaap => "Diluted EPS (GAAP)",
            Metric::EpsNonGaap => "Diluted EPS (non-GAAP)",
            Metric::GrossMargin => "Gross margin",
            Metric::OperatingMargin => "Operating margin (GAAP)",
            Metric::OperatingCashFlow => "Operating cash flow",
            Metric::FreeCashFlow => "Free cash flow (non-GAAP)",
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Metric::Revenue => "营收",
            Metric::NetIncome => "净利润（GAAP）",
            Metric::EpsGaap => "摊薄每股收益（GAAP）",
            Metric::EpsNonGaap => "摊薄每股收益（非GAAP）",
            Metric::GrossMargin => "毛利率",
            Metric::OperatingMargin => "营业利润率（GAAP）",
            Metric::OperatingCashFlow => "经营现金流",
            Metric::FreeCashFlow => "自由现金流（非GAAP）",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Metrics {
    pub revenue: Field<Figure>,
    pub net_income: Field<Figure>,
    pub eps_gaap: Field<Figure>,
    pub eps_non_gaap: Field<Figure>,
    pub gross_margin: Field<Figure>,
    pub operating_margin: Field<Figure>,
    pub operating_cash_flow: Field<Figure>,
    pub free_cash_flow: Field<Figure>,
}

impl Metrics {
    /// Every metric `Missing` with the same reason.
    pub fn all_missing(reason: &str) -> Self {
        let m = || Field::Missing {
            reason: reason.to_string(),
        };
        Metrics {
            revenue: m(),
            net_income: m(),
            eps_gaap: m(),
            eps_non_gaap: m(),
            gross_margin: m(),
            operating_margin: m(),
            operating_cash_flow: m(),
            free_cash_flow: m(),
        }
    }

    pub fn get(&self, m: Metric) -> &Field<Figure> {
        match m {
            Metric::Revenue => &self.revenue,
            Metric::NetIncome => &self.net_income,
            Metric::EpsGaap => &self.eps_gaap,
            Metric::EpsNonGaap => &self.eps_non_gaap,
            Metric::GrossMargin => &self.gross_margin,
            Metric::OperatingMargin => &self.operating_margin,
            Metric::OperatingCashFlow => &self.operating_cash_flow,
            Metric::FreeCashFlow => &self.free_cash_flow,
        }
    }

    pub fn set(&mut self, m: Metric, v: Field<Figure>) {
        let slot = match m {
            Metric::Revenue => &mut self.revenue,
            Metric::NetIncome => &mut self.net_income,
            Metric::EpsGaap => &mut self.eps_gaap,
            Metric::EpsNonGaap => &mut self.eps_non_gaap,
            Metric::GrossMargin => &mut self.gross_margin,
            Metric::OperatingMargin => &mut self.operating_margin,
            Metric::OperatingCashFlow => &mut self.operating_cash_flow,
            Metric::FreeCashFlow => &mut self.free_cash_flow,
        };
        *slot = v;
    }

    pub fn value(&self, m: Metric) -> Option<&Figure> {
        match self.get(m) {
            Field::Value(f) => Some(f),
            Field::Missing { .. } => None,
        }
    }
}

/// A quoted sentence (guidance, highlight) and a short English summary whose
/// numbers all come from the quote.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Note {
    pub quote: String,
    pub summary_en: String,
    /// A highlight that names what the release itself gives for a sharp
    /// swing in profit (an impairment, a charge, a gain on a deal). Absent
    /// from facts stored before it existed, and from every other note.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cause: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FiscalPeriod {
    /// 1-4, or `None` for a full-year-only release.
    pub quarter: Option<u8>,
    pub fiscal_year: i32,
    pub quote: String,
}

/// One discrete GAAP quarter from XBRL.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QuarterPoint {
    pub period_end: NaiveDate,
    pub value: f64,
    pub form: String,
    pub filed: NaiveDate,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Trend {
    pub revenue: Vec<QuarterPoint>,
    pub eps_diluted: Vec<QuarterPoint>,
    pub net_income: Vec<QuarterPoint>,
}

/// The trading session a share move was read in.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareSession {
    Premarket,
    Regular,
    AfterHours,
}

/// The stock's move when the check was built: premarket and regular trading
/// against the previous close, after hours against today's close.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShareMove {
    pub session: ShareSession,
    /// Percent units, one decimal (as printed), from the two prices: -4.2
    /// means -4.2%.
    pub change_pct: f64,
    pub last: f64,
    /// The close `change_pct` is measured from.
    pub ref_close: f64,
    /// The quote's own time (the session's last trade).
    pub as_of: DateTime<Utc>,
}

/// Why facts carry no share move when it was never read: stored before
/// the field existed, or not yet publishable.
const NOT_READ: &str = "not read";

fn share_move_not_read() -> Field<ShareMove> {
    Field::Missing {
        reason: NOT_READ.into(),
    }
}

/// Facts that never read the move store no key, so facts written before the
/// field existed serialize byte for byte as they did.
fn share_move_unread(f: &Field<ShareMove>) -> bool {
    matches!(f, Field::Missing { reason } if reason == NOT_READ)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EarningsFacts {
    pub ticker: String,
    pub company: String,
    pub cik: u64,
    pub accession: String,
    pub filing_date: NaiveDate,
    pub accepted_at: DateTime<Utc>,
    /// The EDGAR filing index page.
    pub filing_url: String,
    /// The Exhibit 99.1 press release.
    pub exhibit_url: Field<String>,
    pub fiscal_period: Field<FiscalPeriod>,
    pub metrics: Metrics,
    pub guidance: Field<Vec<Note>>,
    pub highlights: Field<Vec<Note>>,
    pub trend: Field<Trend>,
    /// Best effort: `Missing` whenever the quote is unavailable or stale.
    #[serde(
        default = "share_move_not_read",
        skip_serializing_if = "share_move_unread"
    )]
    pub share_move: Field<ShareMove>,
}

/// Why a figure is missing when the model found none in the release.
pub const NOT_STATED: &str = "not stated in the press release";

/// Names of required sections that are unavailable. Empty means publishable.
/// Revenue is required unless the release does not state it (insurers and
/// some energy companies lead with other totals); a revenue figure that is
/// there but fails verification still blocks, so the run retries.
pub fn missing_required(f: &EarningsFacts) -> Vec<&'static str> {
    let mut out = Vec::new();
    let not_stated =
        matches!(&f.metrics.revenue, Field::Missing { reason } if reason == NOT_STATED);
    if f.metrics.value(Metric::Revenue).is_none() && !not_stated {
        out.push("revenue");
    }
    if f.metrics.value(Metric::EpsGaap).is_none() && f.metrics.value(Metric::EpsNonGaap).is_none() {
        out.push("eps");
    }
    if !f.filing_url.starts_with("https://www.sec.gov/Archives/") {
        out.push("filing");
    }
    out
}

fn push_quote(out: &mut Vec<f64>, quote: &str, scale: f64) {
    // A table quote's bare numbers mean `scale` dollars; the unscaled
    // values would let a draft read "$27,466" as dollars.
    out.extend(quote_numbers(quote).into_iter().map(|v| v * scale));
}

/// Every number an earnings post may cite: figure values, numbers inside
/// kept quotes (both signs when unsigned; scaled too for table quotes), XBRL
/// trend values, and the filing and fiscal years with their neighbours.
pub fn numbers_in_facts(f: &EarningsFacts) -> Vec<f64> {
    let year = f64::from(f.filing_date.year());
    let mut out = vec![year - 1.0, year, year + 1.0];
    if let Field::Value(p) = &f.fiscal_period {
        let fy = f64::from(p.fiscal_year);
        out.extend([fy - 1.0, fy, fy + 1.0]);
        push_quote(&mut out, &p.quote, 1.0);
    }
    for m in Metric::ALL {
        if let Some(fig) = f.metrics.value(m) {
            out.push(fig.value);
            push_quote(&mut out, &fig.quote, fig.scale);
        }
    }
    for notes in [&f.guidance, &f.highlights] {
        if let Field::Value(ns) = notes {
            for n in ns {
                push_quote(&mut out, &n.quote, 1.0);
            }
        }
    }
    if let Field::Value(t) = &f.trend {
        for p in t.revenue.iter().chain(&t.eps_diluted).chain(&t.net_income) {
            out.push(p.value);
        }
    }
    if let Field::Value(s) = &f.share_move {
        out.extend([s.change_pct, s.last, s.ref_close]);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::numbers::{check, Lang};

    pub(crate) fn fig(value: f64, unit: Unit, amount: &str, quote: &str) -> Field<Figure> {
        Field::Value(Figure {
            value,
            unit,
            amount: amount.into(),
            scale: 1.0,
            quote: quote.into(),
        })
    }

    fn qp(end: &str, value: f64) -> QuarterPoint {
        QuarterPoint {
            period_end: end.parse().unwrap(),
            value,
            form: "10-Q".into(),
            filed: "2026-08-01".parse().unwrap(),
        }
    }

    /// A complete, publishable facts record (figures from the synthetic AAPL exhibit).
    pub(crate) fn facts() -> EarningsFacts {
        let mut metrics = Metrics::all_missing("not in the press release");
        metrics.revenue = fig(
            102.5e9,
            Unit::Usd,
            "$102.5 billion",
            "The Company posted quarterly revenue of $102.5 billion, up 8 percent year over year",
        );
        metrics.eps_gaap = fig(
            1.85,
            Unit::UsdPerShare,
            "$1.85",
            "quarterly diluted earnings per share of $1.85, up 91 percent year over year",
        );
        metrics.gross_margin = fig(
            47.2,
            Unit::Percent,
            "47.2 percent",
            "Gross margin percentage was 47.2 percent for the quarter.",
        );
        EarningsFacts {
            ticker: "AAPL".into(),
            company: "Apple".into(),
            cik: 320193,
            accession: "0000320193-26-000070".into(),
            filing_date: "2026-10-30".parse().unwrap(),
            accepted_at: "2026-10-30T20:30:41Z".parse().unwrap(),
            filing_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm".into(),
            exhibit_url: Field::Value("https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/a8-kex991q4202609262026.htm".into()),
            fiscal_period: Field::Value(FiscalPeriod {
                quarter: Some(4),
                fiscal_year: 2026,
                quote: "financial results for its fiscal 2026 fourth quarter ended September 26, 2026".into(),
            }),
            metrics,
            guidance: Field::Value(vec![Note {
                quote: "the Company expects total revenue to grow 10 to 12 percent year over year".into(),
                summary_en: "Apple expects revenue growth of 10 to 12 percent.".into(),
                cause: false,
            }]),
            highlights: Field::Missing { reason: "none".into() },
            trend: Field::Value(Trend {
                revenue: vec![qp("2026-03-28", 100.4e9), qp("2026-06-27", 99.8e9)],
                eps_diluted: vec![qp("2026-03-28", 1.80), qp("2026-06-27", 1.71)],
                net_income: vec![],
            }),
            share_move: share_move_not_read(),
        }
    }

    #[test]
    fn gate() {
        assert!(missing_required(&facts()).is_empty());
        let mut f = facts();
        f.metrics.eps_gaap = Field::Missing { reason: "x".into() };
        assert_eq!(missing_required(&f), vec!["eps"]);
        f.metrics.eps_non_gaap = fig(
            2.0,
            Unit::UsdPerShare,
            "$2.00",
            "non-GAAP diluted EPS of $2.00 for the quarter",
        );
        assert!(missing_required(&f).is_empty());
        f.metrics.revenue = Field::Missing { reason: "x".into() };
        f.filing_url = String::new();
        assert_eq!(missing_required(&f), vec!["revenue", "filing"]);
        // A release that states no revenue (AIG, ConocoPhillips) still publishes.
        f.metrics.revenue = Field::Missing {
            reason: NOT_STATED.into(),
        };
        assert_eq!(missing_required(&f), vec!["filing"]);
        // An LLM outage is not "not stated": revenue stays required, so the run retries.
        f.metrics.revenue = Field::Missing {
            reason: "extraction failed: http status 503".into(),
        };
        assert_eq!(missing_required(&f), vec!["revenue", "filing"]);
    }

    #[test]
    fn allow_list() {
        let n = numbers_in_facts(&facts());
        for ok in [
            "Revenue was $102.5 billion, up 8%.",
            "Diluted EPS of $1.85 rose 91%.",
            "Gross margin was 47.2%.",
            "Guidance calls for 10% to 12% growth in fiscal 2027.",
            "Revenue was $99.8 billion in the June quarter.",
            "营收1025亿美元，同比增长8%。",
        ] {
            let lang = if ok.is_ascii() { Lang::En } else { Lang::Zh };
            assert_eq!(check(ok, lang, &n), Ok(()), "{ok}");
        }
        assert!(check("Revenue grew 9%.", Lang::En, &n).is_err());
    }

    #[test]
    fn table_quotes_allow_scaled_numbers() {
        let mut f = facts();
        f.metrics.revenue = Field::Value(Figure {
            value: 102.466e9,
            unit: Unit::Usd,
            amount: "102,466".into(),
            scale: 1e6,
            quote: "Total net sales $102,466 $94,930".into(),
        });
        let n = numbers_in_facts(&f);
        assert_eq!(
            check("up from $94.93 billion a year ago", Lang::En, &n),
            Ok(())
        );
    }

    #[test]
    fn json_round_trip() {
        let mut f = facts();
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<EarningsFacts>(&s).unwrap(), f);
        f.share_move = Field::Value(ShareMove {
            session: ShareSession::AfterHours,
            change_pct: -6.0,
            last: 310.31,
            ref_close: 330.12,
            as_of: "2026-11-10T21:22:00Z".parse().unwrap(),
        });
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains(r#""session":"after_hours""#), "{s}");
        assert_eq!(serde_json::from_str::<EarningsFacts>(&s).unwrap(), f);
    }

    #[test]
    fn facts_stored_before_the_share_move_still_read() {
        let mut v = serde_json::to_value(facts()).unwrap();
        v.as_object_mut().unwrap().remove("share_move");
        let f: EarningsFacts = serde_json::from_value(v).unwrap();
        assert_eq!(f, facts());
        assert!(matches!(f.share_move, Field::Missing { .. }));
        // Written back, they are as they were.
        assert!(!serde_json::to_string(&f).unwrap().contains("share_move"));
        // A move that was read and is missing is stored with its reason.
        let mut f = facts();
        f.share_move = Field::Missing {
            reason: "market closed".into(),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(
            s.contains(r#""share_move":{"Missing":{"reason":"market closed"}}"#),
            "{s}"
        );
        assert_eq!(serde_json::from_str::<EarningsFacts>(&s).unwrap(), f);
    }

    #[test]
    fn the_share_move_is_citable() {
        let mut f = facts();
        f.share_move = Field::Value(ShareMove {
            session: ShareSession::Premarket,
            change_pct: -4.2,
            last: 325.72,
            ref_close: 340.0,
            as_of: "2026-11-10T13:12:00Z".parse().unwrap(),
        });
        let n = numbers_in_facts(&f);
        assert_eq!(
            check("Shares fell 4.2% to $325.72 from $340.", Lang::En, &n),
            Ok(())
        );
        assert_eq!(check("股价下跌4.2%。", Lang::Zh, &n), Ok(()));
        assert!(check("Shares fell 4.2%.", Lang::En, &numbers_in_facts(&facts())).is_err());
    }
}
