//! The stored record an earnings health check is written from: quote-anchored
//! press-release figures, GAAP trend from XBRL, and the filing links.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use std::sync::OnceLock;

use regex::Regex;

use crate::anchor::has_word;
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
    /// For such a row with a year header, its current-period cell as
    /// written (2272 for "$2,272 $1,920 ..."): the other cells are other
    /// periods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<f64>,
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

fn push_quote(a: &mut EarningsAllowed, quote: &str, scale: f64) {
    if scale == 1.0 {
        a.any.extend(quote_numbers(quote));
        return;
    }
    // A table quote's bare numbers mean `scale` dollars; the unscaled
    // values would let a draft read "$27,466" as dollars. A percentage
    // ("Revenue $13,467 $14,766 10% 10%") is a rate, cited only as one; an
    // amount with its own size word is not in the table's scale.
    for n in numbers::extract(quote, Lang::En) {
        let (out, v) = if n.is_percent {
            (&mut a.rates, n.value)
        } else if n.multiplier == 1.0 {
            (&mut a.any, n.value * scale)
        } else {
            (&mut a.any, n.value)
        };
        out.push(v);
        if !n.signed {
            out.push(-v);
        }
    }
}

/// What an earnings draft may cite, by how each value may be written
/// (`numbers::Allowed`).
#[derive(Default)]
pub struct EarningsAllowed {
    /// Figure values, the numbers in kept quotes (scaled for table quotes),
    /// trend values, and the filing and fiscal years with their neighbours.
    pub any: Vec<f64>,
    /// Sizes an industry unit implies ("13 MTPA" is 13 million tonnes), with
    /// the words a draft writes that unit in.
    pub quantities: Vec<(f64, &'static [&'static str])>,
    /// Spelled-out time counts ("eighteen months").
    pub counts: Vec<f64>,
    /// The years the trend's quarters fall in.
    pub years: Vec<f64>,
    /// Rates in table quotes.
    pub rates: Vec<f64>,
    /// Table-row cells at the table's scale (SO's segment row "$2,272" is
    /// $2,272 million): only in a sentence that names the row.
    pub rows: Vec<RowCell>,
}

/// A table-row cell at its table's scale and what a sentence citing it
/// must carry: the row's label words (in English, or for a Chinese draft
/// their `ZH` rendering), and for a cell of another period than the row's
/// current one, a period or comparison word.
pub struct RowCell {
    pub value: f64,
    label: Vec<String>,
    other_period: bool,
    /// A per-share row's amount measure ("Non-GAAP net income"), named in
    /// full; a note row's label must also have a word that is not
    /// `GENERIC`.
    measure: bool,
}

/// Label words that are not the label.
const STOP: &[&str] = &["and", "the", "for", "inc", "corp", "total", "from", "with"];

/// Words of a per-share row label, not of its amount.
const PER_SHARE: &[&str] = &[
    "per", "share", "shares", "eps", "diluted", "basic", "earnings", "common", "class",
];

/// Amount measures a per-share row may also carry ("Non-GAAP net income /
/// earnings per share $3,471 $2.09 ...").
const AMOUNTS: &[&str] = &[
    "income", "revenue", "revenues", "sales", "ebitda", "ebit", "profit", "loss",
];

/// Chinese renderings of label words; a word with none must stay English.
const ZH: &[(&str, &[&str])] = &[
    ("traditional", &["传统"]),
    ("electric", &["电力", "电气"]),
    ("operating", &["运营", "经营"]),
    ("companies", &["公司"]),
    ("company", &["公司"]),
    ("southern", &["南方"]),
    ("power", &["电力"]),
    ("gas", &["天然气", "燃气"]),
    ("distribution", &["配送", "分销", "配电"]),
    ("control", &["控制"]),
    ("systems", &["系统"]),
    ("system", &["系统"]),
    ("software", &["软件"]),
    ("test", &["测试"]),
    ("measurement", &["测量"]),
    ("sensors", &["传感器"]),
    ("final", &["终端", "最终"]),
    ("intelligent", &["智能"]),
    ("devices", &["设备", "器件"]),
    ("safety", &["安全"]),
    ("productivity", &["生产力", "效率"]),
    ("services", &["服务"]),
    ("health", &["健康", "医疗"]),
    ("care", &["护理", "医疗"]),
    ("benefits", &["福利"]),
    ("pharmacy", &["药房", "药店", "药品"]),
    ("consumer", &["消费者", "消费"]),
    ("wellness", &["健康"]),
    ("data", &["数据"]),
    ("center", &["中心"]),
    ("client", &["客户端", "个人电脑"]),
    ("gaming", &["游戏"]),
    ("embedded", &["嵌入式"]),
    ("non", &["非"]),
    ("net", &["净"]),
    ("income", &["利润", "收入", "收益"]),
    ("revenue", &["营收", "收入"]),
    ("revenues", &["营收", "收入"]),
    ("sales", &["销售"]),
    ("adjusted", &["调整后"]),
    ("segment", &["分部"]),
];

/// Phrases a sentence uses for another period than the current one; never
/// a bare "from" ("from retail customers") or "较" ("盈利能力较强").
const PERIOD_EN: &[&str] = &[
    "up from",
    "down from",
    "compared with",
    "compared to",
    "versus",
    "vs",
    "year earlier",
    "year ago",
    "prior year",
    "prior-year",
    "last year",
    "nine months",
    "year-to-date",
    "year to date",
];
const PERIOD_ZH: &[&str] = &[
    "较上年",
    "较去年",
    "较同期",
    "上年同期",
    "去年同期",
    "年同期",
    "前九个月",
    "前三季度",
    "年初至今",
];

/// Label words too common to name a row on their own: a label needs one
/// more ("Southern Power" has "southern"; "Net income" alone names no row).
const GENERIC: &[&str] = &[
    "companies",
    "company",
    "power",
    "electric",
    "net",
    "non",
    "income",
    "revenue",
    "revenues",
    "sales",
    "segment",
    "adjusted",
    "operating",
    "gaap",
];

fn label_words(label: &str) -> Vec<String> {
    label
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| w.len() >= 3)
        .map(str::to_ascii_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// `w` in `hay` with no ASCII letter or digit right before or after it, so
/// "gaap" is a word in "非gaap净利润" too: where its last such occurrence
/// starts.
fn last_ascii_word(hay: &str, w: &str) -> Option<usize> {
    hay.match_indices(w)
        .filter(|&(i, _)| {
            let before = hay[..i].chars().next_back();
            let after = hay[i + w.len()..].chars().next();
            !before.is_some_and(|c| c.is_ascii_alphanumeric())
                && !after.is_some_and(|c| c.is_ascii_alphanumeric())
        })
        .map(|(i, _)| i)
        .last()
}

/// Where row label `label` (its words, in English or their `ZH` rendering)
/// ends in `before`: the start of its last word there. `None` unless every
/// word is there and, unless the label is a `measure`, one is more than a
/// `GENERIC` word.
fn label_at(label: &[String], measure: bool, before: &str) -> Option<usize> {
    if label.is_empty() || !measure && label.iter().all(|w| GENERIC.contains(&w.as_str())) {
        return None;
    }
    let lower = before.to_ascii_lowercase();
    label
        .iter()
        .map(|w| {
            let zh = ZH
                .iter()
                .find(|(en, _)| en == w)
                .map_or(&[][..], |(_, zh)| *zh);
            let en = last_ascii_word(&lower, w);
            let zh = zh.iter().filter_map(|z| lower.rfind(z)).max();
            en.max(zh)
        })
        .try_fold(0, |end, at| Some(end.max(at?)))
}

impl RowCell {
    /// The sentence names this cell's row before the number (at byte `at`),
    /// with no other row named between them, and a cell of another period
    /// than the row's current one has a period phrase.
    fn holds(&self, sentence: &str, at: usize, labels: &[(&[String], bool)]) -> bool {
        let before = &sentence[..at];
        let Some(own) = label_at(&self.label, self.measure, before) else {
            return false;
        };
        let nearest = labels
            .iter()
            .filter(|(l, _)| *l != self.label.as_slice())
            .all(|(l, m)| label_at(l, *m, before).is_none_or(|other| other <= own));
        let lower = sentence.to_ascii_lowercase();
        let period = !self.other_period
            || PERIOD_EN
                .iter()
                .any(|w| last_ascii_word(&lower, w).is_some())
            || PERIOD_ZH.iter().any(|w| sentence.contains(w));
        nearest && period
    }
}

impl EarningsAllowed {
    /// `numbers::check_allowed` against these values.
    pub fn check(&self, text: &str, lang: Lang) -> Result<(), Vec<String>> {
        let mut labels: Vec<(&[String], bool)> = self
            .rows
            .iter()
            .map(|c| (c.label.as_slice(), c.measure))
            .collect();
        labels.dedup();
        let holds: Vec<_> = self
            .rows
            .iter()
            .map(|c| {
                let labels = &labels;
                move |s: &str, at: usize| c.holds(s, at, labels)
            })
            .collect();
        let in_context: Vec<(f64, numbers::Holds)> = self
            .rows
            .iter()
            .zip(&holds)
            .flat_map(|(c, h)| {
                [
                    (c.value, h as numbers::Holds),
                    (-c.value, h as numbers::Holds),
                ]
            })
            .collect();
        numbers::check_allowed(
            text,
            lang,
            &numbers::Allowed {
                any: &self.any,
                quantities: &self.quantities,
                counts: &self.counts,
                years: &self.years,
                rates: &self.rates,
                in_context: &in_context,
            },
        )
    }
}

/// The row label of a table quote: the text before its first number.
fn row_label(quote: &str) -> &str {
    let at = quote
        .find(|c: char| c.is_ascii_digit() || c == '$' || c == '(')
        .unwrap_or(quote.len());
    &quote[..at]
}

/// A note's table-row cells at its table's scale: whole-number amount
/// cells ("$2,272", "52", "$1,234.5"), not percentages; none for a
/// per-share row ("Diluted EPS $2 $1.66").
fn push_note_cells(out: &mut Vec<RowCell>, n: &Note) {
    let label_text = row_label(&n.quote);
    let label = label_words(label_text);
    let lower = label_text.to_ascii_lowercase();
    if ["eps", "per share", "diluted", "basic"]
        .iter()
        .any(|w| has_word(&lower, w))
    {
        return;
    }
    for c in numbers::extract(&n.quote, Lang::En) {
        let amount =
            c.multiplier == 1.0 && !c.is_percent && (c.decimals == 0 || c.value.abs() >= 1000.0);
        if amount {
            out.push(RowCell {
                value: c.value.abs() * n.scale,
                label: label.clone(),
                other_period: n.current.is_some_and(|cur| cur != c.value.abs()),
                measure: false,
            });
        }
    }
}

/// A per-share figure's row of mixed measures ("Non-GAAP net income /
/// earnings per share $3,471 $2.09 $2,760 $1.66 ..."): its whole-number
/// cells at the table's scale, labelled by the amount measure; the one
/// right before the figure's own cell is its period. No cell of a row of
/// per-share figures only ("Diluted EPS $2 $1.66"), and no cell with
/// decimals ("$12,345.67"), scales.
fn push_figure_cells(out: &mut Vec<RowCell>, fig: &Figure, scale: f64) {
    let label_text = row_label(&fig.quote);
    let label: Vec<String> = label_words(label_text)
        .into_iter()
        .filter(|w| !PER_SHARE.contains(&w.as_str()))
        .collect();
    if !label.iter().any(|w| AMOUNTS.contains(&w.as_str())) {
        return;
    }
    let cells = numbers::extract(&fig.quote, Lang::En);
    let own = fig.quote.find(&fig.amount).unwrap_or(0);
    let current = cells
        .iter()
        .take_while(|c| c.at < own)
        .last()
        .filter(|c| c.decimals == 0)
        .map(|c| c.value.abs());
    for c in &cells {
        if c.multiplier == 1.0 && !c.is_percent && c.decimals == 0 && c.at != own {
            out.push(RowCell {
                value: c.value.abs() * scale,
                label: label.clone(),
                other_period: current != Some(c.value.abs()),
                measure: true,
            });
        }
    }
}

/// Spelled-out time counts of eleven and up ("over the last eighteen
/// months"); smaller counts are written bare and pass anyway.
fn push_counts(out: &mut Vec<f64>, quote: &str) {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        Regex::new(r"(?i)\b(eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety|hundred)\s+(?:months?|years?|quarters?|weeks?|days?)\b")
            .expect("valid built-in regex")
    });
    const WORDS: [&str; 18] = [
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
        "thirty",
        "forty",
        "fifty",
        "sixty",
        "seventy",
        "eighty",
        "ninety",
        "hundred",
    ];
    const VALUES: [f64; 18] = [
        11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0,
        80.0, 90.0, 100.0,
    ];
    for c in r.captures_iter(quote) {
        let w = c[1].to_ascii_lowercase();
        if let Some(i) = WORDS.iter().position(|x| *x == w) {
            out.push(VALUES[i]);
        }
    }
}

/// Sizes an industry unit implies, with the words a draft writes that unit
/// in: "13 MTPA" is 13 million tonnes ("1300万吨"), "1,450 MMcfe" 1.45
/// billion cubic feet ("14.5亿立方英尺").
fn push_quantities(out: &mut Vec<(f64, &'static [&'static str])>, quote: &str) {
    static R: OnceLock<Regex> = OnceLock::new();
    const TONNES: &[&str] = &["tonnes", "tonne", "tons", "ton", "metric", "吨", "mtpa"];
    const BARRELS: &[&str] = &["barrels", "barrel", "boe", "bbl", "桶", "油当量"];
    const BTU: &[&str] = &["btu", "mmbtu", "英热"];
    const CUBIC_FEET: &[&str] = &["cubic", "cf", "mcf", "bcf", "立方英尺", "立方"];
    let r = R.get_or_init(|| {
        Regex::new(r"(?:^|[^\d,.])(\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)\s?(MTPA|MMBOE|MMboe|MMBbl|MMbbl|MMBtu|MMbtu|MMcfe?|Bcfe?|MBOE|Mboe|MBbl|Mbbl|Mcfe?)\b")
            .expect("valid built-in regex")
    });
    for c in r.captures_iter(quote) {
        let Ok(v) = c[1].replace(',', "").parse::<f64>() else {
            continue;
        };
        let unit = &c[2];
        let size = match unit {
            u if u.starts_with("MM") || u == "MTPA" => 1e6,
            u if u.starts_with('B') => 1e9,
            _ => 1e3,
        };
        let words = match unit.to_ascii_lowercase() {
            u if u == "mtpa" => TONNES,
            u if u.contains("btu") => BTU,
            u if u.contains("cf") => CUBIC_FEET,
            _ => BARRELS,
        };
        out.push((v * size, words));
    }
}

/// Every number an earnings post may cite, by how it may be written.
pub fn allowed(f: &EarningsFacts) -> EarningsAllowed {
    let mut a = EarningsAllowed::default();
    let year = f64::from(f.filing_date.year());
    a.any.extend([year - 1.0, year, year + 1.0]);
    let mut quotes: Vec<&str> = Vec::new();
    if let Field::Value(p) = &f.fiscal_period {
        let fy = f64::from(p.fiscal_year);
        a.any.extend([fy - 1.0, fy, fy + 1.0]);
        push_quote(&mut a, &p.quote, 1.0);
        quotes.push(&p.quote);
    }
    for m in Metric::ALL {
        if let Some(fig) = f.metrics.value(m) {
            a.any.push(fig.value);
            push_quote(&mut a, &fig.quote, fig.scale);
            if let Some(scale) = fig.table_scale {
                push_figure_cells(&mut a.rows, fig, scale);
            }
            quotes.push(&fig.quote);
        }
    }
    for notes in [&f.guidance, &f.highlights] {
        if let Field::Value(ns) = notes {
            for n in ns {
                push_quote(&mut a, &n.quote, 1.0);
                if n.scale != 1.0 {
                    push_note_cells(&mut a.rows, n);
                }
                quotes.push(&n.quote);
            }
        }
    }
    for q in quotes {
        push_counts(&mut a.counts, q);
        push_quantities(&mut a.quantities, q);
    }
    // The trend's values, and the years its quarters fall in (the post's
    // charts show them), as bare years only.
    if let Field::Value(t) = &f.trend {
        for p in t.revenue.iter().chain(&t.eps_diluted).chain(&t.net_income) {
            a.any.push(p.value);
            a.years.push(f64::from(p.period_end.year()));
        }
    }
    a
}

/// The values any number may match (`allowed`'s `any`): figure values,
/// numbers inside kept quotes (both signs when unsigned; scaled too for
/// table quotes), XBRL trend values, and the filing and fiscal years with
/// their neighbours.
pub fn numbers_in_facts(f: &EarningsFacts) -> Vec<f64> {
    allowed(f).any
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
                current: None,
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
            current: None,
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
        let a = allowed(f);
        for (t, lang) in ok {
            assert_eq!(a.check(t, *lang), Ok(()), "{t}");
        }
        for (t, lang) in bad {
            assert!(a.check(t, *lang).is_err(), "{t}");
        }
    }

    #[test]
    fn rates_in_a_table_quote_are_not_in_its_scale() {
        // UBER and QCOM: the growth column of a revenue row in millions.
        let mut f = facts();
        f.guidance = Field::Value(vec![]);
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
                ("a $10 charge", Lang::En),
                ("14,766%", Lang::En),
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

    fn row_note(quote: &str, scale: f64, current: Option<f64>) -> Note {
        Note {
            current,
            ..note(quote, scale)
        }
    }

    #[test]
    fn a_note_quoting_a_table_row_is_in_the_table_scale_where_it_names_the_row() {
        // SO: the segment rows a highlight quotes, from "(In Millions
        // Except Earnings Per Share)"; the year header makes $2,272 and 52
        // the current quarter.
        let mut f = facts();
        f.highlights = Field::Value(vec![
            row_note(
                "Traditional Electric Operating Companies $2,272 $1,920 $4,654 $3,994",
                1e6,
                Some(2272.0),
            ),
            row_note("Southern Power 52 3 30 141", 1e6, Some(52.0)),
        ]);
        verdicts(
            &f,
            &[
                ("Traditional electric operating companies earned $2,272 million, up from $1,920 million a year earlier.", Lang::En),
                ("Traditional Electric Operating Companies earned $2.27 billion.", Lang::En),
                ("Southern Power earned $52 million against $3 million a year earlier.", Lang::En),
                ("传统电力运营公司净利润22.72亿美元，上年同期19.20亿美元；Southern Power净利润为5200万美元，2025年同期为300万美元。", Lang::Zh),
                ("传统电力运营公司前九个月为46.54亿美元。", Lang::Zh),
                ("传统电力运营公司本季度净利润为22.72亿美元，上年同期为19.20亿美元；前九个月为46.54亿美元，上年同期为39.94亿美元。", Lang::Zh),
                ("| Traditional Electric Operating Companies | $2,272 million |", Lang::En),
            ],
            &[
                // The segment figure as the company's.
                ("Southern Company net income was $2,272 million.", Lang::En),
                ("南方公司净利润为22.72亿美元。", Lang::Zh),
                // Another segment's figure under the nearest label.
                ("南方公司旗下传统电力运营公司净利润为5200万美元。", Lang::Zh),
                ("Southern Company's traditional electric operating companies earned $52 million, while Southern Power grew.", Lang::En),
                // A prior-year or year-to-date cell as this quarter: a bare
                // "from" or "较" is no period phrase.
                ("Traditional electric operating companies earned $1,920 million this quarter.", Lang::En),
                ("传统电力运营公司本季度净利润为19.20亿美元。", Lang::Zh),
                ("Traditional electric operating companies earned $1,920 million from retail customers.", Lang::En),
                ("Income from traditional electric operating companies was $4,654 million.", Lang::En),
                ("传统电力运营公司净利润19.20亿美元，盈利能力较强。", Lang::Zh),
                // A wrong digit or scale.
                ("Traditional electric operating companies earned $2,722 million.", Lang::En),
                ("传统电力运营公司净利润2.272亿美元。", Lang::Zh),
                ("传统电力运营公司净利润227.2亿美元。", Lang::Zh),
                ("Southern Power earned $52 billion.", Lang::En),
            ],
        );
        // Generic words alone name no row.
        f.highlights = Field::Value(vec![row_note(
            "Net income $3,864 $3,216",
            1e6,
            Some(3864.0),
        )]);
        verdicts(
            &f,
            &[],
            &[
                ("Net income was $3,864 million.", Lang::En),
                ("净利润38.64亿美元", Lang::Zh),
            ],
        );
        // A note stored before the scale was kept, or a row the exhibit
        // gives no year line for (EMR's segment EBITA, AMGN's GAAP and
        // non-GAAP columns: extraction leaves them at scale 1), is read as
        // written: no scale is guessed.
        for quote in [
            "Traditional Electric Operating Companies $2,272 $1,920 $4,654 $3,994",
            "Control Systems & Software $297 23.8% $99 $16 $412 33.1%",
            "Net income $3,864 $3,520 $3,216 $3,055",
        ] {
            f.highlights = Field::Value(vec![note(quote, 1.0)]);
            verdicts(
                &f,
                &[],
                &[
                    ("Traditional electric operating companies earned $2,272 million.", Lang::En),
                    ("传统电力运营公司净利润22.72亿美元", Lang::Zh),
                    ("Control Systems & Software adjusted segment EBITA was $412 million.", Lang::En),
                    ("Control Systems & Software adjusted segment EBITA was $99 million.", Lang::En),
                    ("Control Systems & Software adjusted segment EBITA was $297 million.", Lang::En),
                    ("Control Systems & Software adjusted segment EBITA was $298 million this quarter.", Lang::En),
                    ("控制系统与软件分部调整后EBITA为9900万美元。", Lang::Zh),
                    ("Amgen net income was $3,216 million.", Lang::En),
                    ("GAAP net income was $3,520 million.", Lang::En),
                ],
            );
        }
        // Per-share rows never scale, integer cells included.
        f.highlights = Field::Value(vec![
            row_note("Diluted EPS $1.77 $1.55", 1e6, Some(1.77)),
            row_note("Diluted EPS $2 $1.66", 1e6, Some(2.0)),
        ]);
        verdicts(
            &f,
            &[("Diluted EPS was $1.77.", Lang::En)],
            &[
                ("Diluted EPS $1.77 million", Lang::En),
                ("Diluted EPS $2 million", Lang::En),
            ],
        );
    }

    #[test]
    fn a_per_share_row_keeps_its_amount_cells_in_the_table_scale() {
        // AMD: non-GAAP net income and EPS alternate in one row; $3,471 is
        // this quarter's, $2,760 the prior quarter's, $1,965 a year ago.
        let mut f = facts();
        let eps = |amount: &str, value: f64, quote: &str| {
            Field::Value(Figure {
                value,
                unit: Unit::UsdPerShare,
                amount: amount.into(),
                scale: 1.0,
                quote: quote.into(),
                table_scale: Some(1e6),
            })
        };
        f.metrics.eps_non_gaap = eps(
            "$2.09",
            2.09,
            "Non-GAAP net income / earnings per share $3,471 $2.09 $2,760 $1.66 $1,965 $1.20",
        );
        verdicts(
            &f,
            &[
                ("Non-GAAP net income was $3,471 million, or $2.09 per share.", Lang::En),
                ("非GAAP净利润34.7亿美元，每股2.09美元。", Lang::Zh),
                ("调整后（非GAAP）摊薄每股收益为2.09美元，对应非GAAP净利润34.71亿美元。", Lang::Zh),
                ("Non-GAAP net income was $3,471 million, up from $1,965 million a year earlier.", Lang::En),
            ],
            &[
                ("$2.09 million", Lang::En),
                ("非GAAP净利润347.1亿美元", Lang::Zh),
                ("Non-GAAP net income was $3,417 million.", Lang::En),
                ("Revenue was $3,471 million.", Lang::En),
                ("GAAP net income was $3,471 million.", Lang::En),
                ("Non-GAAP net income was $2,760 million this quarter.", Lang::En),
                ("Non-GAAP net income from operations was $2,760 million.", Lang::En),
                ("Non-GAAP net income rose from $1,965 million.", Lang::En),
            ],
        );
        // A row of per-share figures only, or a per-share figure of $1,000
        // and up, scales nothing.
        f.metrics.eps_non_gaap = eps("$2", 2.0, "Diluted EPS $2 $1.66");
        verdicts(&f, &[], &[("Diluted EPS $2 million", Lang::En)]);
        f.metrics.eps_non_gaap = eps(
            "$12,345.67",
            12345.67,
            "Net earnings per Class A share $12,345.67 $9,876.54",
        );
        verdicts(&f, &[], &[("Net earnings were $12.35 billion.", Lang::En)]);
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
                // A unit's size is no dollar amount or rate.
                ("a $13 million charge", Lang::En),
                ("13 million customers", Lang::En),
                ("1300万港元", Lang::Zh),
                ("1300万人民币", Lang::Zh),
                ("1300万美元", Lang::Zh),
            ],
        );
        // A comma-grouped quantity keeps its thousands.
        f.highlights = Field::Value(vec![note(
            "Production averaged 1,450 MMcfe per day; realized $2.85/Mcf and 3.5 Bcf/d.",
            1.0,
        )]);
        verdicts(
            &f,
            &[("产量为每日14.5亿立方英尺当量。", Lang::Zh)],
            &[
                ("每日4.5亿立方英尺", Lang::Zh),
                ("$3.5 billion", Lang::En),
                ("$1.45 billion", Lang::En),
            ],
        );
        // A change or two columns are no range.
        f.guidance = Field::Value(vec![
            note("Backlog grew 9 to $45 billion at quarter end.", 1.0),
            note("Adjusted EPS $1.62 ~$7.25 billion buyback", 1.0),
        ]);
        verdicts(
            &f,
            &[("Backlog reached $45 billion.", Lang::En)],
            &[
                ("Backlog grew by $9 billion.", Lang::En),
                ("a $1.62 billion buyback", Lang::En),
            ],
        );
    }

    #[test]
    fn trend_years_and_spelled_counts_are_in_the_facts() {
        let mut f = facts();
        f.guidance = Field::Value(vec![]);
        f.highlights = Field::Value(vec![note(
            "Mobile lines increased by 1.8 million over the last twelve months.",
            1.0,
        )]);
        verdicts(
            &f,
            &[
                ("过去12个月增加180万", Lang::Zh),
                ("over the last 12 months", Lang::En),
            ],
            &[
                ("过去13个月", Lang::Zh),
                ("增加190万", Lang::Zh),
                ("lines grew 12%", Lang::En),
                ("a $12 charge", Lang::En),
                ("opened 12 stores", Lang::En),
            ],
        );
        // Counts under eleven, and counts not of time, add nothing.
        f.highlights = Field::Value(vec![
            note("For the nine months ended September 30, revenue rose.", 1.0),
            note(
                "This is one of the largest one-time charges in three years.",
                1.0,
            ),
            note("one hundred basis points", 1.0),
        ]);
        verdicts(
            &f,
            &[],
            &[
                ("Revenue grew 9%.", Lang::En),
                ("shares fell 3%", Lang::En),
                ("EPS of $1.0", Lang::En),
                ("AIG rose 100%", Lang::En),
            ],
        );
        // The trend's quarters are years the post's charts show, so it may
        // name them, as years; a year before the earliest is not in the facts.
        let a = allowed(&f);
        assert!(a.check("since 2023", Lang::En).is_err());
        if let Field::Value(t) = &mut f.trend {
            t.revenue.insert(0, qp("2024-06-29", 85.8e9));
        }
        verdicts(
            &f,
            &[("up from mid-2024", Lang::En)],
            &[
                ("since 2023", Lang::En),
                ("a $2,024 charge", Lang::En),
                ("2,024 stores", Lang::En),
                ("2024%", Lang::En),
            ],
        );
    }

    #[test]
    fn json_round_trip() {
        let f = facts();
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<EarningsFacts>(&s).unwrap(), f);
    }
}
