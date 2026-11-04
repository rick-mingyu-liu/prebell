//! Press-release figures, extracted by the LLM with verbatim quotes and kept
//! only when code can anchor each one in the fetched exhibit text.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::anchor::{has_word, verify_quote, verify_reason};
use crate::earnings::facts::{Figure, FiscalPeriod, Metric, Metrics, Note, Unit};
use crate::facts::{Field, NewsReason};
use crate::numbers::{extract as extract_numbers, Lang};
use crate::premarket::draft::reject_markup;
use crate::providers::LlmProvider;

const PROMPT: &str = include_str!("../../prompts/earnings_extract_v1.md");
/// Press releases with full statements run 20-60k characters.
const MAX_DOC_CHARS: usize = 60_000;
const MAX_NOTES: usize = 4;
/// Why a table amount from another period's column is rejected.
const CURRENT_COLUMN: &str = "a table amount must be in the current period's column: the row's first number, or the column the year header above gives the latest year";

/// What one press release yields.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub fiscal_period: Field<FiscalPeriod>,
    pub metrics: Metrics,
    pub guidance: Field<Vec<Note>>,
    pub highlights: Field<Vec<Note>>,
}

/// One figure as the LLM proposes it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RawFigure {
    pub amount: String,
    #[serde(default)]
    pub scale: String,
    #[serde(default)]
    pub negative: bool,
    pub quote: String,
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn scale_of(word: &str) -> Option<f64> {
    match word.trim().to_ascii_lowercase().as_str() {
        "" | "units" | "unit" | "ones" => Some(1.0),
        "thousands" => Some(1e3),
        "millions" => Some(1e6),
        "billions" => Some(1e9),
        _ => None,
    }
}

/// Where `amount` stands as a whole number in `quote`: not glued to another
/// digit, decimal point or thousands comma on either side, so "$1.8" never
/// matches inside "$1.85" nor "7,466" inside "27,466".
fn whole_match(quote: &str, amount: &str) -> Option<usize> {
    let numeric = |c: char| c.is_ascii_digit() || matches!(c, '.' | ',');
    quote.match_indices(amount).map(|(i, _)| i).find(|&i| {
        let before = quote[..i].chars().next_back();
        let mut after = quote[i + amount.len()..].chars();
        let next = after.next();
        let glued_after = match next {
            Some('.') | Some(',') => after.next().is_some_and(|c| c.is_ascii_digit()),
            Some(c) => c.is_ascii_digit(),
            None => false,
        };
        !before.is_some_and(numeric) && !glued_after
    })
}

/// "loss" just before the amount ("net loss of $25 million", "loss per
/// share $(0.25)"), ignoring "(loss)" in a row label like "Net income (loss)".
fn loss_before(quote: &str, at: usize) -> bool {
    let lead = quote[..at].to_ascii_lowercase().replace("(loss)", "");
    let tail: String = lead
        .chars()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    tail.contains("loss")
}

/// Words one of which the quote's label (the text before the amount) must
/// contain, as whole words, for the figure to be that metric.
pub fn label_keywords(m: Metric) -> &'static [&'static str] {
    match m {
        Metric::Revenue => &["revenue", "revenues", "net sales", "total revenues"],
        // A loss is reported on the net income line.
        Metric::NetIncome => &["net income", "net loss"],
        Metric::EpsGaap | Metric::EpsNonGaap => &["diluted", "per share", "eps"],
        Metric::GrossMargin => &["gross margin"],
        Metric::OperatingMargin => &["operating margin"],
        Metric::OperatingCashFlow => &["operating activities", "operating cash flow"],
        Metric::FreeCashFlow => &["free cash flow"],
    }
}

/// The quote names the metric: its label holds one of `label_keywords`
/// (per-share figures may say "per share" right after the amount), and a
/// per-share quote is never about a dividend.
fn names_metric(m: Metric, label: &str, after: &str) -> Result<(), String> {
    let lower = label.to_ascii_lowercase();
    if matches!(m, Metric::EpsGaap | Metric::EpsNonGaap)
        && (lower.contains("dividend") || after.to_ascii_lowercase().contains("dividend"))
    {
        return Err("a dividend per share is not earnings per share".into());
    }
    let near_after: String = after.chars().take(30).collect();
    if m == Metric::NetIncome
        && ["per share", "per diluted share", "per basic share"]
            .iter()
            .any(|w| has_word(label, w) || has_word(&near_after, w))
    {
        return Err("a per share figure is not net income".into());
    }
    let per_share_after = matches!(m, Metric::EpsGaap | Metric::EpsNonGaap)
        && (has_word(&near_after, "per share") || has_word(&near_after, "per diluted share"));
    let words = label_keywords(m);
    if per_share_after || words.iter().any(|w| has_word(label, w)) {
        return Ok(());
    }
    Err(format!(
        "the quote's label does not name {} (needs one of {words:?})",
        words[0]
    ))
}

/// A table row's revenue label is the company's total revenue, not a
/// segment or another total: "Total net sales", "Revenues", "Net
/// revenues", "Revenue, net", "Total revenues, net of interest expense",
/// "Total operating revenues", "Total revenues and other income" and
/// "Total net sales and revenue" pass; "Products net sales", "Services revenue", "Total cost of
/// revenues" and "Total noninterest revenue" do not.
fn total_revenue_row(label: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let label = label.trim().trim_end_matches(['$', ':', ' ']).trim();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:total\s+)?(?:net\s+)?(?:operating\s+)?(?:revenues?|sales)(?:\s+and\s+(?:other\s+income|revenues?))?(?:\s*,\s*net(?:\s+of\s+interest\s+expense)?)?$",
        )
        .expect("valid built-in regex")
    })
    .is_match(label)
}

/// Applies the quote-anchoring rules: the quote is in the exhibit; it starts
/// with a label naming the metric (letters before the amount); the amount is
/// a whole number in the quote, read with the unit words that follow it
/// there; a dollar figure written as a bare table number has the scale of
/// the table heading above it, is the current period's column (the row's
/// first number, unless the year header above puts the latest year in a
/// later column) and, for revenue, is the total row; per-share figures
/// carry no size word, are not dividends and, in a row of several periods,
/// follow the same column rule; a sign has evidence next to it.
pub fn verify_figure(text: &str, metric: Metric, raw: &RawFigure) -> Result<Figure, String> {
    let unit = metric.unit();
    let quote = collapse(&raw.quote);
    let amount = collapse(&raw.amount);
    if amount.is_empty() {
        return Err("empty amount".into());
    }
    if !verify_quote(text, &quote) {
        return Err("quote not found in the press release".into());
    }
    if extract_numbers(&amount, Lang::En).len() != 1 {
        return Err(format!("amount {amount:?} must hold exactly one number"));
    }
    let at = whole_match(&quote, &amount)
        .ok_or_else(|| format!("amount {amount:?} is not in its quote"))?;
    let label = &quote[..at];
    if !label.chars().any(char::is_alphabetic) {
        return Err("the quote must start with the row label, not mid-row".into());
    }
    names_metric(metric, label, &quote[at + amount.len()..])?;
    // Read the number in place, so a size word after it ("102.5 billion")
    // counts even when the amount omits it.
    let n = extract_numbers(&quote[at..], Lang::En)
        .into_iter()
        .next()
        .ok_or("no number at the amount")?;
    match unit {
        Unit::Percent if !n.is_percent => return Err("expected a percentage".into()),
        Unit::Usd | Unit::UsdPerShare if n.is_percent => {
            return Err("expected a dollar amount, got a percentage".into())
        }
        _ => {}
    }
    let scale = scale_of(&raw.scale).ok_or_else(|| format!("unknown scale {:?}", raw.scale))?;
    let scale = match unit {
        Unit::Usd if n.multiplier == 1.0 => {
            // A bare table number: only a scale the document itself states,
            // read from the current-period column.
            if scale == 1.0 {
                return Err(format!("bare amount {amount:?} needs the table's scale"));
            }
            let lines = Lines::new(text);
            let table = Table::find(&lines, &quote);
            let cells = row_cells(&quote);
            let col = cell_at(&cells, at, amount.len());
            if !col.is_some_and(|col| current_column(table.as_ref(), col, cells.len())) {
                return Err(CURRENT_COLUMN.into());
            }
            // The row's label, without the earlier columns.
            let row = &label[..label
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(label.len())];
            if metric == Metric::Revenue && !total_revenue_row(row) {
                return Err(format!(
                    "{:?} looks like a segment row; revenue must be the total",
                    row.trim()
                ));
            }
            match table.map(|t| t.scale) {
                Some(stated) if stated == scale => scale,
                Some(_) => {
                    return Err(format!("the table above the quote is not in {}", raw.scale))
                }
                None => {
                    return Err("no \"(In millions)\"-style table heading above the quote".into())
                }
            }
        }
        Unit::UsdPerShare => {
            let lower = quote.to_ascii_lowercase();
            if n.multiplier != 1.0 || n.value.abs() >= 1000.0 {
                return Err("a per-share amount has no size word and is under $1,000".into());
            }
            if !(lower.contains('$')
                || lower.contains("per share")
                || lower.contains("per diluted share")
                || lower.contains("eps"))
            {
                return Err("per-share quote names neither $ nor per share".into());
            }
            // Next to another per-share figure with no words between
            // ("Diluted EPS $7.08 $6.45 ..."), the amount is one column of a
            // row of periods and follows the bare-amount column rule.
            // Only per-share cells are its periods: AMD alternates net
            // income and per share ("$3,471 $2.09 $2,760 $1.66 ...").
            let cells: Vec<_> = row_cells(&quote)
                .into_iter()
                .filter(|&(s, e)| quote[s..e].contains('.'))
                .collect();
            let no_words = |a: usize, b: usize| !quote[a..b].chars().any(char::is_alphabetic);
            if let Some(col) = cell_at(&cells, at, amount.len()) {
                let in_row = match col {
                    0 => cells
                        .get(1)
                        .is_some_and(|&(s, _)| no_words(at + amount.len(), s)),
                    _ => no_words(cells[col - 1].1, at),
                };
                if in_row {
                    let lines = Lines::new(text);
                    let table = Table::find(&lines, &quote);
                    if !current_column(table.as_ref(), col, cells.len()) {
                        return Err(CURRENT_COLUMN.into());
                    }
                }
            }
            1.0
        }
        _ => 1.0,
    };

    let after = &quote[at + amount.len()..];
    let parenthesised = (amount.starts_with('(')
        || amount.starts_with("$(")
        || quote[..at].ends_with('(')
        || quote[..at].ends_with("$("))
        && (amount.ends_with(')') || after.starts_with(')'));
    let minus =
        n.signed && n.value < 0.0 || quote[..at].ends_with('-') || quote[..at].ends_with('−');
    let loss = loss_before(&quote, at);
    if loss && !raw.negative && !parenthesised && !minus {
        return Err("the quote calls this a loss but the figure is not negative".into());
    }
    let evidence = parenthesised || minus || loss;
    if raw.negative && !evidence {
        return Err(
            "negative figure without a minus sign, parentheses or \"loss\" before it".into(),
        );
    }
    let magnitude = n.value.abs() * scale;
    Ok(Figure {
        value: if evidence { -magnitude } else { magnitude },
        unit,
        amount,
        scale,
        quote,
    })
}

/// The nearest scale heading in `before`: where it starts and its scale.
/// A parenthesised heading counts by its lead only: "(In millions, except
/// shares, which are reflected in thousands)" is a millions table. So does
/// an unparenthesised one outside parentheses: "In millions, except per
/// share amounts", "Dollars in millions, except per share data".
fn scale_heading(before: &str) -> Option<(usize, f64)> {
    static PAREN: OnceLock<Regex> = OnceLock::new();
    static BARE: OnceLock<Regex> = OnceLock::new();
    let paren = PAREN.get_or_init(|| {
        Regex::new(r"(?i)\([^()]{0,30}?\bin\s+(thousands|millions|billions)\b")
            .expect("valid built-in regex")
    });
    let bare = BARE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:(?:dollars|amounts)(?:\s+and\s+shares)?\s+in\s+(thousands|millions|billions)\b|in\s+(thousands|millions|billions)\s*,?\s*except\b)",
        )
        .expect("valid built-in regex")
    });
    let inside_parens = |at: usize| before[..at].rfind('(') > before[..at].rfind(')');
    let paren = paren
        .captures_iter(before)
        .filter_map(|c| Some((c.get(0)?.start(), c.get(1)?.as_str())));
    let bare = bare
        .captures_iter(before)
        .filter_map(|c| Some((c.get(0)?.start(), c.get(1).or(c.get(2))?.as_str())))
        .filter(|&(at, _)| !inside_parens(at));
    let (at, unit) = paren.chain(bare).max_by_key(|&(at, _)| at)?;
    Some((at, scale_of(unit)?))
}

/// The exhibit text as `collapse` gives it, with where each line starts.
struct Lines {
    doc: String,
    starts: Vec<usize>,
}

impl Lines {
    fn new(text: &str) -> Self {
        let mut doc = String::new();
        let mut starts = Vec::new();
        for line in text.lines() {
            let line = collapse(line);
            if line.is_empty() {
                continue;
            }
            if !doc.is_empty() {
                doc.push(' ');
            }
            starts.push(doc.len());
            doc.push_str(&line);
        }
        Lines { doc, starts }
    }

    fn line_of(&self, at: usize) -> usize {
        self.starts.partition_point(|&s| s <= at).saturating_sub(1)
    }

    /// Line `i`, cut at `cut` when the cut falls inside it.
    fn line(&self, i: usize, cut: usize) -> &str {
        let end = self.starts.get(i + 1).map_or(self.doc.len(), |&e| e - 1);
        self.doc[self.starts[i]..end.min(cut)].trim()
    }
}

/// A quote's table: where the quote stands and the nearest scale heading
/// above it.
struct Table<'a> {
    lines: &'a Lines,
    quote_at: usize,
    heading_at: usize,
    scale: f64,
}

impl<'a> Table<'a> {
    fn find(lines: &'a Lines, quote: &str) -> Option<Self> {
        let quote_at = lines.doc.find(quote)?;
        let (heading_at, scale) = scale_heading(&lines.doc[..quote_at])?;
        Some(Table {
            lines,
            quote_at,
            heading_at,
            scale,
        })
    }

    /// The nearest year header line above the quote, within its table: the
    /// line index and its years. The search stops at the scale heading and
    /// at a line with no digits that is not a column label ("Supplemental
    /// table", "Revenues:"), and a line with other words is not a header.
    fn header(&self) -> Option<(usize, Vec<u32>)> {
        let l = self.lines;
        let (top, bottom) = (l.line_of(self.heading_at), l.line_of(self.quote_at));
        for i in (top..=bottom).rev() {
            let line = l.line(i, self.quote_at);
            if line.is_empty() {
                continue;
            }
            if let Some(years) = header_years(line, i == top) {
                return Some((i, years));
            }
            if !line.contains(|c: char| c.is_ascii_digit()) && !period_words_only(line) {
                return None;
            }
        }
        None
    }

    /// Whether the period labels of header line `i` (and of a label-only
    /// line right above it) name the quarter first: "Three Months Ended
    /// ... Nine Months Ended", "2025 Q4 2026 Q4". `None` when they name no
    /// period length.
    fn quarter_first(&self, i: usize) -> Option<bool> {
        static R: OnceLock<Regex> = OnceLock::new();
        let r = R.get_or_init(|| {
            Regex::new(r"(?i)\b(?:(three\s+months|thirteen\s+weeks|quarters?|q[1-4])|six\s+months|nine\s+months|twelve\s+months|years?|ytd|year-to-date)\b")
                .expect("valid built-in regex")
        });
        let l = self.lines;
        let above = i
            .checked_sub(1)
            .map(|j| l.line(j, self.quote_at))
            .filter(|s| period_words_only(s))
            .unwrap_or("");
        let labels = format!("{above} {}", l.line(i, self.quote_at));
        r.captures(&labels).map(|c| c.get(1).is_some())
    }
}

/// `s` without its parenthesised parts, nested ones included.
fn strip_parens(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| Regex::new(r"\([^()]*\)").expect("valid built-in regex"));
    let mut s = s.to_string();
    while r.is_match(&s) {
        s = r.replace_all(&s, " ").into_owned();
    }
    s
}

/// Every word outside parentheses names a period or a column: "Three
/// Months Ended September 30,", "GAAP Non-GAAP", "Change", "Inc/ (Dec)
/// Excluding Currency Translation", "(Unaudited)".
fn period_words_only(s: &str) -> bool {
    const WORDS: &[&str] = &[
        "jan",
        "january",
        "feb",
        "february",
        "mar",
        "march",
        "apr",
        "april",
        "may",
        "jun",
        "june",
        "jul",
        "july",
        "aug",
        "august",
        "sep",
        "sept",
        "september",
        "oct",
        "october",
        "nov",
        "november",
        "dec",
        "december",
        "three",
        "six",
        "nine",
        "twelve",
        "thirteen",
        "months",
        "month",
        "weeks",
        "ended",
        "ending",
        "quarter",
        "quarters",
        "year",
        "years",
        "fiscal",
        "full",
        "fy",
        "q",
        "ytd",
        "y",
        "change",
        "inc",
        "increase",
        "decrease",
        "excluding",
        "constant",
        "currency",
        "translation",
        "gaap",
        "non",
        "unaudited",
    ];
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| Regex::new(r"[A-Za-z]+").expect("valid built-in regex"));
    r.find_iter(&strip_parens(s))
        .all(|w| WORDS.contains(&w.as_str().to_ascii_lowercase().as_str()))
}

/// The years a column header line lists ("2025 Q4 2026 Q4 Change 2025
/// 2026", "September 26, 2026 September 27, 2025", "Q3'26 Q3'25"): at
/// least two, no dollar amounts (a data row; a "$ Change" label is fine)
/// and no words but period and column labels. On the scale heading's line only the part from the
/// first year counts ("In millions, except per share amounts 2026 2025").
fn header_years(line: &str, heading_line: bool) -> Option<Vec<u32>> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r =
        R.get_or_init(|| Regex::new(r"\b(?:19|20)\d\d\b|'(\d\d)\b").expect("valid built-in regex"));
    let dollar_amount = line.match_indices('$').any(|(i, _)| {
        line[i + 1..]
            .trim_start()
            .starts_with(|c: char| c.is_ascii_digit() || c == '(')
    });
    if dollar_amount {
        return None;
    }
    let line = strip_parens(line);
    let found: Vec<_> = r.captures_iter(&line).collect();
    if found.len() < 2 {
        return None;
    }
    let from = if heading_line {
        found[0].get(0)?.start()
    } else {
        0
    };
    if !period_words_only(&line[from..]) {
        return None;
    }
    found
        .iter()
        .map(|c| match c.get(1) {
            Some(yy) => yy.as_str().parse::<u32>().ok().map(|y| 2000 + y),
            None => c.get(0)?.as_str().parse().ok(),
        })
        .collect()
}

/// The row's value cells in `quote`, as byte spans of their digits. Not
/// cells: percentages ("10%", "(92) %"), digits glued to letters
/// ("Orders2", "Q4") and a footnote marker after the label words (a 1-2
/// digit integer in parentheses with no "$", not followed by another
/// parenthesised number: "Inc. (2) $6,626", but not "Net loss (62) (37)").
fn row_cells(quote: &str) -> Vec<(usize, usize)> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        Regex::new(r"\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?").expect("valid built-in regex")
    });
    let mut cells = Vec::new();
    for m in r.find_iter(quote) {
        let (before, after) = (&quote[..m.start()], &quote[m.end()..]);
        if before.ends_with(|c: char| c.is_alphabetic())
            || after.starts_with(|c: char| c.is_alphabetic())
            || after.trim_start_matches(')').trim_start().starts_with('%')
        {
            continue;
        }
        let footnote = cells.is_empty()
            && m.as_str().len() <= 2
            && before.ends_with('(')
            && !before.ends_with("$(")
            && after.starts_with(')')
            && !after[1..].trim_start().starts_with('(');
        if !footnote {
            cells.push((m.start(), m.end()));
        }
    }
    cells
}

/// The index of the cell the amount (at byte `at`, `len` long) holds.
fn cell_at(cells: &[(usize, usize)], at: usize, len: usize) -> Option<usize> {
    cells.iter().position(|&(s, _)| s >= at && s < at + len)
}

/// Whether cell `col` of a row of `cells` cells is the current period's
/// column. Without a year header: the first cell. A header is trusted when
/// its years form blocks that each hold every year once and there is one
/// year per cell; then the column is the latest year's place in the first
/// block, which must be the quarter when there are several. An untrusted
/// header allows only the first cell, when the header starts with the
/// latest year. A header whose first period is not the quarter rejects any
/// repeated-year row.
fn current_column(table: Option<&Table>, col: usize, cells: usize) -> bool {
    let Some((t, (i, years))) = table.and_then(|t| Some((t, t.header()?))) else {
        return col == 0;
    };
    let mut distinct = years.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let k = distinct.len();
    let latest = distinct[k - 1];
    let repeated = years.len() > k;
    let quarter_first = if repeated {
        t.quarter_first(i)
    } else {
        Some(true)
    };
    if quarter_first == Some(false) {
        return false;
    }
    let trusted = k >= 2
        && years.len() == cells
        && years.len() % k == 0
        && quarter_first == Some(true)
        && years.chunks(k).all(|c| {
            let mut c = c.to_vec();
            c.sort_unstable();
            c == distinct
        });
    if trusted {
        years[..k].iter().position(|&y| y == latest) == Some(col)
    } else {
        col == 0 && years[0] == latest
    }
}

fn quarter_words(q: u8) -> &'static [&'static str] {
    match q {
        1 => &["first quarter", "q1", "1q"],
        2 => &["second quarter", "q2", "2q"],
        3 => &["third quarter", "q3", "3q"],
        4 => &["fourth quarter", "q4", "4q"],
        _ => &[],
    }
}

/// The fiscal period's quote is in the exhibit and names the year (and the
/// quarter, when one is claimed).
fn verify_period(text: &str, v: &Value) -> Result<FiscalPeriod, String> {
    let quote = collapse(v.get("quote").and_then(Value::as_str).unwrap_or(""));
    let fy = v
        .get("fiscal_year")
        .and_then(Value::as_i64)
        .and_then(|y| i32::try_from(y).ok())
        .filter(|y| (1990..=2100).contains(y))
        .ok_or("missing fiscal_year")?;
    let quarter = match v.get("quarter") {
        None | Some(Value::Null) => None,
        Some(q) => Some(
            q.as_u64()
                .filter(|q| (1..=4).contains(q))
                .map(|q| q as u8)
                .ok_or("quarter must be 1-4")?,
        ),
    };
    if !verify_quote(text, &quote) {
        return Err("fiscal period quote not found".into());
    }
    let lower = quote.to_ascii_lowercase();
    let short = format!("{:02}", fy % 100);
    if !(lower.contains(&fy.to_string())
        || lower.contains(&format!("fy{short}"))
        || lower.contains(&format!("fy {short}")))
    {
        return Err("fiscal period quote does not name the year".into());
    }
    if let Some(q) = quarter {
        if !quarter_words(q).iter().any(|w| lower.contains(w)) {
            return Err("fiscal period quote does not name the quarter".into());
        }
    }
    Ok(FiscalPeriod {
        quarter,
        fiscal_year: fy,
        quote,
    })
}

/// Notes whose quote is in the exhibit and whose summary uses only the
/// quote's numbers and no markup.
fn verify_notes(text: &str, v: Option<&Value>) -> Vec<Note> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|it| serde_json::from_value::<Note>(it.clone()).ok())
        .map(|n| Note {
            quote: collapse(&n.quote),
            summary_en: collapse(&n.summary_en),
        })
        .filter(|n| !n.summary_en.is_empty())
        .filter(|n| reject_markup("summary_en", &n.summary_en).is_ok())
        .filter(|n| {
            let r = NewsReason {
                url: String::new(),
                title: String::new(),
                quote: n.quote.clone(),
                summary_en: n.summary_en.clone(),
            };
            verify_reason(text, &r)
        })
        .take(MAX_NOTES)
        .collect()
}

fn user_prompt(text: &str, company: &str, ticker: &str) -> String {
    let doc: String = text.chars().take(MAX_DOC_CHARS).collect();
    static CLOSE: OnceLock<Regex> = OnceLock::new();
    let close =
        CLOSE.get_or_init(|| Regex::new(r"(?i)<\s*/\s*document").expect("valid built-in regex"));
    let doc = close.replace_all(&doc, "< /document");
    format!("Company: {company} ({ticker})\n\n<document>\n{doc}\n</document>\n")
}

/// Figures shown (and charted) as GAAP.
fn is_gaap(m: Metric) -> bool {
    matches!(
        m,
        Metric::Revenue | Metric::NetIncome | Metric::EpsGaap | Metric::OperatingMargin
    )
}

/// Wording that marks a figure as non-GAAP: adjusted, managed (banks' FTE
/// revenue), "excluding" items.
fn non_gaap_wording(quote: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)\b(?:non-?\s?gaap|adjusted|managed|fte|fully\s+taxable|excluding)\b")
            .expect("valid built-in regex")
    })
    .is_match(quote)
}

/// Asks the LLM for the figures and keeps only what verifies. An LLM error
/// leaves every field `Missing`.
pub async fn extract(llm: &dyn LlmProvider, text: &str, company: &str, ticker: &str) -> Extracted {
    let v = match llm
        .complete_json(PROMPT, &user_prompt(text, company, ticker))
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let reason = format!("extraction failed: {e}");
            return Extracted {
                fiscal_period: Field::Missing {
                    reason: reason.clone(),
                },
                metrics: Metrics::all_missing(&reason),
                guidance: Field::Missing {
                    reason: reason.clone(),
                },
                highlights: Field::Missing { reason },
            };
        }
    };
    from_reply(text, &v)
}

/// The verification half of `extract`, on an LLM reply.
pub fn from_reply(text: &str, v: &Value) -> Extracted {
    let mut metrics = Metrics::all_missing("not stated in the press release");
    let m = v.get("metrics");
    for metric in Metric::ALL {
        let field = match m.and_then(|m| m.get(metric.key())) {
            None | Some(Value::Null) => continue,
            Some(raw) => match serde_json::from_value::<RawFigure>(raw.clone()) {
                Ok(raw) if is_gaap(metric) && non_gaap_wording(&raw.quote) => Field::Missing {
                    reason: "unverified: a GAAP figure quoted with non-GAAP wording".into(),
                },
                Ok(raw) => match verify_figure(text, metric, &raw) {
                    Ok(f) => Field::Value(f),
                    Err(e) => Field::Missing {
                        reason: format!("unverified: {e}"),
                    },
                },
                Err(e) => Field::Missing {
                    reason: format!("malformed: {e}"),
                },
            },
        };
        metrics.set(metric, field);
    }
    let fiscal_period = match v.get("fiscal_period") {
        None | Some(Value::Null) => Field::Missing {
            reason: "not stated in the press release".into(),
        },
        Some(p) => match verify_period(text, p) {
            Ok(p) => Field::Value(p),
            Err(e) => Field::Missing {
                reason: format!("unverified: {e}"),
            },
        },
    };
    Extracted {
        fiscal_period,
        metrics,
        guidance: Field::Value(verify_notes(text, v.get("guidance"))),
        highlights: Field::Value(verify_notes(text, v.get("highlights"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edgar::html_to_text;
    use crate::edgar::tests::EX991;
    use crate::providers::ProviderError;
    use async_trait::async_trait;
    use serde_json::json;

    const REPLY: &str = include_str!("../../tests/fixtures/llm_earnings_extract_AAPL.json");

    fn text() -> String {
        html_to_text(EX991)
    }
    fn raw(amount: &str, scale: &str, negative: bool, quote: &str) -> RawFigure {
        RawFigure {
            amount: amount.into(),
            scale: scale.into(),
            negative,
            quote: quote.into(),
        }
    }

    #[test]
    fn eps_quote_must_not_be_a_dividend() {
        let t = "The board declared a cash dividend of $0.26 per share. Diluted EPS was $1.85 for the quarter.";
        let e = verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$0.26",
                "units",
                false,
                "declared a cash dividend of $0.26 per share",
            ),
        );
        assert!(e.unwrap_err().contains("dividend"));
        assert!(verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$1.85",
                "units",
                false,
                "Diluted EPS was $1.85 for the quarter."
            ),
        )
        .is_ok());
    }

    #[test]
    fn a_mid_row_quote_is_rejected() {
        let t = "(In millions) Total net sales $102,466 $85,777 $391,035 $94,930";
        let e = verify_figure(
            t,
            Metric::Revenue,
            &raw("85,777", "millions", false, "$85,777 $391,035 $94,930"),
        );
        assert!(
            e.unwrap_err().contains("row label"),
            "letters before the amount"
        );
    }

    #[test]
    fn the_quote_must_name_the_metric() {
        let t = text();
        // Net income's row quoted as revenue, and the reverse.
        let e = verify_figure(
            &t,
            Metric::Revenue,
            &raw("27,466", "millions", false, "Net income $27,466 $14,736"),
        );
        assert!(e.unwrap_err().contains("revenue"));
        let e = verify_figure(
            &t,
            Metric::NetIncome,
            &raw(
                "102,466",
                "millions",
                false,
                "Total net sales $102,466 $94,930",
            ),
        );
        assert!(e.unwrap_err().contains("net income"));
        // Operating income is not operating cash flow or margin.
        let e = verify_figure(
            &t,
            Metric::OperatingCashFlow,
            &raw(
                "32,427",
                "millions",
                false,
                "Operating income $32,427 $29,591",
            ),
        );
        assert!(e.is_err());
        for (m, kw) in [
            (Metric::Revenue, "revenue"),
            (Metric::NetIncome, "net income"),
            (Metric::EpsGaap, "diluted"),
            (Metric::OperatingMargin, "operating margin"),
            (Metric::OperatingCashFlow, "operating activities"),
            (Metric::FreeCashFlow, "free cash flow"),
        ] {
            assert!(label_keywords(m).contains(&kw), "{m:?}");
        }
    }

    #[test]
    fn segment_revenue_rows_need_total() {
        let t = "(In millions) Products net sales $80,000 $70,000\nServices revenue $22,466 $20,000\nTotal net sales $102,466 $94,930\nNet revenues $9,000 $8,000";
        for (amount, q) in [
            ("80,000", "Products net sales $80,000 $70,000"),
            ("22,466", "Services revenue $22,466 $20,000"),
        ] {
            let e = verify_figure(t, Metric::Revenue, &raw(amount, "millions", false, q));
            assert!(e.unwrap_err().contains("segment"), "{q}");
        }
        for (amount, q) in [
            ("102,466", "Total net sales $102,466 $94,930"),
            ("9,000", "Net revenues $9,000 $8,000"),
        ] {
            assert!(
                verify_figure(t, Metric::Revenue, &raw(amount, "millions", false, q)).is_ok(),
                "{q}"
            );
        }
    }

    #[test]
    fn net_income_is_never_a_per_share_row() {
        let t = "(In millions, except per-share amounts) Net loss per share $(0.25) $(0.10)\nNet income per diluted share $1.85 $0.97\nNet loss $(25) $(10)";
        for (amount, q) in [
            ("(0.25)", "Net loss per share $(0.25) $(0.10)"),
            ("1.85", "Net income per diluted share $1.85 $0.97"),
        ] {
            let e = verify_figure(t, Metric::NetIncome, &raw(amount, "millions", true, q));
            assert!(e.unwrap_err().contains("per share"), "{q}");
        }
        assert_eq!(
            verify_figure(
                t,
                Metric::NetIncome,
                &raw("(25)", "millions", true, "Net loss $(25) $(10)")
            )
            .unwrap()
            .value,
            -25e6
        );
    }

    #[test]
    fn total_rows_that_are_not_total_revenue_are_rejected() {
        let rows = [
            ("40,000", "Total cost of revenues $40,000 $38,000", false),
            ("9,000", "Total noninterest revenue $9,000 $8,000", false),
            (
                "1,000",
                "Total cost of revenues and other income $1,000 $2,000",
                false,
            ),
            (
                "25,000",
                "Total revenues, net of interest expense $25,000 $24,000",
                true,
            ),
            ("30,000", "Total net revenue $30,000 $29,000", true),
            // Telecoms and utilities (VZ, T, DUK, SO), oil majors (XOM,
            // CVX), GM.
            ("33,000", "Total operating revenues $33,000 $32,000", true),
            (
                "84,000",
                "Total revenues and other income $84,000 $90,000",
                true,
            ),
            (
                "48,000",
                "Total net sales and revenue $48,000 $44,000",
                true,
            ),
        ];
        let t = format!(
            "(In millions) {}",
            rows.iter().map(|r| r.1).collect::<Vec<_>>().join("\n")
        );
        for (amount, q, ok) in rows {
            let r = verify_figure(&t, Metric::Revenue, &raw(amount, "millions", false, q));
            assert_eq!(r.is_ok(), ok, "{q}: {r:?}");
        }
    }

    #[test]
    fn operating_margin_is_gaap_only() {
        let text = "Adjusted operating margin was 31.0% for the quarter. Operating margin was 29.5% for the quarter.";
        let v = json!({"metrics": {"operating_margin_pct": {"amount": "31.0%", "scale": "units", "negative": false, "quote": "Adjusted operating margin was 31.0% for the quarter."}}});
        assert!(from_reply(text, &v)
            .metrics
            .value(Metric::OperatingMargin)
            .is_none());
        let v = json!({"metrics": {"operating_margin_pct": {"amount": "29.5%", "scale": "units", "negative": false, "quote": "Operating margin was 29.5% for the quarter."}}});
        assert_eq!(
            from_reply(text, &v)
                .metrics
                .value(Metric::OperatingMargin)
                .unwrap()
                .value,
            29.5
        );
        assert_eq!(
            Metric::OperatingMargin.label_en(),
            "Operating margin (GAAP)"
        );
        assert_eq!(Metric::OperatingMargin.label_zh(), "营业利润率（GAAP）");
    }

    #[test]
    fn prose_amount_with_size_word() {
        let f = verify_figure(
            &text(),
            Metric::Revenue,
            &raw(
                "$102.5 billion",
                "units",
                false,
                "The Company posted quarterly revenue of $102.5 billion, up 8 percent",
            ),
        )
        .unwrap();
        assert_eq!(f.value, 102.5e9);
        assert_eq!(f.scale, 1.0);
    }

    #[test]
    fn table_amount_uses_stated_scale() {
        let t = text();
        let f = verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "102,466",
                "millions",
                false,
                "Total net sales $102,466 $94,930",
            ),
        )
        .unwrap();
        assert_eq!(f.value, 102.466e9);
        // A bare number needs a scale, and one the document states.
        assert!(verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "102,466",
                "units",
                false,
                "Total net sales $102,466 $94,930"
            )
        )
        .is_err());
        // The heading also says "reflected in thousands" (shares); the table is in millions.
        assert!(verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "102,466",
                "thousands",
                false,
                "Total net sales $102,466 $94,930"
            )
        )
        .is_err());
        // No heading above the quote.
        let bare = "Total net sales $102,466 $94,930 (in millions)";
        assert!(verify_figure(
            bare,
            Metric::Revenue,
            &raw(
                "102,466",
                "millions",
                false,
                "Total net sales $102,466 $94,930"
            )
        )
        .is_err());
    }

    #[test]
    fn quote_must_be_in_exhibit_and_hold_amount() {
        let t = text();
        let e = verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "$103 billion",
                "units",
                false,
                "The Company posted quarterly revenue of $103 billion, up 8 percent",
            ),
        );
        assert!(e.unwrap_err().contains("not found"));
        let e = verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "$99.9 billion",
                "units",
                false,
                "The Company posted quarterly revenue of $102.5 billion, up 8 percent",
            ),
        );
        assert!(e.unwrap_err().contains("not in its quote"));
        let e = verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "$102.5 billion, up 8",
                "units",
                false,
                "The Company posted quarterly revenue of $102.5 billion, up 8 percent",
            ),
        );
        assert!(e.unwrap_err().contains("exactly one number"));
    }

    #[test]
    fn units_are_enforced() {
        let t = text();
        let eps = verify_figure(
            &t,
            Metric::EpsGaap,
            &raw(
                "$1.85",
                "units",
                false,
                "quarterly diluted earnings per share of $1.85, up 91 percent",
            ),
        )
        .unwrap();
        assert_eq!((eps.value, eps.unit), (1.85, Unit::UsdPerShare));
        let gm = verify_figure(
            &t,
            Metric::GrossMargin,
            &raw(
                "47.2 percent",
                "units",
                false,
                "Gross margin percentage was 47.2 percent for the quarter.",
            ),
        )
        .unwrap();
        assert_eq!(gm.value, 47.2);
        assert!(verify_figure(
            &t,
            Metric::GrossMargin,
            &raw(
                "$1.85",
                "units",
                false,
                "quarterly diluted earnings per share of $1.85, up 91 percent"
            )
        )
        .is_err());
        assert!(verify_figure(
            &t,
            Metric::Revenue,
            &raw(
                "8 percent",
                "units",
                false,
                "The Company posted quarterly revenue of $102.5 billion, up 8 percent"
            )
        )
        .is_err());
    }

    #[test]
    fn negatives_need_evidence() {
        let t = "Net loss per share $(0.25) for the quarter; diluted EPS was $0.40 in the period.";
        let f = verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$(0.25)",
                "units",
                false,
                "Net loss per share $(0.25) for the quarter",
            ),
        )
        .unwrap();
        assert_eq!(f.value, -0.25);
        let f = verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "0.25",
                "units",
                true,
                "Net loss per share $(0.25) for the quarter",
            ),
        )
        .unwrap();
        assert_eq!(f.value, -0.25);
        let e = verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$0.40",
                "units",
                true,
                "diluted EPS was $0.40 in the period",
            ),
        );
        assert!(e.unwrap_err().contains("negative"));
    }

    #[test]
    fn amount_must_be_a_whole_number_in_the_quote() {
        let t = text();
        let q = "quarterly diluted earnings per share of $1.85, up 91 percent";
        assert!(verify_figure(&t, Metric::EpsGaap, &raw("$1.8", "units", false, q)).is_err());
        let q = "Net income $27,466 $14,736";
        assert!(verify_figure(&t, Metric::NetIncome, &raw("7,466", "millions", false, q)).is_err());
        assert_eq!(
            verify_figure(&t, Metric::NetIncome, &raw("27,466", "millions", false, q))
                .unwrap()
                .value,
            27.466e9
        );
        // The prior-year column is not the row's first number.
        assert!(
            verify_figure(&t, Metric::NetIncome, &raw("14,736", "millions", false, q)).is_err()
        );
    }

    #[test]
    fn unparenthesised_scale_headings() {
        // CVS: the heading line carries the column years, no parentheses.
        let cvs = "In billions, except per share amounts 2026 2025 2026 Projected\nTotal revenues $108.4 $102.9 At least $420.0\nIn millions, except per share amounts 2026 2025 Change 2026 2025 Change\nTotal revenues $108,399 $102,871 $5,528 $314,921 $296,374 $18,547";
        let q = "Total revenues $108,399 $102,871 $5,528 $314,921 $296,374 $18,547";
        let f = verify_figure(cvs, Metric::Revenue, &raw("$108,399", "millions", false, q));
        assert_eq!(f.unwrap().value, 108.399e9);
        let q = "Total revenues $108.4 $102.9 At least $420.0";
        let f = verify_figure(cvs, Metric::Revenue, &raw("$108.4", "billions", false, q));
        assert_eq!(f.unwrap().value, 108.4e9);
        let f = verify_figure(cvs, Metric::Revenue, &raw("$108.4", "millions", false, q));
        assert!(f.unwrap_err().contains("not in millions"));
        // MCD: "Dollars in millions", heading on its own line.
        let mcd = "KEY FINANCIAL METRICS - CONSOLIDATED\nDollars in millions, except per share data\nQuarters Ended September 30, Nine Months Ended September 30,\n2026 2025 Inc/ (Dec) Inc/ (Dec) Excluding Currency Translation 2026 2025 Inc/ (Dec) Inc/ (Dec) Excluding Currency Translation\nRevenues $7,157 $7,078 1% 1% $20,773 $19,876 5% 2%";
        let q = "Revenues $7,157 $7,078 1% 1% $20,773 $19,876 5% 2%";
        let f = verify_figure(mcd, Metric::Revenue, &raw("$7,157", "millions", false, q));
        assert_eq!(f.unwrap().value, 7.157e9);
        let mcd2 = "Dollars and shares in millions, except per share data\nTOTAL REVENUES 7,157 7,078 79 1";
        let f = verify_figure(
            mcd2,
            Metric::Revenue,
            &raw(
                "7,157",
                "millions",
                false,
                "TOTAL REVENUES 7,157 7,078 79 1",
            ),
        );
        assert_eq!(f.unwrap().value, 7.157e9);
        // Inside a parenthesised heading only the lead counts, even when a
        // later part reads like an unparenthesised heading.
        let t =
            "(In millions, shares in thousands, except per share data)\nTotal revenues $1,000 $900";
        let q = "Total revenues $1,000 $900";
        assert!(verify_figure(t, Metric::Revenue, &raw("1,000", "millions", false, q)).is_ok());
        assert!(verify_figure(t, Metric::Revenue, &raw("1,000", "thousands", false, q)).is_err());
    }

    #[test]
    fn prior_year_first_tables_read_the_latest_column() {
        // EMR: "2025 Q4 2026 Q4", prior year first, on the heading line.
        let emr = "(dollars in millions, except per share) 2025 Q4 2026 Q4 Change 2025 2026 Change\nUnderlying Orders2 14% 9%\nNet Sales $4,855 $5,201 7% $18,016 $18,982 5%";
        let q = "Net Sales $4,855 $5,201 7% $18,016 $18,982 5%";
        let f = verify_figure(emr, Metric::Revenue, &raw("$5,201", "millions", false, q));
        assert_eq!(f.unwrap().value, 5.201e9);
        // The row's first number is the prior year; the full-year column
        // is a 2026 column too, but not the first.
        for amount in ["$4,855", "$18,016", "$18,982"] {
            let r = verify_figure(emr, Metric::Revenue, &raw(amount, "millions", false, q));
            assert!(r.unwrap_err().contains("current period"), "{amount}");
        }
        // UBER: "2025 2026% Change" on the heading line, and a statement
        // whose years sit on their own line under the heading.
        let uber = "(In millions, except percentages and per share amounts) 2025 2026% Change % Change (Constant Currency (1))\nTrips 3,512 4,061 16%\nGross Bookings $49,740 $59,819 20% 20%\nRevenue $13,467 $14,766 10% 10%\n(In millions, except share amounts which are reflected in thousands, and per share amounts)\n(Unaudited)\nThree Months Ended September 30, Nine Months Ended September 30,\n2025 2026 2025 2026\nRevenue $13,467 $14,766 $37,651 $42,160";
        let q = "Revenue $13,467 $14,766 10% 10%";
        let f = verify_figure(uber, Metric::Revenue, &raw("$14,766", "millions", false, q));
        assert_eq!(f.unwrap().value, 14.766e9);
        assert!(
            verify_figure(uber, Metric::Revenue, &raw("$13,467", "millions", false, q)).is_err()
        );
        let q = "Revenue $13,467 $14,766 $37,651 $42,160";
        let f = verify_figure(uber, Metric::Revenue, &raw("$14,766", "millions", false, q));
        assert_eq!(f.unwrap().value, 14.766e9);
        for amount in ["$13,467", "$37,651", "$42,160"] {
            let r = verify_figure(uber, Metric::Revenue, &raw(amount, "millions", false, q));
            assert!(r.is_err(), "{amount}: {r:?}");
        }
        // Current-year-first header: the second column stays rejected.
        let t = "(In millions)\nThree Months Ended\nSeptember 26, 2026 September 27, 2025\nTotal net sales $102,466 $94,930";
        let q = "Total net sales $102,466 $94,930";
        assert!(verify_figure(t, Metric::Revenue, &raw("94,930", "millions", false, q)).is_err());
        assert!(verify_figure(t, Metric::Revenue, &raw("102,466", "millions", false, q)).is_ok());
        // No year header between the scale heading and the row.
        let t = "(In millions)\nTotal net sales $102,466 $94,930";
        assert!(verify_figure(t, Metric::Revenue, &raw("94,930", "millions", false, q)).is_err());
    }

    #[test]
    fn footnote_markers_are_not_columns() {
        // UBER's highlights table, as html_to_text renders it.
        let uber = "Three Months Ended September 30,\n(In millions, except percentages and per share amounts) 2025 2026% Change % Change (Constant Currency (1))\nRevenue $13,467 $14,766 10% 10%\nGAAP Income from operations $1,113 $1,447 30%\nGAAP Net income attributable to Uber Technologies, Inc. (2) $6,626 $512 (92) %\nGAAP Diluted EPS $3.11 $0.25 (92) %\nNon-GAAP EPS (1) $0.65 $0.88 35%";
        let q = "GAAP Net income attributable to Uber Technologies, Inc. (2) $6,626 $512 (92) %";
        let f = verify_figure(uber, Metric::NetIncome, &raw("$512", "millions", false, q));
        assert_eq!(f.unwrap().value, 512e6);
        let r = verify_figure(
            uber,
            Metric::NetIncome,
            &raw("$6,626", "millions", false, q),
        );
        assert!(r.unwrap_err().contains("current period"));
        let q = "Non-GAAP EPS (1) $0.65 $0.88 35%";
        let f = verify_figure(uber, Metric::EpsNonGaap, &raw("$0.88", "units", false, q));
        assert_eq!(f.unwrap().value, 0.88);
        assert!(verify_figure(uber, Metric::EpsNonGaap, &raw("$0.65", "units", false, q)).is_err());
        // Without a header, a footnoted row's first number is its first column.
        let t = "(In millions)\nNet income (1) $900 $800";
        let q = "Net income (1) $900 $800";
        assert!(verify_figure(t, Metric::NetIncome, &raw("$900", "millions", false, q)).is_ok());
        assert!(verify_figure(t, Metric::NetIncome, &raw("$800", "millions", false, q)).is_err());
        // Small negative values in parentheses are columns, not footnotes.
        let t = "(In millions)\n2026 2025\nNet loss for the period (62) (37)";
        let q = "Net loss for the period (62) (37)";
        assert!(verify_figure(t, Metric::NetIncome, &raw("(37)", "millions", true, q)).is_err());
        let f = verify_figure(t, Metric::NetIncome, &raw("(62)", "millions", true, q));
        assert_eq!(f.unwrap().value, -62e6);
    }

    #[test]
    fn untrusted_headers_fail_closed() {
        let reject = |t: &str, m: Metric, amount: &str, scale: &str, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, false, q));
            assert!(r.is_err(), "{amount} in {q:?}: {r:?}");
        };
        // Five trailing quarters, oldest first: no year-per-block shape.
        let t = "(In millions)\nSep 30, 2025 Dec 31, 2025 Mar 31, 2026 Jun 30, 2026 Sep 30, 2026\nTotal revenues $100 $105 $110 $115 $120";
        let q = "Total revenues $100 $105 $110 $115 $120";
        for a in ["$100", "$110", "$120"] {
            reject(t, Metric::Revenue, a, "millions", q);
        }
        // Sequential quarter labels, oldest first.
        let t = "(In millions)\nQ3 2025 Q2 2026 Q3 2026\nTotal revenues $100 $115 $120";
        let q = "Total revenues $100 $115 $120";
        for a in ["$100", "$115", "$120"] {
            reject(t, Metric::Revenue, a, "millions", q);
        }
        // The same, newest first: the first column is the current quarter.
        let t = "(In millions)\nQ3 2026 Q2 2026 Q3 2025\nTotal revenues $120 $115 $100";
        let q = "Total revenues $120 $115 $100";
        assert!(verify_figure(t, Metric::Revenue, &raw("$120", "millions", false, q)).is_ok());
        reject(t, Metric::Revenue, "$115", "millions", q);
        // GAAP and non-GAAP columns under each year, prior year first.
        let t = "(In millions, except per share)\n2025 2026\nGAAP Non-GAAP GAAP Non-GAAP\nDiluted EPS $5.93 $5.64 $7.08 $6.45";
        let q = "Diluted EPS $5.93 $5.64 $7.08 $6.45";
        for a in ["$5.93", "$5.64", "$7.08", "$6.45"] {
            reject(t, Metric::EpsGaap, a, "units", q);
            reject(t, Metric::EpsNonGaap, a, "units", q);
        }
        // Nine months before three months, either year order.
        let t = "(In millions)\nNine Months Ended September 30, Three Months Ended September 30,\n2025 2026 2025 2026\nTotal revenues $300 $360 $100 $120";
        let q = "Total revenues $300 $360 $100 $120";
        for a in ["$300", "$360", "$100", "$120"] {
            reject(t, Metric::Revenue, a, "millions", q);
        }
        let t = "(In millions)\nNine Months Ended September 30, Three Months Ended September 30,\n2026 2025 2026 2025\nTotal revenues $360 $300 $120 $100";
        let q = "Total revenues $360 $300 $120 $100";
        for a in ["$360", "$120"] {
            reject(t, Metric::Revenue, a, "millions", q);
        }
        // Three months first, prior year first, all on the heading line.
        let t = "(In millions) Three Months Ended 2025 2026 Nine Months Ended 2025 2026\nTotal revenues $100 $120 $300 $360";
        let q = "Total revenues $100 $120 $300 $360";
        let f = verify_figure(t, Metric::Revenue, &raw("$120", "millions", false, q));
        assert_eq!(f.unwrap().value, 120e6);
        for a in ["$100", "$300", "$360"] {
            reject(t, Metric::Revenue, a, "millions", q);
        }
    }

    #[test]
    fn headers_stay_within_their_table() {
        let ok = |t: &str, amount: &str, q: &str| {
            let r = verify_figure(t, Metric::Revenue, &raw(amount, "millions", false, q));
            assert!(r.is_ok(), "{amount} in {q:?}: {r:?}");
        };
        let bad = |t: &str, amount: &str, q: &str| {
            let r = verify_figure(t, Metric::Revenue, &raw(amount, "millions", false, q));
            assert!(r.is_err(), "{amount} in {q:?}: {r:?}");
        };
        let q = "Total revenues $14,766 $13,467";
        // A previous table's prior-year-first header does not reach past a
        // title line, nor past the next table's own "Q3'26 Q3'25" header.
        let t = "(In millions) 2025 2026\nGross Bookings $49,740 $59,819\nSupplemental table\nQ3'26 Q3'25\nTotal revenues $14,766 $13,467";
        ok(t, "$14,766", q);
        bad(t, "$13,467", q);
        let t = "(In millions) 2025 2026\nGross Bookings $49,740 $59,819\nQ3'26 Q3'25\nTotal revenues $14,766 $13,467";
        ok(t, "$14,766", q);
        bad(t, "$13,467", q);
        let t = "(In millions) 2025 2026\nGross Bookings $49,740 $59,819\nSupplemental table\nTotal revenues $14,766 $13,467";
        ok(t, "$14,766", q);
        bad(t, "$13,467", q);
        // A "$ Change" column label does not make the header a data row.
        let t = "(In millions)\n2025 2026 $ Change\nTotal revenues $100 $120 $20";
        let q = "Total revenues $100 $120 $20";
        bad(t, "$100", q);
        // A prose line with years is not a header.
        let t = "(In millions)\n2026 2025\nRevenue by region for 2025 and 2026 restated\nTotal revenues $120 $100";
        let q = "Total revenues $120 $100";
        ok(t, "$120", q);
        bad(t, "$100", q);
    }

    #[test]
    fn per_share_rows_read_the_latest_column() {
        // Without a header, a row of periods is read from its first column.
        let t = "Diluted EPS $1.33 $1.08";
        assert!(verify_figure(t, Metric::EpsGaap, &raw("$1.08", "units", false, t)).is_err());
        assert!(verify_figure(t, Metric::EpsGaap, &raw("$1.33", "units", false, t)).is_ok());
        // AMGN: years group GAAP and non-GAAP columns, so only the row's
        // first number maps to a year column with certainty.
        let amgn = "(In millions, except per-share data)\n(Unaudited)\nThe following table presents the computations for GAAP and non-GAAP diluted earnings per share:\nThree months ended September 30, 2026 Three months ended September 30, 2025\nGAAP Non-GAAP GAAP Non-GAAP\nNet income $3,864 $3,520 $3,216 $3,055\nShares (Denominator):\nWeighted-average shares for diluted EPS 546 546 542 542\nDiluted EPS $7.08 $6.45 $5.93 $5.64";
        let q = "Diluted EPS $7.08 $6.45 $5.93 $5.64";
        let f = verify_figure(amgn, Metric::EpsGaap, &raw("$7.08", "units", false, q));
        assert_eq!(f.unwrap().value, 7.08);
        for amount in ["$6.45", "$5.93", "$5.64"] {
            for m in [Metric::EpsGaap, Metric::EpsNonGaap] {
                let r = verify_figure(amgn, m, &raw(amount, "units", false, q));
                assert!(r.is_err(), "{amount} {m:?}: {r:?}");
            }
        }
        // EMR: prior year first.
        let emr = "(AMOUNTS IN MILLIONS EXCEPT PER SHARE, UNAUDITED)\nQuarter Ended September 30, Year Ended September 30,\n2025 2026 2025 2026\nNet sales $4,855 $5,201 $18,016 $18,982\nDiluted earnings per common share $1.08 $1.33 $4.00 $4.79";
        let q = "Diluted earnings per common share $1.08 $1.33 $4.00 $4.79";
        let f = verify_figure(emr, Metric::EpsGaap, &raw("$1.33", "units", false, q));
        assert_eq!(f.unwrap().value, 1.33);
        for amount in ["$1.08", "$4.00", "$4.79"] {
            let r = verify_figure(emr, Metric::EpsGaap, &raw(amount, "units", false, q));
            assert!(r.unwrap_err().contains("current period"), "{amount}");
        }
        // UBER: a footnote marker in the label is not a column.
        let uber = "(In millions, except percentages and per share amounts) 2025 2026% Change % Change (Constant Currency (1))\nGAAP Diluted EPS $3.11 $0.25 (92) %\nNon-GAAP EPS (1) $0.65 $0.88 35%";
        let q = "GAAP Diluted EPS $3.11 $0.25 (92) %";
        let f = verify_figure(uber, Metric::EpsGaap, &raw("$0.25", "units", false, q));
        assert_eq!(f.unwrap().value, 0.25);
        assert!(verify_figure(uber, Metric::EpsGaap, &raw("$3.11", "units", false, q)).is_err());
        let q = "Non-GAAP EPS (1) $0.65 $0.88 35%";
        let f = verify_figure(uber, Metric::EpsNonGaap, &raw("$0.88", "units", false, q));
        assert_eq!(f.unwrap().value, 0.88);
        // AMD: net income and per-share columns alternate; the per-share
        // cells are the row's periods.
        let amd = "(In millions, except per share amounts)\nThree Months Ended\nSeptember 26, 2026 June 27, 2026 September 27, 2025\nNon-GAAP net income / earnings per share $3,471 $2.09 $2,760 $1.66 $1,965 $1.20";
        let q = "Non-GAAP net income / earnings per share $3,471 $2.09 $2,760 $1.66 $1,965 $1.20";
        let f = verify_figure(amd, Metric::EpsNonGaap, &raw("$2.09", "units", false, q));
        assert_eq!(f.unwrap().value, 2.09);
        for a in ["$1.66", "$1.20"] {
            assert!(verify_figure(amd, Metric::EpsNonGaap, &raw(a, "units", false, q)).is_err());
        }
        // Prose that cites another per-share figure first is not a table row.
        let t = "Excluding the current year charges described above of $0.06 per share, diluted earnings per share was $3.57, an increase of 11%.";
        assert!(verify_figure(t, Metric::EpsNonGaap, &raw("$3.57", "units", false, t)).is_ok());
        // Prose under a prior-year-first header, comparing two periods, is
        // not a row of columns either.
        let t = format!("{emr}\nDiluted EPS was $1.33, compared with $1.08 a year ago.");
        let q = "Diluted EPS was $1.33, compared with $1.08 a year ago.";
        assert!(verify_figure(&t, Metric::EpsGaap, &raw("$1.33", "units", false, q)).is_ok());
    }

    #[test]
    fn size_word_after_the_amount_counts() {
        let t = text();
        let q = "The Company posted quarterly revenue of $102.5 billion, up 8 percent";
        let f = verify_figure(&t, Metric::Revenue, &raw("102.5", "millions", false, q)).unwrap();
        assert_eq!(f.value, 102.5e9);
    }

    #[test]
    fn per_share_rules() {
        let t = "Revenue was $1.2 billion. Net income per share was $0.40 for the period.";
        assert!(verify_figure(
            t,
            Metric::EpsGaap,
            &raw("$1.2 billion", "units", false, "Revenue was $1.2 billion.")
        )
        .is_err());
        assert!(verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$0.40",
                "units",
                false,
                "Net income per share was $0.40 for the period."
            )
        )
        .is_ok());
    }

    #[test]
    fn loss_wording_sets_the_sign() {
        let t =
            "Net loss was $25 million in the quarter. Net income (loss) $27,466 for the period.";
        let f = verify_figure(
            t,
            Metric::NetIncome,
            &raw(
                "$25 million",
                "units",
                true,
                "Net loss was $25 million in the quarter.",
            ),
        );
        assert_eq!(f.unwrap().value, -25e6);
        let f = verify_figure(
            t,
            Metric::NetIncome,
            &raw(
                "$25 million",
                "units",
                false,
                "Net loss was $25 million in the quarter.",
            ),
        );
        assert!(f.unwrap_err().contains("loss"));
        // "(loss)" in a row label is not evidence of a negative figure.
        let t2 = "(In millions) Net income (loss) $27,466 for the period.";
        let f = verify_figure(
            t2,
            Metric::NetIncome,
            &raw(
                "27,466",
                "millions",
                true,
                "Net income (loss) $27,466 for the period.",
            ),
        );
        assert!(f.is_err());
        let f = verify_figure(
            t2,
            Metric::NetIncome,
            &raw(
                "27,466",
                "millions",
                false,
                "Net income (loss) $27,466 for the period.",
            ),
        );
        assert_eq!(f.unwrap().value, 27.466e9);
    }

    #[test]
    fn gaap_metrics_reject_non_gaap_wording() {
        let text = "Managed revenue of $42.1 billion on a fully taxable-equivalent basis. Reported revenue of $41.5 billion for the quarter.";
        let v = json!({"metrics": {
            "revenue": {"amount": "$42.1 billion", "scale": "units", "negative": false, "quote": "Managed revenue of $42.1 billion on a fully taxable-equivalent basis."},
            "eps_non_gaap_diluted": null
        }});
        assert!(from_reply(text, &v)
            .metrics
            .value(Metric::Revenue)
            .is_none());
        assert!(!non_gaap_wording("Revenue rose after strong iPhone sales"));
        let v = json!({"metrics": {"revenue": {"amount": "$41.5 billion", "scale": "units", "negative": false, "quote": "Reported revenue of $41.5 billion for the quarter."}}});
        assert_eq!(
            from_reply(text, &v)
                .metrics
                .value(Metric::Revenue)
                .unwrap()
                .value,
            41.5e9
        );
    }

    #[test]
    fn reply_fixture() {
        let v: Value = serde_json::from_str(REPLY).unwrap();
        let x = from_reply(&text(), &v);
        assert_eq!(x.metrics.value(Metric::Revenue).unwrap().value, 102.5e9);
        assert_eq!(x.metrics.value(Metric::NetIncome).unwrap().value, 27.466e9);
        assert_eq!(x.metrics.value(Metric::EpsGaap).unwrap().value, 1.85);
        assert_eq!(x.metrics.value(Metric::GrossMargin).unwrap().value, 47.2);
        assert_eq!(
            x.metrics.value(Metric::OperatingCashFlow).unwrap().value,
            29.7e9
        );
        // Quote invented by the model: not in the exhibit.
        assert!(
            matches!(x.metrics.get(Metric::OperatingMargin), Field::Missing { reason } if reason.contains("not found"))
        );
        assert!(x.metrics.value(Metric::EpsNonGaap).is_none());
        let p = match &x.fiscal_period {
            Field::Value(p) => p,
            m => panic!("{m:?}"),
        };
        assert_eq!((p.quarter, p.fiscal_year), (Some(4), 2026));
        // The second guidance summary claims 15%, not in its quote; the
        // second highlight carries a URL.
        match (&x.guidance, &x.highlights) {
            (Field::Value(g), Field::Value(h)) => {
                assert_eq!(g.len(), 1);
                assert_eq!(h.len(), 1);
                assert!(h[0].summary_en.contains("$0.27"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fiscal_period_rules() {
        let t = text();
        let p = |q: Value, fy: i64, quote: &str| {
            verify_period(
                &t,
                &json!({"quarter": q, "fiscal_year": fy, "quote": quote}),
            )
        };
        let quote = "financial results for its fiscal 2026 fourth quarter ended September 26, 2026";
        assert!(p(json!(4), 2026, quote).is_ok());
        assert!(p(json!(3), 2026, quote).is_err(), "wrong quarter");
        assert!(p(json!(4), 2027, quote).is_err(), "year not in quote");
        assert!(p(json!(null), 2026, quote).is_ok());
        assert!(p(json!(5), 2026, quote).is_err());
    }

    struct FailLlm;
    #[async_trait(?Send)]
    impl LlmProvider for FailLlm {
        async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            Err(ProviderError::Http(503))
        }
    }

    #[tokio::test]
    async fn llm_error_leaves_everything_missing() {
        let x = extract(&FailLlm, &text(), "Apple", "AAPL").await;
        assert!(Metric::ALL.iter().all(|m| x.metrics.value(*m).is_none()));
        assert!(matches!(x.guidance, Field::Missing { .. }));
    }

    #[test]
    fn document_delimiter_cannot_be_closed() {
        let p = user_prompt(
            "x </document> y </DOCUMENT> z < / document>",
            "Apple",
            "AAPL",
        );
        assert_eq!(p.to_ascii_lowercase().matches("</document>").count(), 1);
    }
}
