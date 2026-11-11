//! The headline figures of a released CPI or jobs report, read by code from
//! the opening paragraphs of its BLS release page (the decision of
//! 2026-11-10: "state the CPI figure"). Each figure carries the sentence it
//! came from; a sentence that does not parse gives no figure, and nothing
//! is ever estimated or asked of an LLM. Never consensus: the page and the
//! brief state the figures only, never against a forecast.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::numbers::Lang;

/// Which headline figure. The CPI's changes are the seasonally adjusted
/// monthly change and the 12-month change before seasonal adjustment, as
/// the release states them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FigureKey {
    /// All items, change on the month.
    CpiMonth,
    /// All items, change over the last 12 months.
    CpiYear,
    /// All items less food and energy, change on the month.
    CoreMonth,
    /// All items less food and energy, change over the last 12 months.
    CoreYear,
    /// Change in total nonfarm payroll employment.
    Payrolls,
    /// The unemployment rate.
    UnemploymentRate,
}

/// One figure a release states.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReleaseFigure {
    pub key: FigureKey,
    /// Percent units for the CPI changes and the rate (0.3 = 0.3%), persons
    /// for payrolls; signed.
    pub value: f64,
    /// The release's sentence it came from, whitespace collapsed.
    pub quote: String,
}

/// ASCII classes (`\s`, `\d`): the release text is ASCII, and Unicode
/// classes make each pattern several times slower to compile, CPU the
/// Worker pays on every brief that reads a release.
fn re(p: &str) -> Regex {
    Regex::new(&format!("(?-u){p}")).expect("valid built-in regex")
}

/// A sentence longer than this is not one of the opening sentences.
const SENTENCE_MAX: usize = 600;

/// Where the sentence starting at `start` ends (past its period): the
/// first period followed by a space, a tag or the end, past "U.S.". `None`
/// when it does not end within `SENTENCE_MAX` bytes.
fn sentence_end(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    let mut from = 0;
    while let Some(i) = rest[from..].find('.').map(|i| from + i) {
        if i > SENTENCE_MAX {
            return None;
        }
        let next = rest[i + 1..].chars().next();
        if next.is_none_or(|c| c.is_whitespace() || c == '<') && !rest[..i].ends_with("U.S") {
            return Some(start + i + 1);
        }
        from = i + 1;
    }
    None
}

/// The sentence starting at `start`, whitespace collapsed.
fn sentence(text: &str, start: usize) -> Option<String> {
    let end = sentence_end(text, start)?;
    Some(
        text[start..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// +1, -1 or 0 for the verb a release uses ("edged up", "was unchanged").
fn direction(verb: &str) -> Option<f64> {
    match verb
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .as_str()
    {
        "increased" | "rose" | "grew" | "edged up" => Some(1.0),
        "decreased" | "fell" | "declined" | "edged down" => Some(-1.0),
        "was unchanged" => Some(0.0),
        _ => None,
    }
}

/// The signed percent change from a verb and its amount: "was unchanged"
/// takes no amount, every other verb needs one.
fn change(verb: &str, amount: Option<&str>) -> Option<f64> {
    let d = direction(verb)?;
    if d == 0.0 {
        return Some(0.0);
    }
    Some(d * amount?.parse::<f64>().ok()?)
}

/// One alternation for the four CPI sentences (one regex compiles faster
/// than four). Each branch names its verb, amount and, for core, period.
fn cpi_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        let verb = |n: &str| format!(r"(?P<{n}>increased|rose|decreased|fell|declined|was\s+unchanged)");
        let amount = |n: &str| format!(r"(?:\s+(?P<{n}>\d+\.\d+)\s+percent)?");
        re(&[
            // "The Consumer Price Index for All Urban Consumers (CPI-U)
            // increased 0.4 percent on a seasonally adjusted basis in
            // October" / "was unchanged in July on a seasonally adjusted basis".
            format!(
                r"The\s+Consumer\s+Price\s+Index\s+for\s+All\s+Urban\s+Consumers\s+\(CPI-U\)\s+{}{}(?:\s+in\s+[A-Z][a-z]+)?\s+on\s+a\s+seasonally\s+adjusted\s+basis",
                verb("hv"),
                amount("ha")
            ),
            // "Over the last 12 months, the all items index increased 3.0
            // percent before seasonal adjustment."
            format!(
                r"Over\s+the\s+(?:last|past)\s+12\s+months,\s+the\s+all\s+items\s+index\s+{}{}\s+before\s+seasonal\s+adjustment",
                verb("av"),
                amount("aa")
            ),
            // "The all items index rose 3.0 percent for the 12 months ending
            // September".
            format!(
                r"The\s+all\s+items\s+index\s+{}{}\s+(?:for\s+the\s+12\s+months\s+ending|over\s+the\s+(?:last|past)\s+12\s+months)",
                verb("bv"),
                amount("ba")
            ),
            // "The index for all items less food and energy rose 0.3 percent
            // in October" / "The all items less food and energy index rose
            // 3.0 percent over the last 12 months."
            format!(
                r"(?:The\s+index\s+for\s+all\s+items\s+less\s+food\s+and\s+energy|The\s+all\s+items\s+less\s+food\s+and\s+energy\s+index)\s+{}{}\s+(?P<cp>in\s+[A-Z][a-z]+|over\s+the\s+month|over\s+the\s+(?:last|past)\s+12\s+months|for\s+the\s+12\s+months\s+ending)",
                verb("cv"),
                amount("ca")
            ),
        ]
        .join("|"))
    })
}

fn cpi(text: &str) -> Vec<ReleaseFigure> {
    // The first sentence of each kind that parses; the 12-month change's
    // first wording before its second.
    let [mut month, mut year_a, mut year_b, mut core_month, mut core_year] =
        [None, None, None, None, None];
    for c in cpi_re().captures_iter(text) {
        let Some(start) = c.get(0).map(|m| m.start()) else {
            continue;
        };
        let (slot, key, verb, amount) = if let Some(v) = c.name("hv") {
            (&mut month, FigureKey::CpiMonth, v, "ha")
        } else if let Some(v) = c.name("av") {
            (&mut year_a, FigureKey::CpiYear, v, "aa")
        } else if let Some(v) = c.name("bv") {
            (&mut year_b, FigureKey::CpiYear, v, "ba")
        } else if let Some(v) = c.name("cv") {
            if c.name("cp").is_some_and(|p| p.as_str().contains("12")) {
                (&mut core_year, FigureKey::CoreYear, v, "ca")
            } else {
                (&mut core_month, FigureKey::CoreMonth, v, "ca")
            }
        } else {
            continue;
        };
        if slot.is_some() {
            continue;
        }
        *slot = (|| {
            Some(ReleaseFigure {
                key,
                value: change(verb.as_str(), c.name(amount).map(|m| m.as_str()))?,
                quote: sentence(text, start)?,
            })
        })();
    }
    [month, year_a.or(year_b), core_month, core_year]
        .into_iter()
        .flatten()
        .collect()
}

fn jobs_res() -> &'static [Regex; 2] {
    static R: OnceLock<[Regex; 2]> = OnceLock::new();
    R.get_or_init(|| {
        const NUM: &str = r"(\d{1,3}(?:,\d{3})+|\d+(?:\.\d+)?)(\s+million)?";
        [
            // "Total nonfarm payroll employment rose by 165,000 in October"
            // / "changed little in August (+22,000)".
            re(&format!(
                r"Total\s+nonfarm\s+payroll\s+employment\s+(?:(rose|increased|grew|edged\s+up|declined|decreased|fell|edged\s+down)\s+by\s+{NUM}|(?:changed\s+little|was\s+little\s+changed|was\s+essentially\s+unchanged|was\s+unchanged|held\s+steady)\s+(?:in\s+[A-Z][a-z]+\s+)?\(((?:[+-]|(?u:−))?){NUM}\))"
            )),
            // The overall rate, by its verbs, so "the unemployment rate for
            // teenagers rose to ..." is not it: "the unemployment rate, at
            // 4.4 percent, changed little" / "changed little at 4.1 percent"
            // / "was little changed at" / "edged up to 4.3 percent".
            re(r"[Tt]he\s+unemployment\s+rate(?:,\s+at\s+(\d+\.\d+)\s+percent|\s+(?:was\s+)?(?:essentially\s+|little\s+)?(?:changed|unchanged|edged\s+up|edged\s+down|ticked\s+up|ticked\s+down|rose|fell|increased|decreased|declined|held(?:\s+steady)?|remained)(?:\s+little)?\s+(?:at|to)\s+(\d+\.\d+)\s+percent)"),
        ]
    })
}

/// The rate a match of the `jobs_res` rate pattern states.
fn rate(c: regex::Captures) -> Option<f64> {
    c.get(1).or(c.get(2))?.as_str().parse().ok()
}

/// "165,000" / "20.5" + " million" in persons.
fn persons(n: &str, million: bool) -> Option<f64> {
    let v: f64 = n.replace(',', "").parse().ok()?;
    Some(if million { v * 1e6 } else { v })
}

fn jobs(text: &str) -> Vec<ReleaseFigure> {
    let [payrolls, rate_re] = jobs_res();
    let mut out = Vec::new();
    let mut payroll_sentence = None;
    let mut next_sentence = None;
    if let Some(c) = payrolls.captures(text) {
        let value = if let Some(verb) = c.get(1) {
            direction(verb.as_str())
                .zip(persons(&c[2], c.get(3).is_some()))
                .map(|(d, v)| d * v)
        } else {
            let sign = if matches!(&c[4], "-" | "−") {
                -1.0
            } else {
                1.0
            };
            persons(&c[5], c.get(6).is_some()).map(|v| sign * v)
        };
        let start = c.get(0).map_or(0, |m| m.start());
        payroll_sentence = sentence(text, start);
        // The headline's next sentence, for a rate stated on its own.
        next_sentence = sentence_end(text, start).and_then(|end| {
            let rest = &text[end..];
            sentence(text, end + rest.len() - rest.trim_start().len())
        });
        if let (Some(value), Some(quote)) = (value, payroll_sentence.clone()) {
            out.push(ReleaseFigure {
                key: FigureKey::Payrolls,
                value,
                quote,
            });
        }
    }
    // The rate from the headline sentence, else the one after it; never
    // from further on, where the sub-group rates are.
    let rate_of = |s: Option<String>| s.and_then(|s| Some((rate(rate_re.captures(&s)?)?, s)));
    if let Some((value, quote)) = rate_of(payroll_sentence).or_else(|| rate_of(next_sentence)) {
        out.push(ReleaseFigure {
            key: FigureKey::UnemploymentRate,
            value,
            quote,
        });
    }
    out
}

/// Bytes of a release read for its figures, from its `<h1>` on: the
/// opening summary is about 2 KB.
const OPENING_BYTES: usize = 12 * 1024;

/// The release's opening, past the site navigation: from its `<h1>` (the
/// page's start when it has none), at most `OPENING_BYTES`.
fn opening(page: &str) -> &str {
    let start = page.find("<h1").unwrap_or(0);
    let mut end = page.len().min(start + OPENING_BYTES);
    while !page.is_char_boundary(end) {
        end -= 1;
    }
    &page[start..end]
}

/// The headline figures on a release page (or its first bytes) for
/// `series` `cpi` or `jobs`; none for other kinds or a page that does not
/// parse.
pub fn parse_figures(series: &str, page: &str) -> Vec<ReleaseFigure> {
    match series {
        "cpi" => cpi(opening(page)),
        "jobs" => jobs(opening(page)),
        _ => vec![],
    }
}

/// Whether `series`' release pages have figures to read.
pub fn has_figures(series: &str) -> bool {
    matches!(series, "cpi" | "jobs")
}

/// The sign the site prints: "+", or U+2212 as the movers and share lines do.
fn sign(v: f64) -> &'static str {
    if v < 0.0 {
        "−"
    } else {
        "+"
    }
}

/// "+0.3%", "−0.1%".
fn pct(v: f64) -> String {
    format!("{}{:.1}%", sign(v), v.abs())
}

/// "+119,000", "−13,000".
fn signed_count(v: f64) -> String {
    let digits = format!("{:.0}", v.abs());
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{grouped}", sign(v))
}

/// "+11.9万人" (persons in units of 10,000, at most one decimal).
fn signed_wan(v: f64) -> String {
    let w = format!("{:.1}", v.abs() / 1e4);
    let w = w.strip_suffix(".0").unwrap_or(&w);
    format!("{}{w}万人", sign(v))
}

/// The figures as the watch line states them, or `None` when there are
/// none: "+0.3% on the month, +3.0% from a year earlier; core +0.2% on the
/// month, +3.0% from a year earlier" / "环比 +0.3%，同比 +3.0%；核心环比
/// +0.2%，同比 +3.0%"; "Payrolls +119,000; unemployment rate 4.4%" /
/// "非农就业 +11.9万人；失业率 4.4%".
pub fn figures_text(figures: &[ReleaseFigure], lang: Lang) -> Option<String> {
    let get = |k: FigureKey| figures.iter().find(|f| f.key == k).map(|f| f.value);
    let zh = lang == Lang::Zh;
    // One CPI group: its monthly and 12-month changes.
    let group = |month: FigureKey, year: FigureKey| -> Vec<String> {
        [
            (month, "on the month", "环比"),
            (year, "from a year earlier", "同比"),
        ]
        .into_iter()
        .filter_map(|(k, en, z)| {
            let v = get(k)?;
            Some(match (zh, v == 0.0) {
                (false, false) => format!("{} {en}", pct(v)),
                (false, true) => format!("unchanged {en}"),
                (true, false) => format!("{z} {}", pct(v)),
                (true, true) => format!("{z}持平"),
            })
        })
        .collect()
    };
    let sep = if zh { "，" } else { ", " };
    let mut parts = Vec::new();
    let all = group(FigureKey::CpiMonth, FigureKey::CpiYear);
    if !all.is_empty() {
        parts.push(all.join(sep));
    }
    let core = group(FigureKey::CoreMonth, FigureKey::CoreYear);
    if !core.is_empty() {
        let lead = if zh { "核心" } else { "core " };
        parts.push(format!("{lead}{}", core.join(sep)));
    }
    if let Some(v) = get(FigureKey::Payrolls) {
        parts.push(match (zh, v == 0.0) {
            (false, false) => format!("Payrolls {}", signed_count(v)),
            (false, true) => "Payrolls unchanged".into(),
            (true, false) => format!("非农就业 {}", signed_wan(v)),
            (true, true) => "非农就业持平".into(),
        });
    }
    if let Some(v) = get(FigureKey::UnemploymentRate) {
        let label = if zh { "失业率" } else { "unemployment rate" };
        parts.push(format!("{label} {v:.1}%"));
    }
    (!parts.is_empty()).then(|| parts.join(if zh { "；" } else { "; " }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numbers::check;

    const CPI: &str = include_str!("../../tests/fixtures/bls_cpi_release.htm");
    const CPI_DOWN: &str = include_str!("../../tests/fixtures/bls_cpi_release_decrease.htm");
    const CPI_FLAT: &str = include_str!("../../tests/fixtures/bls_cpi_release_unchanged.htm");
    const JOBS: &str = include_str!("../../tests/fixtures/bls_empsit_release.htm");
    const JOBS_DOWN: &str = include_str!("../../tests/fixtures/bls_empsit_release_decline.htm");
    const JOBS_SUBGROUP: &str =
        include_str!("../../tests/fixtures/bls_empsit_release_subgroup.htm");

    fn values(f: &[ReleaseFigure]) -> Vec<(FigureKey, f64)> {
        f.iter().map(|f| (f.key, f.value)).collect()
    }

    #[test]
    fn october_2026_cpi_gives_the_monthly_changes_and_no_12_month_ones() {
        // BLS published no 12-month changes for October 2026 (no October
        // 2025 survey during the funding lapse): those are absent, not guessed.
        let f = parse_figures("cpi", CPI);
        assert_eq!(
            values(&f),
            [(FigureKey::CpiMonth, 0.4), (FigureKey::CoreMonth, 0.3)]
        );
        assert_eq!(
            f[0].quote,
            "The Consumer Price Index for All Urban Consumers (CPI-U) increased 0.4 percent on a \
             seasonally adjusted basis in October after rising 0.6 percent in September, the U.S. \
             Bureau of Labor Statistics reported today."
        );
        assert_eq!(
            f[1].quote,
            "The index for all items less food and energy rose 0.3 percent in October, as it did \
             in September and August."
        );
    }

    #[test]
    fn older_cpi_wordings_give_all_four_figures() {
        let f = parse_figures("cpi", CPI_DOWN);
        assert_eq!(
            values(&f),
            [
                (FigureKey::CpiMonth, -0.1),
                (FigureKey::CpiYear, 0.1),
                (FigureKey::CoreMonth, -0.1),
                (FigureKey::CoreYear, 1.2),
            ]
        );
        assert_eq!(
            f[1].quote,
            "Over the last 12 months, the all items index increased 0.1 percent before seasonal adjustment."
        );
        assert_eq!(
            f[3].quote,
            "The index for all items less food and energy rose 1.2 percent over the last 12 months."
        );
        let f = parse_figures("cpi", CPI_FLAT);
        assert_eq!(
            values(&f),
            [
                (FigureKey::CpiMonth, 0.0),
                (FigureKey::CpiYear, 8.5),
                (FigureKey::CoreMonth, 0.3),
                (FigureKey::CoreYear, 5.9),
            ]
        );
        assert!(f[0].quote.starts_with(
            "The Consumer Price Index for All Urban Consumers (CPI-U) was unchanged in July"
        ));
        // The second wording of the 12-month change, when the first is absent.
        let alt =
            "<pre>The all items index rose 3.0 percent for the 12 months ending September, after \
                   rising 2.9 percent over the 12 months ending August.</pre>";
        assert_eq!(
            values(&parse_figures("cpi", alt)),
            [(FigureKey::CpiYear, 3.0)]
        );
    }

    #[test]
    fn the_jobs_report_gives_payrolls_and_the_rate() {
        let f = parse_figures("jobs", JOBS);
        assert_eq!(
            values(&f),
            [
                (FigureKey::Payrolls, 165_000.0),
                (FigureKey::UnemploymentRate, 4.1)
            ]
        );
        let s = "Total nonfarm payroll employment rose by 165,000 in October, and the unemployment rate \
                 changed little at 4.1 percent, the U.S. Bureau of Labor Statistics reported today.";
        assert_eq!((f[0].quote.as_str(), f[1].quote.as_str()), (s, s));
        let f = parse_figures("jobs", JOBS_DOWN);
        assert_eq!(
            values(&f),
            [
                (FigureKey::Payrolls, -13_000.0),
                (FigureKey::UnemploymentRate, 4.3)
            ]
        );
    }

    #[test]
    fn a_sub_group_rate_is_never_taken_for_the_unemployment_rate() {
        // The headline's rate is in a form not read ("increased by 0.2
        // percentage point to 4.6 percent"); the teenagers' and Black
        // workers' rates that follow are not the rate.
        assert_eq!(
            values(&parse_figures("jobs", JOBS_SUBGROUP)),
            [(FigureKey::Payrolls, 64_000.0)]
        );
        // Nor a sub-group rate in the headline itself.
        assert!(values(&parse_figures(
            "jobs",
            "Total nonfarm payroll employment rose by 64,000 in November, and the unemployment \
             rate for teenagers rose to 15.0 percent."
        ))
        .iter()
        .all(|(k, _)| *k != FigureKey::UnemploymentRate));
    }

    #[test]
    fn jobs_wording_variants() {
        let v = |s: &str| values(&parse_figures("jobs", s));
        assert_eq!(
            v("Total nonfarm payroll employment rose by 119,000 in September, and the unemployment \
               rate, at 4.4 percent, changed little, the U.S. Bureau of Labor Statistics reported today."),
            [
                (FigureKey::Payrolls, 119_000.0),
                (FigureKey::UnemploymentRate, 4.4)
            ]
        );
        assert_eq!(
            v(
                "Total nonfarm payroll employment changed little in August (+22,000), and the \
               unemployment rate was little changed at 4.3 percent, the U.S. Bureau reported."
            ),
            [
                (FigureKey::Payrolls, 22_000.0),
                (FigureKey::UnemploymentRate, 4.3)
            ]
        );
        assert_eq!(
            v("Total nonfarm payroll employment was essentially unchanged in June (-13,000). The \
               unemployment rate edged down to 4.1 percent."),
            [
                (FigureKey::Payrolls, -13_000.0),
                (FigureKey::UnemploymentRate, 4.1)
            ]
        );
        assert_eq!(
            v("Total nonfarm payroll employment fell by 20.5 million in April, and the unemployment \
               rate rose to 14.7 percent."),
            [
                (FigureKey::Payrolls, -20_500_000.0),
                (FigureKey::UnemploymentRate, 14.7)
            ]
        );
    }

    #[test]
    fn a_page_that_is_not_the_release_gives_nothing() {
        for page in [
            "<title>Access Denied</title>",
            "<nav>Consumer Price Index</nav> Page Not Found",
            "",
            // A changed sentence shape is not guessed at.
            "The Consumer Price Index for All Urban Consumers (CPI-U) moved 0.4 percent on a \
             seasonally adjusted basis in October.",
            "Total nonfarm payroll employment rose sharply in October.",
        ] {
            assert!(parse_figures("cpi", page).is_empty(), "{page}");
            assert!(parse_figures("jobs", page).is_empty(), "{page}");
        }
        assert!(parse_figures("fomc", CPI).is_empty());
        // A sentence that never ends within a sentence's length gives no figure.
        let long = format!(
            "The index for all items less food and energy rose 0.3 percent in October {}",
            "and more ".repeat(100)
        );
        assert!(parse_figures("cpi", &long).is_empty());
    }

    #[test]
    fn figures_render_in_both_languages_and_pass_the_number_check() {
        let fig = |key, value| ReleaseFigure {
            key,
            value,
            quote: String::new(),
        };
        let cpi = [
            fig(FigureKey::CpiMonth, 0.3),
            fig(FigureKey::CpiYear, 3.0),
            fig(FigureKey::CoreMonth, 0.2),
            fig(FigureKey::CoreYear, 3.0),
        ];
        let jobs = [
            fig(FigureKey::Payrolls, 119_000.0),
            fig(FigureKey::UnemploymentRate, 4.4),
        ];
        let cases = [
            (
                &cpi[..],
                Lang::En,
                "+0.3% on the month, +3.0% from a year earlier; core +0.2% on the month, +3.0% from a year earlier",
            ),
            (
                &cpi[..],
                Lang::Zh,
                "环比 +0.3%，同比 +3.0%；核心环比 +0.2%，同比 +3.0%",
            ),
            (&jobs[..], Lang::En, "Payrolls +119,000; unemployment rate 4.4%"),
            (&jobs[..], Lang::Zh, "非农就业 +11.9万人；失业率 4.4%"),
        ];
        for (f, lang, want) in cases {
            let text = figures_text(f, lang).unwrap();
            assert_eq!(text, want);
            let allowed: Vec<f64> = f.iter().map(|f| f.value).collect();
            assert_eq!(check(&text, lang, &allowed), Ok(()), "{text}");
        }
        let down = [
            fig(FigureKey::CpiMonth, 0.0),
            fig(FigureKey::CoreMonth, -0.1),
            fig(FigureKey::Payrolls, -13_000.0),
        ];
        assert_eq!(
            figures_text(&down, Lang::En).unwrap(),
            "unchanged on the month; core −0.1% on the month; Payrolls −13,000"
        );
        assert_eq!(
            figures_text(&down, Lang::Zh).unwrap(),
            "环比持平；核心环比 −0.1%；非农就业 −1.3万人"
        );
        for lang in [Lang::En, Lang::Zh] {
            let text = figures_text(&down, lang).unwrap();
            assert_eq!(
                check(&text, lang, &[0.0, -0.1, -13_000.0]),
                Ok(()),
                "{text}"
            );
        }
        assert_eq!(figures_text(&[], Lang::En), None);
    }

    #[test]
    fn figures_round_trip_through_json() {
        let f = parse_figures("cpi", CPI);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v[0]["key"], "cpi_month");
        let back: Vec<ReleaseFigure> = serde_json::from_value(v).unwrap();
        assert_eq!(back, f);
    }
}
