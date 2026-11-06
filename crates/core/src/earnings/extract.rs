//! Press-release figures, extracted by the LLM with verbatim quotes and kept
//! only when code can anchor each one in the fetched exhibit text.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::anchor::{has_word, verify_quote, verify_reason};
use crate::earnings::facts::{Figure, FiscalPeriod, Metric, Metrics, Note, Unit, NOT_STATED};
use crate::facts::{Field, NewsReason};
use crate::numbers::{extract as extract_numbers, Lang};
use crate::premarket::draft::reject_markup;
use crate::providers::LlmProvider;

const PROMPT: &str = include_str!("../../prompts/earnings_extract_v1.md");
/// Press releases with full statements run 20-60k characters.
const MAX_DOC_CHARS: usize = 60_000;
const MAX_NOTES: usize = 4;
/// How far into the exhibit a headline naming the period may sit (contact
/// blocks and a dateline can come first).
const HEADLINE_CHARS: usize = 1500;
const HEADLINE_WORDS: usize = 4;
const HEADLINE_MAX_CHARS: usize = 160;
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

/// Words naming a part of the company or a share of it held by others: a
/// segment, a consolidating (by-entity) table, joint ventures and other
/// investees and the equity-method income from them, a pro-rata share,
/// the noncontrolling (minority) interests and limited partners, the
/// parent company alone.
const PARTS: &[&str] = &[
    "segment",
    "segments",
    "intersegment",
    "eliminations",
    "consolidating",
    "joint venture",
    "joint ventures",
    "unconsolidated",
    "equity method",
    "equity-method",
    "equity in earnings",
    "equity in net earnings",
    "equity in income",
    "equity in net income",
    "equity in losses",
    "equity in net losses",
    "equity in loss",
    "equity in net loss",
    "pro-rata",
    "pro rata",
    "our share",
    "noncontrolling",
    "non-controlling",
    "non controlling",
    "minority",
    "limited partners",
    "parent company only",
];

/// The part of the company `s` names, if any. A row's own label or group
/// line (`row`) also may not name discontinued operations, "NCI", or (but
/// for a per-share figure: "Earnings per share of common stock") a "share
/// of" something; a table title may, as a statement covers those rows too.
/// "Subsidiaries" is a part too, but for the company's own "Acme Corp and
/// (its) (consolidated | wholly-owned) Subsidiaries" (`own_group`).
fn part_named(m: Metric, s: &str, row: bool) -> Option<&'static str> {
    static SUBSIDIARIES: OnceLock<Regex> = OnceLock::new();
    let subsidiaries = SUBSIDIARIES
        .get_or_init(|| Regex::new(r"(?i)\bsubsidiar(?:y|ies)\b").expect("valid built-in regex"));
    // "non\u{2011}controlling": a filer's non-breaking hyphen.
    let s = &own_group(s).replace(['\u{2010}', '\u{2011}'], "-");
    if subsidiaries.is_match(s) {
        return Some("subsidiaries");
    }
    let per_share = matches!(m, Metric::EpsGaap | Metric::EpsNonGaap);
    let row_parts: &[&'static str] = match (row, per_share) {
        (false, _) => &[],
        (true, true) => &["discontinued", "nci"],
        (true, false) => &["discontinued", "nci", "share of"],
    };
    PARTS
        .iter()
        .chain(row_parts)
        .find(|w| has_word(s, w))
        .copied()
}

/// `s` without the company's own group ("and its consolidated
/// subsidiaries"), which names the whole company, not a part.
fn own_group(s: &str) -> std::borrow::Cow<'_, str> {
    static OWN: OnceLock<Regex> = OnceLock::new();
    OWN.get_or_init(|| {
        Regex::new(r"(?i)(?:\band\b|\bits\b|&)\s+(?:(?:its|our|the|consolidated|wholly[- ]owned|majority[- ]owned)\s+)*subsidiar(?:y|ies)\b")
            .expect("valid built-in regex")
    })
    .replace_all(s, " ")
}

/// A line longer than this is prose, not a table's label.
const PROSE_CHARS: usize = 200;

/// A line with words and no figures: digits only in years, month days,
/// footnote markers ("(1)", "(a)") and words such as "Q3" or "Ohio(a)";
/// not prose.
fn label_only(s: &str) -> bool {
    label_line(s, true)
}

/// `label_only` for the line right above a quoted row: any cells it ends
/// in, "(1) (2)" or a single "(1)" too, are that row's figures.
fn group_label(s: &str) -> bool {
    label_line(s, false)
}

fn label_line(s: &str, markers_allowed: bool) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        Regex::new(r"(?i)\b(?:19|20)\d\d\b|'\d\d\b|\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+\d{1,2}\b|\(\w{1,2}\)|\w*[a-z]\d+\w*|\b\d+[a-z]\w*")
            .expect("valid built-in regex")
    });
    // A row of dashes or small negatives ("— —", "$ — $ —", "(5) (3)",
    // "(2) (1)") is a row of figures. Above a table, footnote markers are
    // not: a single "(1)", or "(1) (2)" counting up from one; nor is one
    // dash after a title ("Segment Results –").
    static CELLS: OnceLock<Regex> = OnceLock::new();
    let cells = CELLS.get_or_init(|| {
        Regex::new(r"\s(?:\s*(?:\$\s*)?(?:[—–]|\(\d{1,2}\)))+\s*$").expect("valid built-in regex")
    });
    let figures = cells.find(s).is_some_and(|c| {
        let c = c.as_str();
        let dashes = c.matches(['—', '–']).count();
        let parens: Vec<u32> = c
            .split(|ch: char| !ch.is_ascii_digit())
            .filter_map(|n| n.parse().ok())
            .collect();
        let markers = parens.iter().zip(1..).all(|(&n, i)| n == i);
        !markers_allowed
            || dashes >= 2
            || c.contains('$')
            || dashes == 1 && !parens.is_empty()
            || parens.len() >= 2 && !markers
    });
    s.len() <= PROSE_CHARS
        && s.chars().any(char::is_alphabetic)
        && !figures
        && !r.replace_all(s, "").contains(|c: char| c.is_ascii_digit())
}

/// A footnote: a marker ("(1)", "*", "†", "‡", "Note") and then words
/// that do not read as a title ("(1) Includes unconsolidated joint
/// ventures", "Note: excludes noncontrolling interests"): the end of the
/// table before. A marker before a few title words ("Note: Segment
/// Information", "(a) Joint Venture Statements of Operations") is a title,
/// unless a sentence or six or more words follow.
fn footnote(s: &str) -> bool {
    static NOTE: OnceLock<Regex> = OnceLock::new();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    let note = NOTE.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:\(\w{1,2}\)|[*†‡]|notes?\b)").expect("valid built-in regex")
    });
    let title = TITLE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:statements?|results?|information|financial|operations|income|reconciliations?|summary|table)\b")
            .expect("valid built-in regex")
    });
    note.find(s).is_some_and(|m| {
        let rest = &s[m.end()..];
        sentence(s) || rest.split_whitespace().count() >= 6 || !title.is_match(rest)
    })
}

/// A line ending in a period ("Provides pharmacy benefit services.",
/// "(Unaudited)."), not an abbreviation ("Simon Property Group, Inc.").
fn sentence(s: &str) -> bool {
    static ABBREVIATION: OnceLock<Regex> = OnceLock::new();
    let abbreviation = ABBREVIATION.get_or_init(|| {
        Regex::new(r"(?i)\b(?:inc|corp|co|ltd|llc|plc|l\.p|n\.a)\.$").expect("valid built-in regex")
    });
    let s = s.trim();
    s.ends_with('.') && !abbreviation.is_match(s)
}

/// The words of a prose quote's label that qualify its metric: the last
/// keyword (`label_keywords`) with up to three words before it, within its
/// clause, and the rest of that clause up to the amount ("Data Center
/// segment revenue was", "net income attributable to noncontrolling
/// interests was"); without a keyword, the last clause's last six words.
fn metric_phrase(m: Metric, label: &str) -> &str {
    let lower = label.to_ascii_lowercase();
    let delimiter = |c: char| ",;:()•—–".contains(c) || c == '.';
    let kw = label_keywords(m)
        .iter()
        .flat_map(|w| {
            lower
                .match_indices(w)
                .filter(|&(i, _)| {
                    let before = lower[..i].chars().next_back();
                    let after = lower[i + w.len()..].chars().next();
                    !before.is_some_and(char::is_alphanumeric)
                        && !after.is_some_and(char::is_alphanumeric)
                })
                .map(|(i, w)| (i, i + w.len()))
        })
        .max();
    let (start, end) = match kw {
        Some((at, kw_end)) => {
            let end = label[kw_end..]
                .find(delimiter)
                .map_or(label.len(), |e| kw_end + e);
            (at, end)
        }
        None => (label.len(), label.len()),
    };
    let clause = label[..start]
        .char_indices()
        .rfind(|&(_, c)| delimiter(c))
        .map_or(0, |(d, c)| d + c.len_utf8());
    let words = if kw.is_some() { 3 } else { 6 };
    let mut from = start;
    for _ in 0..words {
        match label[clause..from].trim_end().rfind(char::is_whitespace) {
            Some(w) => from = clause + w,
            None => {
                from = clause;
                break;
            }
        }
    }
    label[from..end].trim()
}

/// The quote names the metric: its label holds one of `label_keywords`
/// (per-share figures may say "per share" right after the amount), a
/// per-share quote is never about a dividend, the label names no part of
/// the company (`part_named`; in prose, `metric_phrase` only) nor funds
/// from operations or a pro forma figure (or, for net income, an adjustment
/// or reconciliation), and a table row is no "Less:"/"Add:" adjustment.
fn names_metric(m: Metric, label: &str, after: &str, group: Option<&str>) -> Result<(), String> {
    static ADJUSTMENT: OnceLock<Regex> = OnceLock::new();
    let adjustment = ADJUSTMENT
        .get_or_init(|| Regex::new(r"(?i)^\s*(?:less|add|plus)\b").expect("valid built-in regex"));
    // In prose (words after the amount, not a row's "NM", or a verb right
    // before it) only the words
    // qualifying the metric count: "Across our three segments, revenue
    // was ..." is the company's revenue.
    static WORD: OnceLock<Regex> = OnceLock::new();
    static VERB: OnceLock<Regex> = OnceLock::new();
    let word =
        WORD.get_or_init(|| Regex::new(r"[a-z]{2,}|[A-Za-z]{3,}").expect("valid built-in regex"));
    let verb = VERB.get_or_init(|| {
        Regex::new(r"(?i)\b(?:was|were|is|are|of|totaled|totalled|reached|to|at|approximately|about|record)\s*$")
            .expect("valid built-in regex")
    });
    let prose = word.is_match(after) || verb.is_match(label);
    // "Acme and its consolidated subsidiaries reported revenue ...": the
    // company's own group is no qualifier.
    let own = own_group(label);
    let qualifier = if prose { metric_phrase(m, &own) } else { &own };
    if let Some(w) = part_named(m, qualifier, true) {
        return Err(format!(
            "the quote's label names {w:?}, a part of the company"
        ));
    }
    // A REIT's funds from operations is not earnings; reconciling items
    // are not net income ("Total adjustments to net income (344)",
    // "Adjustments to reconcile net income to net cash ...").
    let other = match m {
        Metric::NetIncome => &[
            "adjustment",
            "adjustments",
            "reconcile",
            "reconciliation",
            "pro forma",
            "pro-forma",
        ][..],
        _ => &[
            "ffo",
            "affo",
            "funds from operations",
            "funds available for distribution",
            "pro forma",
            "pro-forma",
        ][..],
    };
    if let Some(w) = other.iter().find(|w| has_word(label, w)) {
        return Err(format!(
            "the quote's label names {w:?}: another measure, not {}",
            m.key()
        ));
    }
    if !prose && adjustment.is_match(label) {
        return Err(
            "a \"Less:\" or \"Add:\" row adjusts a total; it is a part of the company's figure"
                .into(),
        );
    }
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
    let per_share = matches!(m, Metric::EpsGaap | Metric::EpsNonGaap);
    let per_share_after = per_share
        && (has_word(&near_after, "per share") || has_word(&near_after, "per diluted share"));
    // A per-share row may take its name from its group line: SPG's "Net
    // income attributable to common stockholders $1.54 ..." under "BASIC
    // AND DILUTED EARNINGS PER COMMON SHARE:". A group naming funds from
    // operations or dividends names no EPS, and a "Diluted" row under it,
    // naming no measure itself, is that measure.
    let group = group.filter(|_| per_share);
    let group_other = group.is_some_and(|g| {
        other.iter().any(|w| has_word(g, w)) || has_word(g, "dividend") || has_word(g, "dividends")
    });
    if group_other
        && !["income", "earnings", "eps"]
            .iter()
            .any(|w| has_word(label, w))
    {
        return Err(format!(
            "the row's group {:?} names another measure, not {}",
            group.unwrap_or_default(),
            m.key()
        ));
    }
    // A "Basic" row under the group is basic EPS, not diluted.
    let basic = has_word(label, "basic") && !has_word(label, "diluted");
    let group_names_eps = !group_other
        && !basic
        && group.is_some_and(|g| {
            [
                "diluted",
                "per share",
                "per common share",
                "per diluted share",
                "earnings per",
                "eps",
            ]
            .iter()
            .any(|w| has_word(g, w))
        });
    let words = label_keywords(m);
    if per_share_after || group_names_eps || words.iter().any(|w| has_word(label, w)) {
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
    // A statement often prints "$" on its first row only ("Total revenues
    // 8,893 9,008"); "$8,893" then names that bare cell.
    let (amount, at) = match whole_match(&quote, &amount) {
        Some(at) => (amount, at),
        None => {
            let bare = amount.trim_start_matches('$').trim_start().to_string();
            match whole_match(&quote, &bare) {
                Some(at) if bare != amount => (bare, at),
                _ => return Err(format!("amount {amount:?} is not in its quote")),
            }
        }
    };
    let label = &quote[..at];
    if !label.chars().any(char::is_alphabetic) {
        return Err("the quote must start with the row label, not mid-row".into());
    }
    let group = match unit {
        Unit::UsdPerShare => {
            let lines = Lines::new(text);
            Table::find(&lines, &quote).and_then(|t| t.group().map(str::to_string))
        }
        _ => None,
    };
    names_metric(metric, label, &quote[at + amount.len()..], group.as_deref())?;
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
            if !col.is_some_and(|col| current_column(table.as_ref(), &quote, &cells, col)) {
                return Err(CURRENT_COLUMN.into());
            }
            if let Some(e) = table.as_ref().and_then(|t| t.part_of_company(metric, true)) {
                return Err(e);
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
                let lines = Lines::new(text);
                let table = Table::find(&lines, &quote);
                if in_row && !current_column(table.as_ref(), &quote, &cells, col) {
                    return Err(CURRENT_COLUMN.into());
                }
                // A lone per-share cell may be prose: only its group counts.
                if let Some(e) = table
                    .as_ref()
                    .and_then(|t| t.part_of_company(metric, in_row))
                {
                    return Err(e);
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

    /// The nearest year header line between the scale heading and the
    /// quote; a line with other words is not a header. It is `past_stop`
    /// when a line with no digits that is not a column label ("Supplemental
    /// table", "Revenues:") sits between it and the quote: such a header may
    /// reject the row's first column but never accepts a later one.
    fn header(&self) -> Option<Header> {
        let l = self.lines;
        let (top, bottom) = (l.line_of(self.heading_at), l.line_of(self.quote_at));
        let mut past_stop = false;
        for i in (top..=bottom).rev() {
            let line = l.line(i, self.quote_at);
            if line.is_empty() {
                continue;
            }
            if let Some(tokens) = header_years(line, i == top) {
                let (years, periods) = tokens.into_iter().unzip();
                return Some(Header {
                    line: i,
                    years,
                    periods,
                    past_stop,
                });
            }
            if !line.contains(|c: char| c.is_ascii_digit()) && !period_words_only(line) {
                past_stop = true;
            }
        }
        None
    }

    /// Whether the period labels of header line `i` (and of up to three
    /// label lines right above it) name the quarter first: "Three Months
    /// Ended ... Nine Months Ended", "2025 Q4 2026 Q4"; DUK puts the lengths
    /// and the dates on two lines above the years. A label line has words,
    /// all of them period or column labels, and is not a year header; the
    /// walk stops at any other line and at the scale heading, unless the
    /// heading line is the header (CVS puts its labels above it). `None`
    /// when no line names a period length; `Some(false)` when the lines
    /// disagree.
    fn quarter_first(&self, i: usize) -> Option<bool> {
        let firsts: Vec<bool> = self
            .period_lengths(i)
            .iter()
            .filter_map(|line| line.first().copied())
            .collect();
        match firsts.as_slice() {
            [] => None,
            [first, rest @ ..] => Some(*first && rest.iter().all(|&q| q)),
        }
    }

    /// Whether every period length the labels of header line `i` name is
    /// the quarter ("Three Months Ended September 30,"): none is a longer
    /// period, and at least one is named.
    fn quarter_only(&self, i: usize) -> bool {
        let lengths = self.period_lengths(i).concat();
        !lengths.is_empty() && lengths.iter().all(|&q| q)
    }

    /// For each label line `quarter_first` reads (top down, the header line
    /// last), whether each period length it names is the quarter.
    fn period_lengths(&self, i: usize) -> Vec<Vec<bool>> {
        static R: OnceLock<Regex> = OnceLock::new();
        static WORD: OnceLock<Regex> = OnceLock::new();
        let r = R.get_or_init(|| {
            Regex::new(r"(?i)\b(?:(three\s+months|thirteen\s+weeks|quarters?|q[1-4])|six\s+months|nine\s+months|twelve\s+months|years?|ytd|year-to-date)\b")
                .expect("valid built-in regex")
        });
        let word = WORD.get_or_init(|| Regex::new(r"[A-Za-z]").expect("valid built-in regex"));
        let l = self.lines;
        let top = l.line_of(self.heading_at);
        let floor = if i > top { top + 1 } else { 0 };
        let label_line = |s: &str| {
            word.is_match(&strip_parens(s))
                && period_words_only(s)
                && header_years(s, false).is_none()
        };
        let mut lines: Vec<&str> = (floor..i)
            .rev()
            .take(3)
            .map(|j| l.line(j, self.quote_at))
            .take_while(|s| label_line(s))
            .collect();
        lines.reverse();
        lines.push(l.line(i, self.quote_at));
        lines
            .iter()
            .map(|s| r.captures_iter(s).map(|c| c.get(1).is_some()).collect())
            .collect()
    }

    /// The column basis labels on the line right below header `h`, one per
    /// year with "% Change" columns left out: whether each column is pro
    /// forma ("Pro Forma Pro Forma % Change Actual Actual % Change" at
    /// CHTR, which reports the Cox merger's combined quarter first).
    /// `None` unless every word on the line is such a label.
    fn pro_forma_columns(&self, h: &Header) -> Option<Vec<bool>> {
        static LABEL: OnceLock<Regex> = OnceLock::new();
        let label = LABEL.get_or_init(|| {
            Regex::new(r"(?i)\b(pro\s+forma)\b(?:\s*\(\w{1,2}\))?|\b(?:actual|as\s+reported|reported)\b|(%\s*change\b|\bchange\b)")
                .expect("valid built-in regex")
        });
        let l = self.lines;
        let below = h.line + 1;
        if below >= l.line_of(self.quote_at) {
            return None;
        }
        let line = l.line(below, self.quote_at);
        if !label.replace_all(line, "").trim().is_empty() {
            return None;
        }
        let columns: Vec<bool> = label
            .captures_iter(line)
            .filter(|c| c.get(2).is_none())
            .map(|c| c.get(1).is_some())
            .collect();
        (columns.len() == h.years.len()).then_some(columns)
    }

    /// Why the quote's table or row group is about a part of the company,
    /// if it is. The table's head is the nearest period line above the
    /// quote (a year header or period labels), else its scale heading. Its
    /// title (up to six label lines right above the head) and the head may
    /// not name a part (`PARTS`: SPG's "Joint Venture Combined Statements",
    /// DUK's "Consolidating" tables, CVS's "... segment" and segment
    /// reconciliation, QCOM's "Segment Results"); the column labels right
    /// below the head may not name one either, but for "segment": AMD's
    /// "Segment and Disaggregated Revenue Information" lists segments as
    /// rows over the company's total. The label line right above the row,
    /// its group, may not name a part (`part_named` for a row: DUK's
    /// "Income (loss) from discontinued operations ..." over "Basic and
    /// Diluted $0.02"), nor per-share rows for a figure that is not one.
    /// Not `whole`: the group only, of a quote that starts its line.
    fn part_of_company(&self, m: Metric, whole: bool) -> Option<String> {
        static LENGTH: OnceLock<Regex> = OnceLock::new();
        let length = LENGTH.get_or_init(|| {
            Regex::new(r"(?i)\b(?:months|weeks|quarters?|years?|q[1-4]|ytd|year-to-date)\b")
                .expect("valid built-in regex")
        });
        let l = self.lines;
        let (top, at) = (l.line_of(self.heading_at), l.line_of(self.quote_at));
        if !whole && l.starts[at] != self.quote_at {
            return None;
        }
        let line = |i: usize| l.line(i, self.quote_at);
        if whole {
            // A lone "September 30, 2025" heads a block within the table.
            let period_line = |s: &str| {
                header_years(s, false).is_some()
                    || (label_only(s) && period_words_only(s) && length.is_match(&strip_parens(s)))
            };
            let head_above = |below: usize| {
                (top..below)
                    .rev()
                    .find(|&i| i == top || period_line(line(i)))
                    .unwrap_or(top)
            };
            // A period label right under a row ("Nine Months Ended" in CVS's
            // segment reconciliation) starts a block of the table above.
            let mut head = head_above(at);
            for _ in 0..4 {
                let block = head > top
                    && header_years(line(head), false).is_none()
                    && line(head - 1).len() <= PROSE_CHARS
                    && !label_only(line(head - 1));
                if !block {
                    break;
                }
                head = head_above(head);
            }
            // Up to a row of figures or a footnote under the table before;
            // a description or other sentence in between is skipped.
            let title = (0..head)
                .rev()
                .take(6)
                .map(line)
                .filter(|s| s.len() <= PROSE_CHARS && !sentence(s))
                .take_while(|s| label_only(s) && !footnote(s))
                .chain(Some(line(head)).filter(|s| label_only(s)))
                .find_map(|s| part_named(m, s, false));
            let columns = (head + 1..at)
                .map(line)
                .take_while(|s| label_only(s))
                .find_map(|s| part_named(m, s, false).filter(|w| !w.starts_with("segment")));
            if let Some(w) = title.or(columns) {
                return Some(format!("the table is about {w:?}, a part of the company"));
            }
        }
        let group = self.group()?;
        if let Some(w) = part_named(m, group, true) {
            return Some(format!(
                "the row's group {group:?} names {w:?}, a part of the company"
            ));
        }
        let per_share = [
            "per share",
            "per common share",
            "per diluted share",
            "earnings per",
        ];
        if !matches!(m, Metric::EpsGaap | Metric::EpsNonGaap)
            && per_share.iter().any(|w| has_word(group, w))
        {
            return Some(format!("the row's group {group:?} lists per-share figures"));
        }
        None
    }
}

impl<'a> Table<'a> {
    /// The label line right above a quote that starts its line, below the
    /// scale heading: the row's group ("BASIC AND DILUTED EARNINGS PER
    /// COMMON SHARE:", "Revenues:").
    fn group(&self) -> Option<&'a str> {
        let l = self.lines;
        let (top, at) = (l.line_of(self.heading_at), l.line_of(self.quote_at));
        if l.starts[at] != self.quote_at {
            return None;
        }
        at.checked_sub(1)
            .filter(|&i| i > top)
            .map(|i| l.line(i, self.quote_at))
            .filter(|s| group_label(s))
    }
}

/// A year header line found above a quote.
struct Header {
    line: usize,
    years: Vec<u32>,
    /// The period written just before each year, if any.
    periods: Vec<Option<Period>>,
    past_stop: bool,
}

/// A year token's period label: "September 26, 2026" is month 9, day 26;
/// "Q3 2026" and "Q3'26" are quarter 3.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
enum Period {
    Month(u32, u32),
    Quarter(u32),
}

/// The period label that ends `before` (the text just before a year).
fn period_before(before: &str) -> Option<Period> {
    static MONTH: OnceLock<Regex> = OnceLock::new();
    static QUARTER: OnceLock<Regex> = OnceLock::new();
    let month = MONTH.get_or_init(|| {
        Regex::new(r"(?i)\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)(?:uary|ruary|ch|il|e|y|ust|t|tember|ober|ember)?\b\.?(?:\s+(\d{1,2}))?\s*,?\s*$")
            .expect("valid built-in regex")
    });
    let quarter =
        QUARTER.get_or_init(|| Regex::new(r"(?i)\bq([1-4])\s*$").expect("valid built-in regex"));
    if let Some(c) = quarter.captures(before) {
        return Some(Period::Quarter(c[1].parse().ok()?));
    }
    let c = month.captures(before)?;
    let m = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| c[1].eq_ignore_ascii_case(m))?;
    let day = c.get(2).map_or(Some(0), |d| d.as_str().parse().ok())?;
    Some(Period::Month(m as u32 + 1, day))
}

/// For an untrusted header that starts with the latest year: whether that
/// first token is the current period. Within the first block (up to where
/// every year has appeared; the whole header for a single year), a latest
/// year listed once is; listed more than once, every such token needs a
/// period of one kind, and the first must be strictly the newest ("September
/// 26, 2026 June 27, 2026" passes; "June 30, 2026 September 30, 2026" and a
/// tie do not). No latest-year period anywhere in the header may be newer
/// than the first.
fn first_is_newest(h: &Header, distinct: &[u32]) -> bool {
    let latest = distinct[distinct.len() - 1];
    let end = if distinct.len() == 1 {
        h.years.len()
    } else {
        (1..=h.years.len())
            .find(|&i| distinct.iter().all(|y| h.years[..i].contains(y)))
            .unwrap_or(h.years.len())
    };
    let latest_periods: Vec<_> = (0..end)
        .filter(|&i| h.years[i] == latest)
        .map(|i| h.periods[i])
        .collect();
    let [first, rest @ ..] = latest_periods.as_slice() else {
        return false;
    };
    // Anywhere in the header, no latest-year period of the same kind is
    // newer than the first ("June 30, 2026 September 30, 2025 September 30,
    // 2026"); a tie (AMD's nine-month "September 26, 2026") is fine.
    let newer_later = first.is_some_and(|first| {
        (0..h.years.len()).any(|i| {
            h.years[i] == latest
                && h.periods[i].is_some_and(|p| {
                    std::mem::discriminant(&p) == std::mem::discriminant(&first) && p > first
                })
        })
    });
    if newer_later {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    let Some(first) = first else {
        return false;
    };
    rest.iter().all(|p| {
        p.is_some_and(|p| std::mem::discriminant(&p) == std::mem::discriminant(first) && p < *first)
    })
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
/// Months Ended September 30,", "For the Nine Months Ended", "GAAP
/// Non-GAAP", "Change", "Inc/ (Dec) Excluding Currency Translation",
/// "(Unaudited)".
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
        "for",
        "the",
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
fn header_years(line: &str, heading_line: bool) -> Option<Vec<(u32, Option<Period>)>> {
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
        .map(|c| {
            let whole = c.get(0)?;
            let year = match c.get(1) {
                Some(yy) => 2000 + yy.as_str().parse::<u32>().ok()?,
                None => whole.as_str().parse().ok()?,
            };
            Some((year, period_before(&line[..whole.start()])))
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

/// Whether cell `col` of the row's `cells` (spans in `quote`) is the
/// current period's column. Without a year header: the first cell. A
/// header is trusted when it is not past a stop line, its years form blocks
/// that each hold every year once, and there is one year per cell; then the
/// column is the latest year's place in the first block, which must be the
/// quarter when there are several. An untrusted header allows only the
/// first cell, when the header starts with the latest year, names its
/// periods if it repeats years, and lists that first period as the newest
/// (`first_is_newest`). A header whose
/// first period is not the quarter rejects any repeated-year row, and with
/// more cells than years a leading "(5)" may be a footnote, not a column.
fn current_column(
    table: Option<&Table>,
    quote: &str,
    cells: &[(usize, usize)],
    col: usize,
) -> bool {
    let Some((t, h)) = table.and_then(|t| Some((t, t.header()?))) else {
        return col == 0;
    };
    let years = &h.years;
    // Pro forma columns are never the reported figure; with some, the
    // reported quarter is the latest year among the others, when the
    // header names the quarter alone.
    if let Some(pro_forma) = t
        .pro_forma_columns(&h)
        .filter(|p| p.len() == cells.len() && p.contains(&true))
    {
        if pro_forma[col] || !t.quarter_only(h.line) {
            return false;
        }
        let reported: Vec<usize> = (0..years.len()).filter(|&i| !pro_forma[i]).collect();
        let latest = years.iter().max().copied();
        let current: Vec<usize> = reported
            .iter()
            .copied()
            .filter(|&i| Some(years[i]) == latest)
            .collect();
        return current == [col];
    }
    if col == 0 && cells.len() > years.len() {
        let (s, e) = cells[0];
        if e - s <= 2 && quote[..s].ends_with('(') && quote[e..].starts_with(')') {
            return false;
        }
    }
    let mut distinct = years.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let k = distinct.len();
    let latest = distinct[k - 1];
    let repeated = years.len() > k;
    let quarter_first = if repeated {
        t.quarter_first(h.line)
    } else {
        Some(true)
    };
    if quarter_first == Some(false) {
        return false;
    }
    let trusted = !h.past_stop
        && k >= 2
        && years.len() == cells.len()
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
        // Repeated years with no period labels at all, neither a period
        // length on the header or its label line ("Three Months Ended ...")
        // nor a month or quarter on a latest-year token: order unknown.
        let unlabelled = repeated
            && quarter_first.is_none()
            && (0..years.len()).all(|i| years[i] != latest || h.periods[i].is_none());
        col == 0 && years[0] == latest && !unlabelled && first_is_newest(&h, &distinct)
    }
}

fn quarter_words(q: u8) -> &'static [&'static str] {
    match q {
        1 => &["first quarter", "first-quarter", "q1", "1q"],
        2 => &["second quarter", "second-quarter", "q2", "2q"],
        3 => &["third quarter", "third-quarter", "q3", "3q"],
        4 => &["fourth quarter", "fourth-quarter", "q4", "4q"],
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
    // Non-breaking hyphens and dashes read as "-" ("third\u{2011}quarter").
    let lower: String = quote
        .to_ascii_lowercase()
        .chars()
        .map(|c| match c {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' => '-',
            c => c,
        })
        .collect();
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

/// The text has a revenue or sales label with a number soon after it, in
/// the same sentence ("Total revenues 8,893", "Revenue for the third
/// quarter was a record $8.9 billion"); boilerplate such as "projected
/// revenues, costs and plans" does not count. Erring towards a match only
/// keeps revenue required.
fn states_revenue(text: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)\b(?:revenues?|sales)\b[^.\d]{0,60}\d").expect("valid built-in regex")
    })
    .is_match(text)
}

/// The claimed quarter re-anchored to a line in the exhibit's opening that
/// names it and the year ("Qualcomm Announces Fourth Quarter and Fiscal 2026
/// Results"), for when the model's own quote does not verify. Only for a
/// claimed quarter: a full-year claim would match a dateline. A line needs
/// `HEADLINE_WORDS` words, so a file name ("ex99-pressreleaseq32026.htm")
/// is not a headline, and at most `HEADLINE_MAX_CHARS`, so a paragraph is
/// not one either (the period quote's numbers join the article's allow-list).
fn period_from_headline(text: &str, v: &Value) -> Option<FiscalPeriod> {
    let mut claim = v.as_object()?.clone();
    claim.get("quarter")?.as_u64()?;
    let opening: String = text.chars().take(HEADLINE_CHARS).collect();
    let mut headlines = opening.lines().map(collapse).filter(|l| {
        l.split_whitespace().count() >= HEADLINE_WORDS && l.chars().count() <= HEADLINE_MAX_CHARS
    });
    headlines.find_map(|line| {
        claim.insert("quote".into(), Value::String(line));
        verify_period(text, &Value::Object(claim.clone())).ok()
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

/// A rejected quote for a failure reason, so a failed run (which stores no
/// facts) shows what the model quoted: `[quote: "..."]`, cut at 120 chars.
fn cite(quote: &str) -> String {
    const CHARS: usize = 120;
    let quote = collapse(quote);
    let cut: String = quote.chars().take(CHARS).collect();
    let more = if quote.chars().count() > CHARS {
        "…"
    } else {
        ""
    };
    format!("[quote: {:?}]", format!("{cut}{more}"))
}

/// The verification half of `extract`, on an LLM reply.
pub fn from_reply(text: &str, v: &Value) -> Extracted {
    let mut metrics = Metrics::all_missing(NOT_STATED);
    let m = v.get("metrics");
    for metric in Metric::ALL {
        let field = match m.and_then(|m| m.get(metric.key())) {
            None | Some(Value::Null) => continue,
            Some(raw) => match serde_json::from_value::<RawFigure>(raw.clone()) {
                Ok(raw) if is_gaap(metric) && non_gaap_wording(&raw.quote) => Field::Missing {
                    reason: format!(
                        "unverified: a GAAP figure quoted with non-GAAP wording {}",
                        cite(&raw.quote)
                    ),
                },
                Ok(raw) => match verify_figure(text, metric, &raw) {
                    Ok(f) => Field::Value(f),
                    Err(e) => Field::Missing {
                        reason: format!("unverified: {e} {}", cite(&raw.quote)),
                    },
                },
                Err(e) => Field::Missing {
                    reason: format!("malformed: {e}"),
                },
            },
        };
        metrics.set(metric, field);
    }
    // "Not stated" waives revenue (`missing_required`), so it must mean the
    // release has none, not that the model skipped it: only when the model
    // saw the whole release and no revenue line with a number is in it.
    if matches!(&metrics.revenue, Field::Missing { reason } if reason == NOT_STATED)
        && (text.chars().count() > MAX_DOC_CHARS || states_revenue(text))
    {
        metrics.set(
            Metric::Revenue,
            Field::Missing {
                reason: "the release names a revenue figure the extraction did not give".into(),
            },
        );
    }
    let fiscal_period = match v.get("fiscal_period") {
        None | Some(Value::Null) => Field::Missing {
            reason: NOT_STATED.into(),
        },
        Some(p) => match verify_period(text, p) {
            Ok(p) => Field::Value(p),
            Err(e) => match period_from_headline(text, p) {
                Some(p) => Field::Value(p),
                None => Field::Missing {
                    reason: format!("unverified: {e}"),
                },
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
    fn revenue_is_not_stated_only_when_the_release_has_none() {
        let none = json!({"metrics": {}});
        let reason = |t: &str| match from_reply(t, &none).metrics.revenue {
            Field::Missing { reason } => reason,
            Field::Value(_) => panic!("no figure was given"),
        };
        // AIG-style: no revenue line; boilerplate without a figure.
        let aig = "AIG reports third quarter 2026 results. Net income per diluted share $1.92. \
                   Statements about projected revenues, costs and plans are forward-looking.";
        assert_eq!(reason(aig), NOT_STATED);
        // The release has a revenue figure the model left out: still required.
        assert_ne!(
            reason("(In millions) Total revenues 8,893 9,008"),
            NOT_STATED
        );
        assert_ne!(reason("Revenue of $102.5 billion, up 8%."), NOT_STATED);
        assert_ne!(
            reason("Revenue for the third quarter was a record $8.9 billion."),
            NOT_STATED
        );
        assert_ne!(reason("Net sales\n$4,210"), NOT_STATED);
        // The model saw only the opening: revenue may sit further down.
        let long = format!("{aig} {}", "x ".repeat(MAX_DOC_CHARS));
        assert_ne!(reason(&long), NOT_STATED);
    }

    #[test]
    fn a_dollar_amount_matches_a_table_cell_printed_without_the_sign() {
        // Booking's statement prints "$" on the first row only; the model
        // still writes the amount as "$8,893".
        let t = "(In millions) Merchant revenues $ 6,998 $ 6,131\nTotal revenues 8,893 9,008 21,777 20,568";
        let f = verify_figure(
            t,
            Metric::Revenue,
            &raw(
                "$8,893",
                "millions",
                false,
                "Total revenues 8,893 9,008 21,777 20,568",
            ),
        )
        .unwrap();
        assert_eq!(f.value, 8_893e6);
        // The sign is only dropped to find the cell: the column rule still holds.
        let e = verify_figure(
            t,
            Metric::Revenue,
            &raw(
                "$9,008",
                "millions",
                false,
                "Total revenues 8,893 9,008 21,777 20,568",
            ),
        );
        assert!(e.is_err());
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
    fn rows_naming_a_part_of_the_company_are_rejected() {
        // DUK's statement: the noncontrolling interests' share, a "Less:"
        // row and the discontinued operations' EPS are not the company's.
        let periods =
            "Three Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025";
        let duk = format!("DUKE ENERGY CORPORATION\nCONDENSED CONSOLIDATED STATEMENTS OF OPERATIONS\n(Unaudited)\n(In millions, except per share amounts)\n{periods}\nOperating Revenues\nTotal operating revenues 9,104 8,542 25,874 24,299\nNet Income 1,650 1,454 4,372 3,865\nLess: Net Income Attributable to NCI 85 33 165 81\nNet Income Attributable to Duke Energy Corporation 1,565 1,421 4,207 3,784\nNet Income Available to Duke Energy Corporation Common Stockholders $1,551 $1,407 $4,164 $3,743\nEarnings Per Share – Basic and Diluted\nIncome from continuing operations available to Duke Energy Corporation common stockholders\nBasic and Diluted $1.99 $1.81 $5.32 $4.81\nIncome (loss) from discontinued operations attributable to Duke Energy Corporation common stockholders\nBasic and Diluted $ — $ — $0.02 $ —\nNet income available to Duke Energy Corporation common stockholders\nBasic and Diluted $1.99 $1.81 $5.34 $4.81");
        let ok = |t: &str, m: Metric, amount: &str, scale: &str, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, false, q));
            assert!(r.is_ok(), "{m:?} {q:?}: {r:?}");
        };
        let bad = |t: &str, m: Metric, amount: &str, scale: &str, negative: bool, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, negative, q));
            assert!(
                r.unwrap_err().contains("part of the company"),
                "{m:?} {q:?}"
            );
        };
        ok(
            &duk,
            Metric::Revenue,
            "9,104",
            "millions",
            "Total operating revenues 9,104 8,542 25,874 24,299",
        );
        ok(
            &duk,
            Metric::NetIncome,
            "1,650",
            "millions",
            "Net Income 1,650 1,454 4,372 3,865",
        );
        ok(
            &duk,
            Metric::NetIncome,
            "1,565",
            "millions",
            "Net Income Attributable to Duke Energy Corporation 1,565 1,421 4,207 3,784",
        );
        ok(&duk, Metric::NetIncome, "$1,551", "millions", "Net Income Available to Duke Energy Corporation Common Stockholders $1,551 $1,407 $4,164 $3,743");
        ok(
            &duk,
            Metric::EpsGaap,
            "$1.99",
            "units",
            "Basic and Diluted $1.99 $1.81 $5.32 $4.81",
        );
        ok(
            &duk,
            Metric::EpsGaap,
            "$1.99",
            "units",
            "Basic and Diluted $1.99 $1.81 $5.34 $4.81",
        );
        bad(
            &duk,
            Metric::NetIncome,
            "85",
            "millions",
            false,
            "Less: Net Income Attributable to NCI 85 33 165 81",
        );
        bad(
            &duk,
            Metric::EpsGaap,
            "$0.02",
            "units",
            false,
            "Basic and Diluted $ — $ — $0.02 $ —",
        );
        // SO and SPG: the noncontrolling interests' net income.
        let so = format!("(In millions)\n{periods}\nNet income attributable to noncontrolling interests 28 (4) 32 24\nNet income attributable to Southern Company $2,046 $1,711 $4,577 $3,925");
        bad(
            &so,
            Metric::NetIncome,
            "28",
            "millions",
            false,
            "Net income attributable to noncontrolling interests 28 (4) 32 24",
        );
        ok(
            &so,
            Metric::NetIncome,
            "$2,046",
            "millions",
            "Net income attributable to Southern Company $2,046 $1,711 $4,577 $3,925",
        );
        // SPG: "Our Share" and third parties' share of a joint venture's net
        // income; a per-share row under its group label is not net income.
        let spg = format!("(Dollars in thousands, except per share amounts)\n{periods}\nNet loss (gain) attributable to noncontrolling interest holders in properties 20,716 (1,231) 8,695 34\nThird-Party Investors’ Share of Net Income $257,552 $110,051 $516,134 $314,298\nOur Share of Net Income 247,391 107,526 495,157 302,181\nNET INCOME ATTRIBUTABLE TO COMMON STOCKHOLDERS $498,959 $606,174 $1,461,668 $1,576,006\nBASIC AND DILUTED EARNINGS PER COMMON SHARE:\nNet income attributable to common stockholders $1.54 $1.86 $4.51 $4.83");
        for (amount, q) in [
            ("20,716", "Net loss (gain) attributable to noncontrolling interest holders in properties 20,716 (1,231) 8,695 34"),
            ("$257,552", "Third-Party Investors’ Share of Net Income $257,552 $110,051 $516,134 $314,298"),
            ("247,391", "Our Share of Net Income 247,391 107,526 495,157 302,181"),
            ("$1.54", "Net income attributable to common stockholders $1.54 $1.86 $4.51 $4.83"),
        ] {
            let r = verify_figure(&spg, Metric::NetIncome, &raw(amount, "thousands", false, q));
            assert!(r.is_err(), "{q}: {r:?}");
        }
        ok(&spg, Metric::NetIncome, "$498,959", "thousands", "NET INCOME ATTRIBUTABLE TO COMMON STOCKHOLDERS $498,959 $606,174 $1,461,668 $1,576,006");
        // AMD: a segment's revenue in prose.
        let t = "•Data Center segment revenue was a record $8.5 billion, up 97% year-over-year.";
        bad(t, Metric::Revenue, "$8.5 billion", "units", false, t);
    }

    #[test]
    fn other_measures_are_not_eps_or_net_income() {
        let periods =
            "Three Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025";
        let ok = |t: &str, m: Metric, amount: &str, scale: &str, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, false, q));
            assert!(r.is_ok(), "{m:?} {q:?}: {r:?}");
        };
        let bad = |t: &str, m: Metric, amount: &str, scale: &str, negative: bool, q: &str| {
            for m in match m {
                Metric::EpsGaap => vec![Metric::EpsGaap, Metric::EpsNonGaap],
                m => vec![m],
            } {
                let r = verify_figure(t, m, &raw(amount, scale, negative, q));
                assert!(r.is_err(), "{m:?} {q:?}: {r:?}");
            }
        };
        // SPG's funds from operations per share, next to its EPS.
        let spg = format!("(Amounts in thousands, except per share amounts)\n{periods}\nDiluted net income per share $1.54 $1.86 $4.51 $4.83\nDiluted FFO per share(1) $3.09 $3.25 $9.12 $9.07\nReal Estate FFO per share(1) $3.37 $3.22 $9.82 $9.24\nBasic and Diluted Earnings per Common Share (EPS) $1.54 $1.86 $4.51 $4.83\nBasic and Diluted Real Estate FFO per Share $3.37 $3.22 $9.82 $9.24\nBasic and Diluted FFO per Share (FFOPS) $3.09 $3.25 $9.12 $9.07\nEstimated FFO per diluted share $12.60 $12.70");
        for (amount, q) in [
            ("$3.09", "Diluted FFO per share(1) $3.09 $3.25 $9.12 $9.07"),
            (
                "$3.37",
                "Real Estate FFO per share(1) $3.37 $3.22 $9.82 $9.24",
            ),
            (
                "$3.37",
                "Basic and Diluted Real Estate FFO per Share $3.37 $3.22 $9.82 $9.24",
            ),
            (
                "$3.09",
                "Basic and Diluted FFO per Share (FFOPS) $3.09 $3.25 $9.12 $9.07",
            ),
            ("$12.60", "Estimated FFO per diluted share $12.60 $12.70"),
        ] {
            bad(&spg, Metric::EpsGaap, amount, "units", false, q);
        }
        ok(
            &spg,
            Metric::EpsGaap,
            "$1.54",
            "units",
            "Diluted net income per share $1.54 $1.86 $4.51 $4.83",
        );
        ok(
            &spg,
            Metric::EpsGaap,
            "$1.54",
            "units",
            "Basic and Diluted Earnings per Common Share (EPS) $1.54 $1.86 $4.51 $4.83",
        );
        for t in [
            "AFFO per diluted share was $1.10 for the quarter.",
            "Funds from operations per diluted share was $2.05 for the quarter.",
            "Core FFO of $1.42 per diluted share for the quarter.",
        ] {
            let amount = t.split_whitespace().find(|w| w.starts_with('$')).unwrap();
            bad(t, Metric::EpsGaap, amount, "units", false, t);
        }
        let t = "Diluted earnings per share were $1.42 for the quarter.";
        ok(t, Metric::EpsGaap, "$1.42", "units", t);
        // AMGN's and DUK's reconciliations: adjustments to net income are
        // not net income.
        let amgn = format!("(In millions, except per-share data)\n{periods}\nGAAP net income $3,864 $3,216 $8,058 $6,378\nAdjustments to net income:\nAdjustments to income before income taxes, net of the income tax effect (370) (103) 1,691 2,637\nTotal adjustments to net income (344) (161) 1,684 2,584");
        bad(
            &amgn,
            Metric::NetIncome,
            "(344)",
            "millions",
            true,
            "Total adjustments to net income (344) (161) 1,684 2,584",
        );
        ok(
            &amgn,
            Metric::NetIncome,
            "$3,864",
            "millions",
            "GAAP net income $3,864 $3,216 $8,058 $6,378",
        );
        let duk = "(In millions)\nNine Months Ended September 30,\n2026 2025\nCASH FLOWS FROM OPERATING ACTIVITIES\nNet Income $4,372 $3,865\nAdjustments to reconcile net income to net cash provided by operating activities 3,517 4,807";
        bad(duk, Metric::NetIncome, "3,517", "millions", false, "Adjustments to reconcile net income to net cash provided by operating activities 3,517 4,807");
        ok(
            duk,
            Metric::NetIncome,
            "$4,372",
            "millions",
            "Net Income $4,372 $3,865",
        );
        let t = "(In millions)\n2026 2025\nReconciliation of net income to adjusted EBITDA: net income $2,717 $2,500";
        let q = "Reconciliation of net income to adjusted EBITDA: net income $2,717 $2,500";
        bad(t, Metric::NetIncome, "$2,717", "millions", false, q);
    }

    #[test]
    fn company_rows_next_to_part_rows_still_verify() {
        let hdr = "ACME CORPORATION\nCONDENSED CONSOLIDATED STATEMENTS OF INCOME\n(Unaudited)\n(In millions, except per share amounts)\nThree Months Ended September 30,\n2026 2025";
        let ok = |t: &str, m: Metric, amount: &str, scale: &str, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, false, q));
            assert!(r.is_ok(), "{m:?} {q:?} in {t:?}: {r:?}");
        };
        // A row of dashes or small negatives above the total is a row, not
        // the total's group label.
        for row in [
            "Loss from discontinued operations, net of tax (5) (3)",
            "Loss from discontinued operations, net of tax (50) (30)",
            "Income (loss) from discontinued operations, net of tax — —",
        ] {
            let t = format!("{hdr}\nNet sales $5,000 $4,500\nIncome from continuing operations 600 500\n{row}\nNet income $600 $500");
            ok(
                &t,
                Metric::NetIncome,
                "$600",
                "millions",
                "Net income $600 $500",
            );
        }
        for row in [
            "Less: Net income attributable to noncontrolling interests — —",
            "Net income attributable to noncontrolling interests $ — $ —",
            "Net loss attributable to noncontrolling interests (2) (1)",
            "Net loss attributable to noncontrolling interests (12) (9)",
        ] {
            let t = format!("{hdr}\nNet sales $5,000 $4,500\nNet income 600 500\n{row}\nNet income attributable to Acme Corporation $590 $492");
            ok(
                &t,
                Metric::NetIncome,
                "$590",
                "millions",
                "Net income attributable to Acme Corporation $590 $492",
            );
        }
        for row in [
            "Intersegment eliminations (5) (3)",
            "Intersegment eliminations — —",
            "Corporate and eliminations (5) (3)",
        ] {
            let t = format!("{hdr}\nCloud $2,000 $1,800\nServices 3,005 2,703\n{row}\nTotal revenues $5,000 $4,500");
            ok(
                &t,
                Metric::Revenue,
                "$5,000",
                "millions",
                "Total revenues $5,000 $4,500",
            );
        }
        let t = format!("{hdr}\nNet income $600 $500\nEarnings per share:\nDiscontinued operations — —\nDiluted – as reported $1.20 $1.00");
        ok(
            &t,
            Metric::EpsGaap,
            "$1.20",
            "units",
            "Diluted – as reported $1.20 $1.00",
        );
        // Nor do such rows, or footnotes, at the end of the table before
        // make the next one about a part.
        let stmt = format!(
            "{hdr}\nNet sales $5,000 $4,500\nNet income attributable to Acme Corporation $590 $492"
        );
        for end in [
            "Intersegment eliminations (5) (3)",
            "Noncontrolling interests — —",
            "Total segment revenue 3,000 2,800\n(1) Includes unconsolidated joint ventures.",
            "Total segment revenue 3,000 2,800\nNote: excludes noncontrolling interests.",
            "Total segment revenue 3,000 2,800\nSegment results are discussed above.",
            "Total segment revenue 3,000 2,800\n* Joint venture revenue at our share.",
        ] {
            let t = format!("Segment Information\n(In millions)\nThree Months Ended September 30,\n2026 2025\nCloud 1,000 900\n{end}\n{stmt}");
            ok(
                &t,
                Metric::Revenue,
                "$5,000",
                "millions",
                "Net sales $5,000 $4,500",
            );
            ok(
                &t,
                Metric::NetIncome,
                "$590",
                "millions",
                "Net income attributable to Acme Corporation $590 $492",
            );
        }
        // Prose: a part word elsewhere in the sentence does not qualify the
        // figure.
        for (m, amount, t) in [
            (
                Metric::Revenue,
                "$13.4 billion",
                "Across our three segments, third quarter revenue was $13.4 billion, up 5%.",
            ),
            (
                Metric::NetIncome,
                "$1.2 billion",
                "Net income, including discontinued operations, was $1.2 billion.",
            ),
            (
                Metric::Revenue,
                "$13.4 million",
                "Revenue, excluding the share of results from affiliates, was $13.4 million.",
            ),
            (
                Metric::Revenue,
                "$13.4 million",
                "Plus Therapeutics reported revenue of $13.4 million for the quarter.",
            ),
        ] {
            ok(t, m, amount, "units", t);
        }
    }

    #[test]
    fn other_wordings_of_a_part_are_rejected() {
        let hdr = "ACME CORPORATION\nCONDENSED CONSOLIDATED STATEMENTS OF INCOME\n(Unaudited)\n(In millions, except per share amounts)\nThree Months Ended September 30,\n2026 2025";
        let bad = |t: &str, m: Metric, amount: &str, scale: &str, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, false, q));
            assert!(r.is_err(), "{m:?} {q:?}: {r:?}");
        };
        for label in [
            "Net income attributable to minority shareholders",
            "Net income attributable to minority interest",
            "Net income attributable to non\u{2011}controlling interests",
            "Net income attributable to non\u{2010}controlling interests",
            "Net income attributable to non controlling interests",
            "Equity in net income of affiliates",
            "Net income attributable to limited partners",
            "Parent company only net income",
            "Net income of subsidiaries",
            "Pro forma net income",
        ] {
            let q = format!("{label} $10 $8");
            let t = format!("{hdr}\nNet sales $5,000 $4,500\nNet income 600 500\n{q}");
            bad(&t, Metric::NetIncome, "$10", "millions", &q);
        }
        for label in [
            "Pro forma diluted earnings per share",
            "Pro-forma diluted EPS",
        ] {
            let q = format!("{label} $1.20 $1.00");
            let t = format!("{hdr}\nNet sales $5,000 $4,500\n{q}");
            bad(&t, Metric::EpsGaap, "$1.20", "units", &q);
        }
        // "Subsidiaries" in a table title or group, but not "... and
        // Subsidiaries", the company's own statements.
        let t = "Condensed Statements of Income of Subsidiaries\n(In millions)\nThree Months Ended September 30,\n2026 2025\nTotal revenues $1,000 $900";
        bad(
            t,
            Metric::Revenue,
            "$1,000",
            "millions",
            "Total revenues $1,000 $900",
        );
        let t = "(In millions)\nThree Months Ended September 30,\n2026 2025\nTotal revenues $5,000 $4,500\nSubsidiaries:\nTotal revenues $1,000 $900";
        bad(
            t,
            Metric::Revenue,
            "$1,000",
            "millions",
            "Total revenues $1,000 $900",
        );
        let t = "ACME CORPORATION AND SUBSIDIARIES\nCONDENSED CONSOLIDATED STATEMENTS OF INCOME\n(In millions)\nThree Months Ended September 30,\n2026 2025\nTotal revenues $5,000 $4,500";
        let r = verify_figure(
            t,
            Metric::Revenue,
            &raw("$5,000", "millions", false, "Total revenues $5,000 $4,500"),
        );
        assert!(r.is_ok(), "{r:?}");
        // Prose that names the part on the metric itself stays rejected.
        for (m, amount, t) in [
            (Metric::Revenue, "$8.5 billion", "•Data Center segment revenue was a record $8.5 billion, up 97% year-over-year."),
            (Metric::Revenue, "$40 million", "Revenue from discontinued operations was $40 million in the quarter."),
            (Metric::NetIncome, "$10 million", "Net income attributable to noncontrolling interests was $10 million for the quarter."),
            (Metric::Revenue, "$1.4 billion", "Our share of revenue was $1.4 billion for the quarter."),
        ] {
            bad(t, m, amount, "units", t);
        }
    }

    #[test]
    fn titles_read_past_sentences_but_not_footnotes() {
        let periods = "Three Months Ended September 30,\n2026 2025";
        let q = "Total revenues $1,000 $900";
        let verify = |top: &str| {
            let t = format!("(In millions)\n{top}\n{periods}\n{q}");
            verify_figure(&t, Metric::Revenue, &raw("$1,000", "millions", false, q))
        };
        // The company's own statements: "... and (its) (consolidated |
        // wholly-owned) subsidiaries".
        for title in [
            "ACME CORPORATION AND CONSOLIDATED SUBSIDIARIES",
            "ACME CORPORATION AND ITS CONSOLIDATED SUBSIDIARIES",
            "ACME CORPORATION AND WHOLLY-OWNED SUBSIDIARIES",
            "Acme Corporation and its wholly owned subsidiaries",
            "ACME CORPORATION & CONSOLIDATED SUBSIDIARIES",
        ] {
            let r = verify(&format!(
                "{title}\nCONDENSED CONSOLIDATED STATEMENTS OF INCOME"
            ));
            assert!(r.is_ok(), "{title}: {r:?}");
        }
        let t = format!("(In millions)\n{periods}\nNet income attributable to Acme Corporation and its consolidated subsidiaries $590 $492");
        let q2 = "Net income attributable to Acme Corporation and its consolidated subsidiaries $590 $492";
        assert!(verify_figure(&t, Metric::NetIncome, &raw("$590", "millions", false, q2)).is_ok());
        let t = "Net income of Acme and its consolidated subsidiaries was $590 million.";
        assert!(verify_figure(
            t,
            Metric::NetIncome,
            &raw("$590 million", "units", false, t)
        )
        .is_ok());
        let t =
            "Acme Corporation and its consolidated subsidiaries reported revenue of $5.0 billion.";
        let r = verify_figure(t, Metric::Revenue, &raw("$5.0 billion", "units", false, t));
        assert!(r.is_ok(), "{r:?}");
        // A short sentence under a title is skipped, not the title's end;
        // a title may start with "Note", "*" or "(a)", and end in footnote
        // markers or a dash.
        for top in [
            "Health Services segment\nProvides pharmacy benefit services.",
            "Segment Information\n(Unaudited).",
            "Note: Segment Information",
            "* Segment Results",
            "(a) Joint Venture Statements of Operations",
            "Segment Results (1) (2)",
            "Segment Results –",
        ] {
            let r = verify(top);
            assert!(r.unwrap_err().contains("part of the company"), "{top}");
        }
        // A footnote that reads as one still ends the table before.
        for note in [
            "(1) Includes unconsolidated joint ventures.",
            "Note: excludes noncontrolling interests.",
            "* Joint venture revenue at our share.",
            "(a) Amounts for our segments exclude intersegment eliminations and corporate items",
            // Without a period, unless the words read as a title.
            "(1) Includes unconsolidated joint ventures",
            "* Excludes noncontrolling interests",
            "Note: excludes noncontrolling interests",
            "Notes: segment totals exclude eliminations",
            "(a) Segment revenue",
        ] {
            let r = verify(&format!("Segment Information\nCloud 1,000 900\n{note}\nCONDENSED CONSOLIDATED STATEMENTS OF INCOME"));
            assert!(r.is_ok(), "{note}: {r:?}");
        }
    }

    #[test]
    fn the_row_above_is_figures_even_when_its_cells_count_up() {
        // "(1) (2)" right above the row is a row of small losses, not
        // footnote markers.
        let hdr = "ACME CORPORATION\nCONDENSED CONSOLIDATED STATEMENTS OF INCOME\n(In millions, except per share amounts)\nThree Months Ended September 30,\n2026 2025";
        let q = "Net income attributable to Acme Corporation $590 $492";
        for row in [
            "Net loss attributable to noncontrolling interests (1) (2)",
            "Net loss attributable to noncontrolling interests (1) (2) (3) (4)",
            "Net loss attributable to noncontrolling interests (1)(2)",
        ] {
            let t = format!("{hdr}\nNet income 600 500\n{row}\n{q}");
            let r = verify_figure(&t, Metric::NetIncome, &raw("$590", "millions", false, q));
            assert!(r.is_ok(), "{row}: {r:?}");
        }
    }

    #[test]
    fn tables_of_a_part_of_the_company_are_rejected() {
        let periods =
            "Three Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025";
        let bad = |t: &str, m: Metric, amount: &str, scale: &str, negative: bool, q: &str| {
            let r = verify_figure(t, m, &raw(amount, scale, negative, q));
            assert!(
                r.unwrap_err().contains("part of the company"),
                "{m:?} {q:?}"
            );
        };
        // SPG's joint ventures' combined statement, and its pro-rata table
        // whose columns are the noncontrolling interests and "Our Share".
        let jv = format!("EARNINGS RELEASE\nSimon Property Group, Inc.\nUnaudited Joint Venture Combined Statements of Operations\n(Dollars in thousands)\n{periods}\nREVENUE:\nLease income $954,138 $758,148 $2,813,583 $2,265,844\nTotal revenue 1,056,910 868,249 3,125,244 2,582,952\nNET INCOME $504,943 $217,577 $1,011,291 $616,479");
        bad(
            &jv,
            Metric::Revenue,
            "1,056,910",
            "thousands",
            false,
            "Total revenue 1,056,910 868,249 3,125,244 2,582,952",
        );
        bad(
            &jv,
            Metric::NetIncome,
            "$504,943",
            "thousands",
            false,
            "NET INCOME $504,943 $217,577 $1,011,291 $616,479",
        );
        let pro_rata = "Table of Contents\nNON-GAAP PRO-RATA FINANCIAL INFORMATION\n(In thousands)\nThree Months Ended September 30, 2026 Three Months Ended September 30, 2025\nNoncontrolling Interests (1) Our Share of Joint Ventures Noncontrolling Interests (1) Our Share of Joint Ventures\nREVENUE:\nLease income $(28,490) $440,899 $(17,604) $357,737\nTotal revenue (30,716) 491,726 (19,776) 413,656";
        bad(
            pro_rata,
            Metric::Revenue,
            "(30,716)",
            "thousands",
            true,
            "Total revenue (30,716) 491,726 (19,776) 413,656",
        );
        // DUK's consolidating statement and segment income tables: by-entity
        // columns, the first a segment or a subsidiary.
        let duk = "DUKE ENERGY CORPORATION\nCONDENSED CONSOLIDATING STATEMENTS OF OPERATIONS\n(Unaudited)\nThree Months Ended September 30, 2026\n(In millions) Electric Utilities and Infrastructure Gas Utilities and Infrastructure Other Eliminations/Adjustments Duke Energy\nOperating Revenues\nRegulated electric $8,691 $ — $ — $(14) $8,677\nTotal operating revenues 8,747 390 41 (74) 9,104";
        bad(
            duk,
            Metric::Revenue,
            "8,747",
            "millions",
            false,
            "Total operating revenues 8,747 390 41 (74) 9,104",
        );
        let duk = "22\nELECTRIC UTILITIES AND INFRASTRUCTURE\nCONDENSED CONSOLIDATING SEGMENT INCOME\n(Unaudited)\nThree Months Ended September 30, 2026\n(In millions) Duke Energy Carolinas Duke Energy Progress Duke Energy Florida Duke Energy Ohio(a) Duke Energy Indiana Eliminations/ Other Electric Utilities and Infrastructure\nOperating Revenues $3,055 $2,242 $1,867 $591 $1,108 $(116) $8,747";
        bad(
            duk,
            Metric::Revenue,
            "$3,055",
            "millions",
            false,
            "Operating Revenues $3,055 $2,242 $1,867 $591 $1,108 $(116) $8,747",
        );
        // CVS's segment reconciliation, under the previous table's heading.
        let cvs = "In millions, except per share amounts Total Company Per Common Share Total Company Per Common Share\nAdjusted income attributable to CVS Health (2) $9,313 $7.24 $7,185 $5.66\n14\nSupplemental Information\n(Unaudited)\nThe following are reconciliations of financial measures of the Company’s segments to the consolidated totals:\nIn millions Health Care Benefits Health Services Pharmacy & Consumer Wellness Corporate/ Other Intersegment Eliminations (a) Consolidated Totals\nThree Months Ended\nSeptember 30, 2026\nTotal revenues $36,855 $53,964 $35,498 $205 $(18,123) $108,399";
        bad(
            cvs,
            Metric::Revenue,
            "$36,855",
            "millions",
            false,
            "Total revenues $36,855 $53,964 $35,498 $205 $(18,123) $108,399",
        );
        // CVS's own segment tables, a description between title and
        // periods, and its reconciliation's later blocks, which do not
        // repeat the column labels.
        let cvs_segment = "•Days claims payable were 41.9 days as of September 30, 2026, an increase of 0.2 days compared to June 30, 2026.\n3\nHealth Services segment\nThe Health Services segment provides a full range of pharmacy benefit management solutions, delivers health care services in its medical clinics, virtually, and in the home, and offers specialty pharmacy, infusion and other services.\nThree Months Ended September 30, Nine Months Ended September 30,\nIn millions 2026 2025 Change 2026 2025 Change\nTotal revenues $53,964 $49,266 $4,698 $153,996 $139,181 $14,815";
        let t = format!("(In millions, except per share amounts)\n{cvs_segment}");
        bad(
            &t,
            Metric::Revenue,
            "$53,964",
            "millions",
            false,
            "Total revenues $53,964 $49,266 $4,698 $153,996 $139,181 $14,815",
        );
        let t = format!("{cvs}\nAdjusted operating income (loss) (1) 912 2,106 1,635 (459) — 4,194\nSeptember 30, 2025\nTotal revenues $35,993 $49,266 $36,214 $133 $(18,735) $102,871");
        bad(
            &t,
            Metric::Revenue,
            "$35,993",
            "millions",
            false,
            "Total revenues $35,993 $49,266 $36,214 $133 $(18,735) $102,871",
        );
        let t = format!("{t}\nAdjusted operating income (loss) (1) 314 2,050 1,478 (383) — 3,459\nNine Months Ended\nSeptember 30, 2026\nTotal revenues $110,364 $153,996 $101,303 $478 $(51,220) $314,921");
        bad(
            &t,
            Metric::Revenue,
            "$110,364",
            "millions",
            false,
            "Total revenues $110,364 $153,996 $101,303 $478 $(51,220) $314,921",
        );
        // QCOM's segment results.
        let qcom = "Segment Results\nFourth Quarter\nQCT QTL\n(in millions, except percentages) Q4 Fiscal 2026 Q4 Fiscal 2025 Change Q4 Fiscal 2026 Q4 Fiscal 2025 Change\nRevenues $9,015 $9,821 (8%) $1,304 $1,409 (7%)";
        bad(
            qcom,
            Metric::Revenue,
            "$9,015",
            "millions",
            false,
            "Revenues $9,015 $9,821 (8%) $1,304 $1,409 (7%)",
        );
        // The company's own statements still verify.
        let spg = format!("EARNINGS RELEASE\nSimon Property Group, Inc.\nUnaudited Consolidated Statements of Operations\n(Dollars in thousands, except per share amounts)\n{periods}\nREVENUE:\nLease income $1,681,393 $1,452,930 $4,969,633 $4,199,812\nTotal revenue 1,814,556 1,601,572 5,362,247 4,573,043");
        let q = "Total revenue 1,814,556 1,601,572 5,362,247 4,573,043";
        let f = verify_figure(
            &spg,
            Metric::Revenue,
            &raw("1,814,556", "thousands", false, q),
        );
        assert_eq!(f.unwrap().value, 1.814556e9);
        let qcom = "Reconciliations of GAAP Results to Non-GAAP Results\nGAAP to Non-GAAP Reconciliation\n(in millions, except per share data and percentages) GAAP Results Less QSI Less Share-Based Compensation Less Other Items1, 2 Non-GAAP Results\nQ4 Fiscal 2026\nRevenues $10,322 $— $— ($120) $10,442";
        let q = "Revenues $10,322 $— $— ($120) $10,442";
        let f = verify_figure(qcom, Metric::Revenue, &raw("$10,322", "millions", false, q));
        assert_eq!(f.unwrap().value, 10.322e9);
        // AMD lists its segments as rows over the company's total.
        let amd = "(In millions, except per share amounts)\nCash and cash equivalents $7,404 $4,808 $7,404 $4,808\n10\nADVANCED MICRO DEVICES, INC.\nSELECTED CORPORATE DATA\n(Millions) (Unaudited)\nThree Months Ended Nine Months Ended\nSeptember 26, 2026 June 27, 2026 September 27, 2025 September 26, 2026 September 27, 2025\nSegment and Disaggregated Revenue Information (1)\nNet Revenue:\nData Center Segment $8,546 $6,718 $4,341 $21,039 $11,255\nClient and Gaming Segment\nClient 2,949 3,062 2,750 8,896 7,543\nGaming 773 779 1,298 2,272 3,067\nTotal Client and Gaming 3,722 3,841 4,048 11,168 10,610\nEmbedded Segment 1,109 977 857 2,959 2,504\nTotal net revenue $13,377 $11,536 $9,246 $35,166 $24,369";
        let q = "Total net revenue $13,377 $11,536 $9,246 $35,166 $24,369";
        let f = verify_figure(amd, Metric::Revenue, &raw("$13,377", "millions", false, q));
        assert_eq!(f.unwrap().value, 13.377e9);
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
        let cvs = "Three Months Ended September 30, Year Ending December 31,\nIn billions, except per share amounts 2026 2025 2026 Projected\nTotal revenues $108.4 $102.9 At least $420.0\nConsolidated third quarter results\nThree Months Ended September 30, Nine Months Ended September 30,\nIn millions, except per share amounts 2026 2025 Change 2026 2025 Change\nTotal revenues $108,399 $102,871 $5,528 $314,921 $296,374 $18,547";
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
        // More cells than header years: a leading "(5)" may be a footnote,
        // so it is not read as the current column.
        let t = "(In millions)\n2026 2025\nNet loss for the period (5) (62) (37)";
        let q = "Net loss for the period (5) (62) (37)";
        for a in ["(5)", "(62)", "(37)"] {
            let r = verify_figure(t, Metric::NetIncome, &raw(a, "millions", true, q));
            assert!(r.is_err(), "{a}: {r:?}");
        }
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
    fn a_repeated_latest_year_needs_its_periods_newest_first() {
        // AMD's statement: two 2026 quarters, newest first, then 2025.
        let t = "(In millions, except per share amounts)\nThree Months Ended Nine Months Ended\nSeptember 26, 2026 June 27, 2026 September 27, 2025 September 26, 2026 September 27, 2025\nNet revenue $13,377 $11,536 $9,246 $35,166 $24,369";
        let q = "Net revenue $13,377 $11,536 $9,246 $35,166 $24,369";
        let f = verify_figure(t, Metric::Revenue, &raw("$13,377", "millions", false, q));
        assert_eq!(f.unwrap().value, 13.377e9);
        assert!(verify_figure(t, Metric::Revenue, &raw("$11,536", "millions", false, q)).is_err());
        // A newer latest-year period later in the header, past the first
        // block: rejected.
        for header in [
            "June 30, 2026 September 30, 2025 September 30, 2026",
            "Q2 2026 Q3 2025 Q3 2026",
        ] {
            let t = format!("(In millions)\n{header}\nTotal revenues $115 $100 $120");
            let q = "Total revenues $115 $100 $120";
            let r = verify_figure(&t, Metric::Revenue, &raw("$115", "millions", false, q));
            assert!(r.is_err(), "{header}: {r:?}");
        }
        // Oldest first, a tie, or a latest year with no period: rejected;
        // "Decrease" is not December.
        assert_eq!(period_before("Increase Decrease "), None);
        assert_eq!(period_before("Sept. 30, "), Some(Period::Month(9, 30)));
        assert_eq!(period_before("Q3"), Some(Period::Quarter(3)));
        for header in [
            "June 30, 2026 September 30, 2026",
            "Q2 2026 Q3 2026 Q2 2025 Q3 2025",
            "Q3 2026 Q3 2026",
            "2026 September 30, 2026 2025",
        ] {
            let t = format!("(In millions)\n{header}\nTotal revenues $120 $100 $90 $95");
            let q = "Total revenues $120 $100 $90 $95";
            let r = verify_figure(&t, Metric::Revenue, &raw("$120", "millions", false, q));
            assert!(r.is_err(), "{header}: {r:?}");
        }
        // CVS repeats 2026 across blocks with no month or quarter on its
        // year tokens; the label line above names the periods (three months
        // first), so the first column stands.
        let t = "Three Months Ended September 30, Nine Months Ended September 30,\nIn millions, except per share amounts 2026 2025 Change 2026 2025 Change\nTotal revenues $108,399 $102,871 $5,528 $314,921 $296,374 $18,547";
        let q = "Total revenues $108,399 $102,871 $5,528 $314,921 $296,374 $18,547";
        assert!(verify_figure(t, Metric::Revenue, &raw("$108,399", "millions", false, q)).is_ok());
        // With no period labels at all, the order is unknown.
        let t = "In millions, except per share amounts 2026 2025 Change 2026 2025 Change\nTotal revenues $108,399 $102,871 $5,528 $314,921 $296,374 $18,547";
        assert!(verify_figure(t, Metric::Revenue, &raw("$108,399", "millions", false, q)).is_err());
        let t = "(In millions)\n2026 2025 2026 2025\nTotal revenues $360 $300 $120 $100";
        let q = "Total revenues $360 $300 $120 $100";
        assert!(verify_figure(t, Metric::Revenue, &raw("$360", "millions", false, q)).is_err());
    }

    #[test]
    fn a_per_share_group_names_the_rows_under_it() {
        // SPG's statement: the EPS row is labelled by its group line.
        let spg = "Unaudited Consolidated Statements of Operations\n(Dollars in thousands, except per share amounts)\nFor the Three Months Ended September 30, For the Nine Months Ended September 30,\n2026 2025 2026 2025\nNET INCOME ATTRIBUTABLE TO COMMON STOCKHOLDERS $498,959 $606,174 $1,461,668 $1,576,006\nBASIC AND DILUTED EARNINGS PER COMMON SHARE:\nNet income attributable to common stockholders $1.54 $1.86 $4.51 $4.83";
        let q = "Net income attributable to common stockholders $1.54 $1.86 $4.51 $4.83";
        let f = verify_figure(spg, Metric::EpsGaap, &raw("$1.54", "units", false, q));
        assert_eq!(f.unwrap().value, 1.54);
        for a in ["$1.86", "$4.51", "$4.83"] {
            let r = verify_figure(spg, Metric::EpsGaap, &raw(a, "units", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
        // Still not net income.
        assert!(
            verify_figure(spg, Metric::NetIncome, &raw("$1.54", "thousands", false, q)).is_err()
        );
        // A group naming funds from operations or dividends does not make
        // its rows EPS, and a noun-less "Diluted" under it is FFO.
        for group in [
            "FUNDS FROM OPERATIONS PER SHARE:",
            "FFO per diluted share:",
            "Dividends declared per common share:",
        ] {
            let t = format!("(Dollars in thousands, except per share amounts)\nThree Months Ended September 30,\n2026 2025\n{group}\nDiluted $3.09 $3.25");
            let r = verify_figure(
                &t,
                Metric::EpsGaap,
                &raw("$3.09", "units", false, "Diluted $3.09 $3.25"),
            );
            assert!(r.is_err(), "{group}: {r:?}");
        }
        // A "Basic" row under a per-share group is basic EPS.
        let t = "(In millions, except per share amounts)\nThree Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025\nNet income (loss) per share attributable to CVS Health:\nBasic $1.70 $(3.13) $6.33 $(0.93)\nDiluted $1.68 $(3.13) $6.29 $(0.93)";
        let r = verify_figure(
            t,
            Metric::EpsGaap,
            &raw("$1.70", "units", false, "Basic $1.70 $(3.13) $6.33 $(0.93)"),
        );
        assert!(r.unwrap_err().contains("does not name"), "basic");
        let f = verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$1.68",
                "units",
                false,
                "Diluted $1.68 $(3.13) $6.29 $(0.93)",
            ),
        );
        assert_eq!(f.unwrap().value, 1.68);
        // Without a per-share group the label still needs its own keyword.
        let t = "(Dollars in thousands, except per share amounts)\nThree Months Ended September 30,\n2026 2025\nNET INCOME:\nNet income attributable to common stockholders $1.54 $1.86";
        let r = verify_figure(
            t,
            Metric::EpsGaap,
            &raw(
                "$1.54",
                "units",
                false,
                "Net income attributable to common stockholders $1.54 $1.86",
            ),
        );
        assert!(r.unwrap_err().contains("does not name"));
    }

    #[test]
    fn unverified_reasons_cite_the_quote() {
        let text = "(In millions)\nTotal revenues $9,000 $8,000";
        let v = json!({"metrics": {"revenue": {"amount": "8,000", "scale": "millions", "negative": false, "quote": "Total revenues $9,000 $8,000"}}});
        let Field::Missing { reason } = from_reply(text, &v).metrics.revenue else {
            panic!("verified");
        };
        assert!(reason.starts_with("unverified: a table amount"), "{reason}");
        assert!(
            reason.ends_with("[quote: \"Total revenues $9,000 $8,000\"]"),
            "{reason}"
        );
        let long = format!(
            "Total revenues $9,000 $8,000 {}",
            "and more words ".repeat(20)
        );
        let v = json!({"metrics": {"revenue": {"amount": "8,000", "scale": "millions", "negative": false, "quote": long}}});
        let Field::Missing { reason } = from_reply(text, &v).metrics.revenue else {
            panic!("verified");
        };
        let cited = reason.split("[quote: \"").nth(1).unwrap();
        assert!(cited.chars().count() <= 123, "{cited}");
        assert!(cited.ends_with("…\"]"), "{cited}");
    }

    #[test]
    fn pro_forma_columns_are_never_the_reported_figure() {
        let ok = |t: &str, m: Metric, amount: &str, neg: bool, q: &str| {
            let r = verify_figure(
                t,
                m,
                &raw(
                    amount,
                    if m == Metric::Revenue || m == Metric::NetIncome {
                        "millions"
                    } else {
                        "units"
                    },
                    neg,
                    q,
                ),
            );
            assert!(r.is_ok(), "{amount} in {q:?}: {r:?}");
        };
        let bad = |t: &str, m: Metric, amount: &str, neg: bool, q: &str| {
            let r = verify_figure(
                t,
                m,
                &raw(
                    amount,
                    if m == Metric::Revenue || m == Metric::NetIncome {
                        "millions"
                    } else {
                        "units"
                    },
                    neg,
                    q,
                ),
            );
            assert!(
                r.unwrap_err().contains("current period"),
                "{amount} in {q:?}"
            );
        };
        // CHTR's highlights: the Cox merger's pro forma quarter first, then
        // the reported ("Actual") quarter.
        let chtr = "Third Quarter Financial Results\n(in millions)\nThree Months Ended September 30,\n2026 2025 2026 2025\nPro Forma Pro Forma % Change Actual Actual % Change\nRevenues:\nInternet $7,097 $7,366 (3.7) % $6,395 $5,971 7.1%\nTotal Revenues $16,505 $16,764 (1.5) % $14,922 $13,672 9.1%\nNet income (loss) attributable to Charter shareholders $(2,079) $952 (318.4) % $(2,397) $1,137 (310.8) %";
        let q = "Total Revenues $16,505 $16,764 (1.5) % $14,922 $13,672 9.1%";
        ok(chtr, Metric::Revenue, "$14,922", false, q);
        for a in ["$16,505", "$16,764", "$13,672"] {
            bad(chtr, Metric::Revenue, a, false, q);
        }
        let q = "Net income (loss) attributable to Charter shareholders $(2,079) $952 (318.4) % $(2,397) $1,137 (310.8) %";
        ok(chtr, Metric::NetIncome, "$(2,397)", true, q);
        bad(chtr, Metric::NetIncome, "$(2,079)", true, q);
        bad(chtr, Metric::NetIncome, "$1,137", false, q);
        // CHTR's statements: the reported one reads its first column, the
        // pro forma one and its alternative presentation none.
        let head = "CHARTER COMMUNICATIONS, INC. AND SUBSIDIARIES\nUNAUDITED CONSOLIDATED STATEMENTS OF OPERATIONS\n(dollars in millions, except per share data)\nThree Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025";
        let t = format!("{head}\nActual Actual Actual Actual\nREVENUES $14,922 $13,672 $42,045 $41,173\nDiluted $(20.50) $8.34 $0.38 $25.95");
        ok(
            &t,
            Metric::Revenue,
            "$14,922",
            false,
            "REVENUES $14,922 $13,672 $42,045 $41,173",
        );
        ok(
            &t,
            Metric::EpsGaap,
            "$(20.50)",
            true,
            "Diluted $(20.50) $8.34 $0.38 $25.95",
        );
        bad(
            &t,
            Metric::Revenue,
            "$42,045",
            false,
            "REVENUES $14,922 $13,672 $42,045 $41,173",
        );
        let t = format!("{head}\nPro Forma (b) Pro Forma (b) Pro Forma (b) Pro Forma (b)\nREVENUES $16,505 $16,764 $49,720 $50,615\nDiluted $(17.78) $6.98 $(0.04) $21.58");
        for a in ["$16,505", "$16,764", "$49,720", "$50,615"] {
            bad(
                &t,
                Metric::Revenue,
                a,
                false,
                "REVENUES $16,505 $16,764 $49,720 $50,615",
            );
        }
        bad(
            &t,
            Metric::EpsGaap,
            "$(17.78)",
            true,
            "Diluted $(17.78) $6.98 $(0.04) $21.58",
        );
        let t = "UNAUDITED ALTERNATIVE PRESENTATION OF ADJUSTED EBITDA\n(dollars in millions)\nThree Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025\nPro Forma (b) Pro Forma (b) % Change Pro Forma (b) Pro Forma (b) % Change\nREVENUES:\nTotal Revenues 16,505 16,764 (1.5) % 49,720 50,615 (1.8) %";
        bad(
            t,
            Metric::Revenue,
            "16,505",
            false,
            "Total Revenues 16,505 16,764 (1.5) % 49,720 50,615 (1.8) %",
        );
        // The reported block must be the quarter alone, and its latest
        // year: prior year first reads the second reported column; with a
        // nine-month block too, the order of blocks is unknown.
        let t = "(in millions)\nThree Months Ended September 30,\n2025 2026 2025 2026\nPro Forma Pro Forma Actual Actual\nTotal Revenues $16,764 $16,505 $13,672 $14,922";
        let q = "Total Revenues $16,764 $16,505 $13,672 $14,922";
        ok(t, Metric::Revenue, "$14,922", false, q);
        for a in ["$16,764", "$16,505", "$13,672"] {
            bad(t, Metric::Revenue, a, false, q);
        }
        let t = "(in millions)\nThree Months Ended September 30, Nine Months Ended September 30,\n2026 2025 2026 2025\nPro Forma Pro Forma Actual Actual\nTotal Revenues $16,505 $16,764 $14,922 $13,672";
        let q = "Total Revenues $16,505 $16,764 $14,922 $13,672";
        for a in ["$16,505", "$16,764", "$14,922", "$13,672"] {
            bad(t, Metric::Revenue, a, false, q);
        }
    }

    #[test]
    fn period_labels_can_span_lines_above_the_years() {
        // DUK's statement: the period lengths, then the dates, then the
        // years, each on its own line.
        let duk = "(In millions, except per share amounts)\nThree Months Ended Nine Months Ended\nSeptember 30, September 30,\n2026 2025 2026 2025\nOperating Revenues\nRegulated electric $8,677 $8,106 $23,583 $22,138\nRegulated natural gas 358 361 2,073 1,928\nNonregulated electric and other 69 75 218 233\nTotal operating revenues 9,104 8,542 25,874 24,299\nEarnings Per Share – Basic and Diluted\nNet income available to Duke Energy Corporation common stockholders\nBasic and Diluted $1.99 $1.81 $5.34 $4.81";
        let q = "Total operating revenues 9,104 8,542 25,874 24,299";
        let f = verify_figure(duk, Metric::Revenue, &raw("9,104", "millions", false, q));
        assert_eq!(f.unwrap().value, 9.104e9);
        for a in ["8,542", "25,874", "24,299"] {
            let r = verify_figure(duk, Metric::Revenue, &raw(a, "millions", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
        let q = "Basic and Diluted $1.99 $1.81 $5.34 $4.81";
        let f = verify_figure(duk, Metric::EpsGaap, &raw("$1.99", "units", false, q));
        assert_eq!(f.unwrap().value, 1.99);
        for a in ["$1.81", "$5.34", "$4.81"] {
            let r = verify_figure(duk, Metric::EpsGaap, &raw(a, "units", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
        // SPG writes "For the Three Months Ended ...".
        let spg = "(Dollars in thousands, except per share amounts)\nFor the Three Months Ended September 30, For the Nine Months Ended September 30,\n2026 2025 2026 2025\nREVENUE:\nLease income $1,681,393 $1,452,930 $4,969,633 $4,199,812\nTotal revenue 1,814,556 1,601,572 5,362,247 4,573,043\nDiluted net income per share $1.54 $1.86 $4.51 $4.83";
        let q = "Total revenue 1,814,556 1,601,572 5,362,247 4,573,043";
        let f = verify_figure(
            spg,
            Metric::Revenue,
            &raw("1,814,556", "thousands", false, q),
        );
        assert_eq!(f.unwrap().value, 1.814556e9);
        for a in ["1,601,572", "5,362,247", "4,573,043"] {
            let r = verify_figure(spg, Metric::Revenue, &raw(a, "thousands", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
        let q = "Diluted net income per share $1.54 $1.86 $4.51 $4.83";
        let f = verify_figure(spg, Metric::EpsGaap, &raw("$1.54", "units", false, q));
        assert_eq!(f.unwrap().value, 1.54);
        for a in ["$1.86", "$4.51", "$4.83"] {
            let r = verify_figure(spg, Metric::EpsGaap, &raw(a, "units", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
        // Prior year first: the quarter's 2026 column, never the prior
        // year's or the nine months'.
        let t = "(In millions)\nThree Months Ended Nine Months Ended\nSeptember 30, September 30,\n2025 2026 2025 2026\nTotal revenues $100 $120 $300 $360";
        let q = "Total revenues $100 $120 $300 $360";
        let f = verify_figure(t, Metric::Revenue, &raw("$120", "millions", false, q));
        assert_eq!(f.unwrap().value, 120e6);
        for a in ["$100", "$300", "$360"] {
            let r = verify_figure(t, Metric::Revenue, &raw(a, "millions", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
        // The year-to-date block first is still rejected, and so is a
        // period line that a row or a group label separates from the years.
        for header in [
            "Nine Months Ended Three Months Ended\nSeptember 30, September 30,\n2026 2025 2026 2025",
            "For the Nine Months Ended September 30, For the Three Months Ended September 30,\n2026 2025 2026 2025",
            "Three Months Ended Nine Months Ended\nOperating Revenues\nSeptember 30, September 30,\n2026 2025 2026 2025",
            "Three Months Ended Nine Months Ended\nTotal revenues 300 360 900 1,080\nSeptember 30, September 30,\n2026 2025 2026 2025",
        ] {
            let t = format!("(In millions)\n{header}\nTotal revenues $360 $300 $120 $100");
            let q = "Total revenues $360 $300 $120 $100";
            for a in ["$360", "$300", "$120", "$100"] {
                let r = verify_figure(&t, Metric::Revenue, &raw(a, "millions", false, q));
                assert!(r.is_err(), "{a} under {header:?}: {r:?}");
            }
        }
        // Label lines count only inside the table, below its scale heading,
        // up to three of them, and never through a number-only row or a
        // "(Unaudited)"; label lines that disagree about which period comes
        // first say nothing.
        for top in [
            "Three Months Ended\n(In millions)\nNine Months Ended Three Months Ended",
            "Q3\n(In millions)\nSix Months Ended",
            "Three Months Ended\n(In millions)\n1,234 5,678",
            "Quarter Ended\n(Unaudited)\n(In millions)",
            "For the Quarter\n(In millions)",
            "(In millions)\nThree Months Ended Nine Months Ended\n1,234 5,678",
            "(In millions)\nThree Months Ended Nine Months Ended\n(Unaudited)",
            "(In millions)\nThree Months Ended\nSeptember 30,\nSeptember 30,\nSeptember 30,",
            "(In millions)\nThree Months Ended\nNine Months Ended",
            "(In millions)\nQ3\nYear Ended December 31, Year Ended",
            "(In millions)\nThree Months Ended Nine Months Ended\nNine Months Ended Three Months Ended",
        ] {
            let t = format!("{top}\n2026 2025 2026 2025\nTotal revenues $360 $300 $120 $100");
            let q = "Total revenues $360 $300 $120 $100";
            for a in ["$360", "$300", "$120", "$100"] {
                let r = verify_figure(&t, Metric::Revenue, &raw(a, "millions", false, q));
                assert!(r.is_err(), "{a} under {top:?}: {r:?}");
            }
        }
        let t = "(In millions)\nThree Months Ended\nNine Months Ended 2026 2025 Three Months Ended 2026 2025\nTotal revenues $360 $300 $120 $100";
        let q = "Total revenues $360 $300 $120 $100";
        assert!(verify_figure(t, Metric::Revenue, &raw("$360", "millions", false, q)).is_err());
        // Dated years do not settle disagreeing label lines either; with
        // agreeing ones, the first cell is the quarter.
        let dated = "September 30, 2026 September 30, 2025 September 30, 2026 September 30, 2025";
        for top in [
            "(In millions)\nThree Months Ended\nNine Months Ended".to_string(),
            "(In millions)\nQ3\nYear Ended December 31, Year Ended".to_string(),
            "(In millions)\nThree Months Ended Nine Months Ended\nNine Months Ended Three Months Ended"
                .to_string(),
            format!("(In millions)\nThree Months Ended\nNine Months Ended {dated}"),
        ] {
            let header = if top.ends_with(dated) { "" } else { dated };
            let t = format!("{top}\n{header}\nTotal revenues $360 $300 $120 $100");
            for a in ["$360", "$300", "$120", "$100"] {
                let r = verify_figure(&t, Metric::Revenue, &raw(a, "millions", false, q));
                assert!(r.is_err(), "{a} under {top:?}: {r:?}");
            }
        }
        let t = format!("(In millions)\nThree Months Ended Nine Months Ended\n{dated}\nTotal revenues $360 $300 $120 $100");
        let f = verify_figure(&t, Metric::Revenue, &raw("$360", "millions", false, q));
        assert_eq!(f.unwrap().value, 360e6);
        for a in ["$300", "$120", "$100"] {
            let r = verify_figure(&t, Metric::Revenue, &raw(a, "millions", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
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
        // Past the title line the earlier header can still reject the first
        // column (its first year is not the latest), and never accepts.
        let t = "(In millions) 2025 2026\nGross Bookings $49,740 $59,819\nSupplemental table\nTotal revenues $14,766 $13,467";
        bad(t, "$14,766", q);
        bad(t, "$13,467", q);
        // A row-group label between header and row: the header can only
        // reject.
        let t = "(In millions)\n2025 2026\nRevenues:\nTotal revenues $100 $120";
        let q2 = "Total revenues $100 $120";
        bad(t, "$100", q2);
        bad(t, "$120", q2);
        let t = "(In millions)\n2026 2025\nRevenues:\nTotal revenues $120 $100";
        let q2 = "Total revenues $120 $100";
        ok(t, "$120", q2);
        bad(t, "$100", q2);
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
        // EMR's statement: "Costs and expenses:" and a group label sit
        // between the "2025 2026 2025 2026" header and the EPS row.
        let emr = "(AMOUNTS IN MILLIONS EXCEPT PER SHARE, UNAUDITED)\nQuarter Ended September 30, Year Ended September 30,\n2025 2026 2025 2026\nNet sales $4,855 $5,201 $18,016 $18,982\nCosts and expenses:\nCost of sales 2,335 2,461 8,497 8,854\nNet earnings common stockholders $609 $744 $2,270 $2,691\nDiluted earnings per share common stockholders\nEarnings from continuing operations $1.07 $1.33 $3.99 $4.79\nDiscontinued operations 0.01 — 0.01 —\nDiluted earnings per common share $1.08 $1.33 $4.00 $4.79";
        let q = "Diluted earnings per common share $1.08 $1.33 $4.00 $4.79";
        for a in ["$1.08", "$1.33", "$4.79"] {
            let r = verify_figure(emr, Metric::EpsGaap, &raw(a, "units", false, q));
            assert!(r.unwrap_err().contains("current period"), "{a}");
        }
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
        // Hyphenated, as in "Southern Company reports third-quarter 2026 earnings",
        // with an ASCII or a non-breaking hyphen.
        for (q, word) in [(1, "first"), (2, "second"), (3, "third"), (4, "fourth")] {
            for dash in ["-", "\u{2011}"] {
                let quote = format!("Southern Company reports {word}{dash}quarter 2026 earnings");
                let v = json!({"quarter": q, "fiscal_year": 2026, "quote": quote});
                assert!(verify_period(&quote, &v).is_ok(), "{quote}");
            }
        }
    }

    #[test]
    fn fiscal_period_falls_back_to_the_headline() {
        let t = "EX-99.1 2 qcom092726erex991-fourth-quarter-2026.htm\n\
                 Qualcomm Contact: Investor Relations\nPhone: 1-858-658-4813\n\
                 Qualcomm Announces Fourth Quarter and Fiscal 2026 Results\n\
                 Fiscal 2026 GAAP Revenues: $43.1 billion\n\
                 SAN DIEGO - November 4, 2026 - Qualcomm today announced results";
        let period = |q: Value, fy: i64| {
            let v = json!({"fiscal_period": {"quarter": q, "fiscal_year": fy, "quote": "a quote that is not in the text"}});
            from_reply(t, &v).fiscal_period
        };
        match period(json!(4), 2026) {
            Field::Value(p) => {
                assert_eq!((p.quarter, p.fiscal_year), (Some(4), 2026));
                assert_eq!(
                    p.quote,
                    "Qualcomm Announces Fourth Quarter and Fiscal 2026 Results"
                );
            }
            other => panic!("{other:?}"),
        }
        // The headline must name the claimed quarter and year.
        assert!(matches!(period(json!(3), 2026), Field::Missing { .. }));
        assert!(matches!(period(json!(4), 2027), Field::Missing { .. }));
        // No fallback for a full-year claim: the dateline names the year.
        assert!(matches!(period(json!(null), 2026), Field::Missing { .. }));
        // A paragraph naming the period is not a headline.
        let para = format!(
            "Qualcomm today announced fourth quarter fiscal 2026 results {}\nrest of the release",
            "with revenue of $10.3 billion and more ".repeat(5)
        );
        let v = json!({"fiscal_period": {"quarter": 4, "fiscal_year": 2026, "quote": "not in the text at all"}});
        assert!(matches!(
            from_reply(&para, &v).fiscal_period,
            Field::Missing { .. }
        ));
        // Only the exhibit's opening counts as the headline.
        let late = format!("{}\n{t}", "x ".repeat(HEADLINE_CHARS));
        let v = json!({"fiscal_period": {"quarter": 4, "fiscal_year": 2026, "quote": "not in the text at all"}});
        assert!(matches!(
            from_reply(&late, &v).fiscal_period,
            Field::Missing { .. }
        ));
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
