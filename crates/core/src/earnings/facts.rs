//! The stored record an earnings health check is written from: quote-anchored
//! press-release figures, GAAP trend from XBRL, and the filing links.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::facts::{quote_numbers, Field};
use crate::numbers::{self, Lang};

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
    /// For a per-share figure read from a table row, the table's scale
    /// ("(In millions, except per share amounts)" -> 1e6): the row's
    /// whole-number cells ("Non-GAAP net income / earnings per share $3,471
    /// $2.09 ...") are amounts in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_scale: Option<f64>,
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
    /// For a note quoting a table row, the table's scale ("(In millions)"
    /// -> 1e6: "Traditional Electric Operating Companies $2,272 ..." is
    /// $2,272 million); 1 for prose, and in facts stored before it was kept.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f64,
}

fn one() -> f64 {
    1.0
}

fn is_one(v: &f64) -> bool {
    *v == 1.0
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
    if scale == 1.0 {
        out.extend(quote_numbers(quote));
        return;
    }
    // A table quote's bare numbers mean `scale` dollars; the unscaled
    // values would let a draft read "$27,466" as dollars. A percentage
    // ("Revenue $13,467 $14,766 10% 10%") or an amount with its own size
    // word is not in the table's scale.
    for n in numbers::extract(quote, Lang::En) {
        let v = if n.multiplier == 1.0 && !n.is_percent {
            n.value * scale
        } else {
            n.value
        };
        out.push(v);
        if !n.signed {
            out.push(-v);
        }
    }
}

/// The whole-number cells of a table row in `scale` ("$2,272", "52"), as
/// amounts; a cell with decimals under 1,000 ("$1.77") is a per-share or
/// ratio cell and not scaled.
fn push_cells(out: &mut Vec<f64>, quote: &str, scale: f64) {
    for n in numbers::extract(quote, Lang::En) {
        let amount =
            n.multiplier == 1.0 && !n.is_percent && (n.decimals == 0 || n.value.abs() >= 1000.0);
        if amount {
            out.push(n.value * scale);
            if !n.signed {
                out.push(-n.value * scale);
            }
        }
    }
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
            if let Some(scale) = fig.table_scale {
                push_cells(&mut out, &fig.quote, scale);
            }
        }
    }
    for notes in [&f.guidance, &f.highlights] {
        if let Field::Value(ns) = notes {
            for n in ns {
                push_quote(&mut out, &n.quote, 1.0);
                if n.scale != 1.0 {
                    push_cells(&mut out, &n.quote, n.scale);
                }
            }
        }
    }
    // The trend's values and the years its quarters fall in (the post's
    // charts show them).
    if let Field::Value(t) = &f.trend {
        for p in t.revenue.iter().chain(&t.eps_diluted).chain(&t.net_income) {
            out.push(p.value);
            out.push(f64::from(p.period_end.year()));
        }
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
            table_scale: None,
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
                scale: 1.0,
            }]),
            highlights: Field::Missing { reason: "none".into() },
            trend: Field::Value(Trend {
                revenue: vec![qp("2026-03-28", 100.4e9), qp("2026-06-27", 99.8e9)],
                eps_diluted: vec![qp("2026-03-28", 1.80), qp("2026-06-27", 1.71)],
                net_income: vec![],
            }),
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
            table_scale: None,
        });
        let n = numbers_in_facts(&f);
        assert_eq!(
            check("up from $94.93 billion a year ago", Lang::En, &n),
            Ok(())
        );
    }

    fn note(quote: &str, scale: f64) -> Note {
        Note {
            quote: quote.into(),
            summary_en: String::new(),
            scale,
        }
    }

    fn table_fig(value: f64, unit: Unit, amount: &str, scale: f64, quote: &str) -> Field<Figure> {
        Field::Value(Figure {
            value,
            unit,
            amount: amount.into(),
            scale,
            quote: quote.into(),
            table_scale: None,
        })
    }

    /// Each draft numbers check from the LLM eval (2026-11-06): what the
    /// models wrote from the bundle passes, a wrong digit, scale or rate
    /// does not.
    fn verdicts(f: &EarningsFacts, ok: &[(&str, Lang)], bad: &[(&str, Lang)]) {
        let n = numbers_in_facts(f);
        for (t, lang) in ok {
            assert_eq!(check(t, *lang, &n), Ok(()), "{t}");
        }
        for (t, lang) in bad {
            assert!(check(t, *lang, &n).is_err(), "{t}");
        }
    }

    #[test]
    fn rates_in_a_table_quote_are_not_in_its_scale() {
        // UBER and QCOM: the growth column of a revenue row in millions.
        let mut f = facts();
        f.metrics.revenue = table_fig(
            14.766e9,
            Unit::Usd,
            "$14,766",
            1e6,
            "Revenue $13,467 $14,766 10% 10%",
        );
        verdicts(
            &f,
            &[
                ("Revenue rose 10% to $14.77 billion.", Lang::En),
                ("营收增长10%。", Lang::Zh),
            ],
            &[
                ("Revenue rose 13% to $14.77 billion.", Lang::En),
                ("Revenue was $10 million higher.", Lang::En),
            ],
        );
        f.metrics.revenue = table_fig(
            10.322e9,
            Unit::Usd,
            "$10,322",
            1e6,
            "Revenues $10,322 $11,270 (8%) $10,442 $11,270 (7%)",
        );
        verdicts(
            &f,
            &[
                ("Revenue fell 8% to $10.32 billion.", Lang::En),
                ("营收同比下降8%。", Lang::Zh),
            ],
            &[
                ("Revenue fell 9%.", Lang::En),
                ("营收下降8000万美元。", Lang::Zh),
            ],
        );
    }

    #[test]
    fn a_note_quoting_a_table_row_is_in_the_table_scale() {
        // SO: the segment row a highlight quotes, from "(In Millions Except
        // Earnings Per Share)".
        let mut f = facts();
        f.highlights = Field::Value(vec![
            note(
                "Traditional Electric Operating Companies $2,272 $1,920 $4,654 $3,994",
                1e6,
            ),
            note("Southern Power 52 3 30 141", 1e6),
        ]);
        verdicts(
            &f,
            &[
                ("Traditional electric operating companies earned $2,272 million, up from $1,920 million.", Lang::En),
                ("They earned $2.27 billion; Southern Power $52 million against $3 million.", Lang::En),
                ("传统电力运营公司净利润22.72亿美元，上年同期19.20亿美元；南方电力5200万美元，上年同期300万美元。", Lang::Zh),
                ("南方电力0.52亿美元，前九个月46.54亿美元。", Lang::Zh),
            ],
            &[
                ("They earned $2,722 million.", Lang::En),
                ("They earned 2.272亿美元.", Lang::Zh),
                ("They earned 227.2亿美元.", Lang::Zh),
                ("Southern Power earned $52 billion.", Lang::En),
            ],
        );
        // A note stored before the scale was kept is read as written: no
        // scale is guessed.
        f.highlights = Field::Value(vec![note(
            "Traditional Electric Operating Companies $2,272 $1,920 $4,654 $3,994",
            1.0,
        )]);
        verdicts(
            &f,
            &[],
            &[
                ("They earned $2,272 million.", Lang::En),
                ("22.72亿美元", Lang::Zh),
            ],
        );
        // EMR: a segment EBITA row with margins; the margin is a rate.
        f.highlights = Field::Value(vec![note(
            "Control Systems & Software $297 23.8% $99 $16 $412 33.1%",
            1e6,
        )]);
        verdicts(
            &f,
            &[
                ("Segment EBITA was $412 million, a 33.1% margin.", Lang::En),
                ("分部EBITA为4.12亿美元，利润率33.1%。", Lang::Zh),
            ],
            &[
                ("Segment EBITA was 41.2亿美元.", Lang::Zh),
                ("a $33.1 million margin", Lang::En),
                ("Segment EBITA was $421 million.", Lang::En),
            ],
        );
        // Per-share cells of such a row stay per share.
        f.highlights = Field::Value(vec![note("Diluted EPS $1.77 $1.55", 1e6)]);
        verdicts(
            &f,
            &[("EPS was $1.77.", Lang::En)],
            &[("$1.77 million", Lang::En)],
        );
    }

    #[test]
    fn a_per_share_row_keeps_its_amount_cells_in_the_table_scale() {
        // AMD: non-GAAP net income and EPS alternate in one row.
        let mut f = facts();
        f.metrics.eps_non_gaap = Field::Value(Figure {
            value: 2.09,
            unit: Unit::UsdPerShare,
            amount: "$2.09".into(),
            scale: 1.0,
            quote:
                "Non-GAAP net income / earnings per share $3,471 $2.09 $2,760 $1.66 $1,965 $1.20"
                    .into(),
            table_scale: Some(1e6),
        });
        verdicts(
            &f,
            &[
                (
                    "Non-GAAP net income was $3,471 million, or $2.09 per share.",
                    Lang::En,
                ),
                ("非GAAP净利润34.7亿美元，每股2.09美元。", Lang::Zh),
                ("非GAAP净利润34.71亿美元。", Lang::Zh),
            ],
            &[
                ("$2.09 million", Lang::En),
                ("347.1亿美元", Lang::Zh),
                ("$3,417 million", Lang::En),
            ],
        );
    }

    #[test]
    fn a_range_writes_its_unit_once() {
        // COP: a production range and an LNG capacity, in Chinese units.
        let mut f = facts();
        f.guidance = Field::Value(vec![
            note("Fourth-quarter 2026 production is expected to be 2.32 to 2.36 million barrels of oil equivalent per day.", 1.0),
            note("Operating Cash Flow $4.3B - $4.4B", 1.0),
        ]);
        f.highlights = Field::Value(vec![note(
            "Expanded commercial LNG offtake portfolio to 13 MTPA.",
            1.0,
        )]);
        verdicts(
            &f,
            &[
                ("第四季度产量预计为每日232万至236万桶油当量。", Lang::Zh),
                ("第四季度产量指引为每日2.32至2.36百万桶油当量。", Lang::Zh),
                (
                    "Production of 2.32 to 2.36 million barrels a day.",
                    Lang::En,
                ),
                ("液化天然气承购组合扩大至每年1300万吨。", Lang::Zh),
                ("The LNG portfolio reached 13 MTPA.", Lang::En),
                ("Operating cash flow of $4.3 to $4.4 billion.", Lang::En),
            ],
            &[
                ("每日2320万桶", Lang::Zh),
                ("每日233万桶", Lang::Zh),
                ("每日2.32亿桶", Lang::Zh),
                ("每日2.32至2.36千万桶", Lang::Zh),
                ("每年130万吨", Lang::Zh),
                ("Operating cash flow of $4.3 to $4.5 billion.", Lang::En),
                ("Operating cash flow of $4.3 million.", Lang::En),
            ],
        );
    }

    #[test]
    fn trend_years_and_spelled_counts_are_in_the_facts() {
        let mut f = facts();
        f.highlights = Field::Value(vec![note(
            "Mobile lines increased by 1.8 million over the last twelve months.",
            1.0,
        )]);
        let n = numbers_in_facts(&f);
        assert_eq!(check("过去12个月增加180万", Lang::Zh, &n), Ok(()));
        assert!(check("过去13个月", Lang::Zh, &n).is_err());
        assert!(check("增加190万", Lang::Zh, &n).is_err());
        // The trend's quarters are years the post's charts show, so it may
        // name them; a year before the earliest is not in the facts.
        assert!(check("since 2023", Lang::En, &n).is_err());
        if let Field::Value(t) = &mut f.trend {
            t.revenue.insert(0, qp("2024-06-29", 85.8e9));
        }
        let n = numbers_in_facts(&f);
        assert_eq!(check("up from mid-2024", Lang::En, &n), Ok(()));
        assert!(check("since 2023", Lang::En, &n).is_err());
    }

    #[test]
    fn json_round_trip() {
        let f = facts();
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<EarningsFacts>(&s).unwrap(), f);
    }
}
