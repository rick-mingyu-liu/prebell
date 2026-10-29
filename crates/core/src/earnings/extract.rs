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

/// A table row's revenue label is the total, not a segment: "Total net
/// sales", "Revenues", "Net revenues", "Revenue, net" pass; "Products net
/// sales" and "Services revenue" do not.
fn total_revenue_row(label: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let label = label.trim().trim_end_matches(['$', ':', ' ']).trim();
    has_word(label, "total")
        || R.get_or_init(|| {
            Regex::new(r"(?i)^(?:net\s+)?(?:revenues?|sales)(?:\s*,\s*net)?$")
                .expect("valid built-in regex")
        })
        .is_match(label)
}

/// Applies the quote-anchoring rules: the quote is in the exhibit; it starts
/// with a label naming the metric (letters before the amount); the amount is
/// a whole number in the quote, read with the unit words that follow it
/// there; a dollar figure written as a bare table number has the scale of
/// the table heading above it, is the row's first number and, for revenue,
/// is the total row; per-share figures carry no size word and are not
/// dividends; a sign has evidence next to it.
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
            // read from the current-period column (the row's first number).
            if scale == 1.0 {
                return Err(format!("bare amount {amount:?} needs the table's scale"));
            }
            if quote[..at].chars().any(|c| c.is_ascii_digit()) {
                return Err("a bare table amount must be the row's first number".into());
            }
            if metric == Metric::Revenue && !total_revenue_row(label) {
                return Err(format!(
                    "{:?} looks like a segment row; revenue must be the total",
                    label.trim()
                ));
            }
            match table_scale(text, &quote) {
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

/// The unit of the nearest "(In millions ...)"-style heading before the
/// quote. Only the parenthesised lead counts: "(In millions, except shares,
/// which are reflected in thousands)" is a millions table.
fn table_scale(text: &str, quote: &str) -> Option<f64> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        Regex::new(r"(?i)\([^()]{0,30}?\bin\s+(thousands|millions|billions)\b")
            .expect("valid built-in regex")
    });
    let doc = collapse(text);
    let before = &doc[..doc.find(quote)?];
    let unit = r
        .captures_iter(before)
        .last()?
        .get(1)?
        .as_str()
        .to_ascii_lowercase();
    scale_of(&unit)
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
