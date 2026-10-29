//! The daily gap report: tickers and topics the competitor covered in the
//! window that we did not, and its webhook digest.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::clock::et;

/// `alerts::deliver` caps a webhook message at 1900 chars including its
/// `"{level}: {run_key}: "` prefix; this leaves room for it.
pub const DIGEST_MAX_CHARS: usize = 1850;

/// A `competitor_items` row inside the window.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StoredItem {
    pub link: String,
    pub title: String,
    /// UTC ISO 8601 with `Z`.
    pub published_at: String,
    pub category: String,
    pub tickers: Vec<String>,
}

/// One of our published posts: EN title + summary + body + source titles.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OurPost {
    pub slug: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ItemRef {
    pub title: String,
    pub link: String,
    pub category: String,
    pub published_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TickerGap {
    pub ticker: String,
    /// Their items naming the ticker, newest first.
    pub items: Vec<ItemRef>,
}

/// Stored as `competitor_reports.report_json`; read by the admin page.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Report {
    /// ET date, `YYYY-MM-DD`.
    pub report_date: String,
    pub window_start: String,
    pub window_end: String,
    pub their_item_count: usize,
    /// Start of our side of the comparison (see `our_window_start`); it
    /// ends at `window_end` too.
    pub our_window_start: String,
    pub our_post_count: usize,
    pub our_slugs: Vec<String>,
    /// Their tickers we did not mention, most-covered first.
    pub uncovered: Vec<TickerGap>,
    /// Their tickers we also mentioned, sorted.
    pub covered: Vec<String>,
    /// Their items with no ticker in the title, newest first. Code can't
    /// match these; they are listed for a human.
    pub topics: Vec<ItemRef>,
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn item_ref(i: &StoredItem) -> ItemRef {
    ItemRef {
        title: i.title.clone(),
        link: i.link.clone(),
        category: i.category.clone(),
        published_at: i.published_at.clone(),
    }
}

/// Compares their items published in `[window_start, window_end)` with our
/// posts. A ticker counts as ours if `tickers_in` finds it in a post's text.
pub fn build_report(
    report_date: NaiveDate,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    our_window_start: DateTime<Utc>,
    theirs: &[StoredItem],
    ours: &[OurPost],
) -> Report {
    let in_window = |i: &&StoredItem| {
        DateTime::parse_from_rfc3339(&i.published_at)
            .is_ok_and(|t| t >= window_start && t < window_end)
    };
    let mut theirs: Vec<&StoredItem> = theirs.iter().filter(in_window).collect();
    theirs.sort_by(|a, b| b.published_at.cmp(&a.published_at));

    // Their tickers are matched as whole words in our text, so our `| F |`
    // table cell covers their `$F` even though a bare `F` is not a ticker.
    let ours_mention = |t: &str| ours.iter().any(|p| mentions(&p.text, t));

    let mut by_ticker: BTreeMap<&str, Vec<ItemRef>> = BTreeMap::new();
    let mut topics = Vec::new();
    for i in &theirs {
        if i.tickers.is_empty() {
            topics.push(item_ref(i));
        }
        for t in &i.tickers {
            by_ticker.entry(t).or_default().push(item_ref(i));
        }
    }
    let covered: Vec<String> = by_ticker
        .keys()
        .filter(|t| ours_mention(t))
        .map(|t| t.to_string())
        .collect();
    let mut uncovered: Vec<TickerGap> = by_ticker
        .into_iter()
        .filter(|(t, _)| !ours_mention(t))
        .map(|(t, items)| TickerGap {
            ticker: t.to_string(),
            items,
        })
        .collect();
    // Stable sort keeps the BTreeMap's alphabetical order within a count.
    uncovered.sort_by_key(|g| std::cmp::Reverse(g.items.len()));

    Report {
        report_date: report_date.format("%Y-%m-%d").to_string(),
        window_start: iso(window_start),
        window_end: iso(window_end),
        their_item_count: theirs.len(),
        our_window_start: iso(our_window_start),
        our_post_count: ours.len(),
        our_slugs: ours.iter().map(|p| p.slug.clone()).collect(),
        uncovered,
        covered,
        topics,
    }
}

/// `ticker` appears in `text` with no `[A-Za-z0-9.]` on either side.
fn mentions(text: &str, ticker: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '.';
    text.match_indices(ticker).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        // A trailing sentence dot (`… NVDA.`) still ends the word.
        let after = text[i + ticker.len()..]
            .trim_start_matches('.')
            .chars()
            .next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// At most `max` characters, with `…` when cut. `@` gets a zero-width space
/// so a title like `@everyone` can't ping the webhook's channel.
fn cut(s: &str, max: usize) -> String {
    let s = match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    };
    s.replace('@', "@\u{200b}")
}

/// One webhook message of at most `DIGEST_MAX_CHARS` characters. Entries
/// that don't fit are counted in a closing `…and N more` line.
pub fn digest(r: &Report) -> String {
    let end_et = DateTime::parse_from_rfc3339(&r.window_end)
        .map(|t| {
            et(t.with_timezone(&Utc))
                .format("%Y-%m-%d %H:%M ET")
                .to_string()
        })
        .unwrap_or_else(|_| r.window_end.clone());
    let since = DateTime::parse_from_rfc3339(&r.our_window_start)
        .map(|t| et(t.with_timezone(&Utc)).format("%a %m-%d").to_string())
        .unwrap_or_else(|_| r.our_window_start.clone());
    let ours = format!(
        "{} post{} from us since {since}",
        r.our_post_count,
        if r.our_post_count == 1 { "" } else { "s" }
    );
    if r.their_item_count == 0 {
        return format!(
            "Competitor watch {}: no items from them in the 24h to {end_et}; {ours}.",
            r.report_date
        );
    }
    let header = format!(
        "Competitor watch {} (24h to {end_et}): {} item{} from them, {ours}.",
        r.report_date,
        r.their_item_count,
        if r.their_item_count == 1 { "" } else { "s" }
    );

    // (line, is an entry counted by "…and N more")
    let mut lines: Vec<(String, bool)> = Vec::new();
    if !r.uncovered.is_empty() {
        lines.push((format!("Not covered by us ({}):", r.uncovered.len()), false));
        for g in &r.uncovered {
            let n = if g.items.len() > 1 {
                format!(" ×{}", g.items.len())
            } else {
                String::new()
            };
            let Some(first) = g.items.first() else {
                continue;
            };
            lines.push((
                format!(
                    "- {}{n}: {} <{}>",
                    g.ticker,
                    cut(&first.title, 80),
                    first.link
                ),
                true,
            ));
        }
    }
    if !r.topics.is_empty() {
        lines.push((
            format!("Topics without tickers ({}):", r.topics.len()),
            false,
        ));
        for t in &r.topics {
            let cat = if t.category.is_empty() {
                String::new()
            } else {
                format!("[{}] ", cut(&t.category, 40))
            };
            lines.push((format!("- {cat}{} <{}>", cut(&t.title, 80), t.link), true));
        }
    }
    if !r.covered.is_empty() {
        lines.push((
            format!("Covered by both: {}", cut(&r.covered.join(", "), 300)),
            false,
        ));
    }

    let total: usize = lines.iter().map(|(l, _)| l.chars().count() + 1).sum();
    if header.chars().count() + total <= DIGEST_MAX_CHARS {
        let mut out = header;
        for (l, _) in lines {
            out.push('\n');
            out.push_str(&l);
        }
        return out;
    }

    const MORE_RESERVE: usize = 60; // "\n…and NNNN more (see /admin/competitor)"
    let mut out = header;
    let mut used = out.chars().count();
    let mut shown = 0;
    let mut stopped = false;
    for (l, entry) in &lines {
        let len = l.chars().count() + 1;
        if stopped || used + len > DIGEST_MAX_CHARS - MORE_RESERVE {
            stopped = true;
            continue;
        }
        out.push('\n');
        out.push_str(l);
        used += len;
        shown += usize::from(*entry);
    }
    let entries = lines.iter().filter(|(_, e)| *e).count();
    if entries > shown {
        out.push_str(&format!(
            "\n…and {} more (see /admin/competitor)",
            entries - shown
        ));
    } else {
        out.push_str("\n…(cut; see /admin/competitor)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn item(n: u32, tickers: &[&str], at: &str) -> StoredItem {
        StoredItem {
            link: format!("https://comp.test/{n}"),
            title: format!("Fixture title {n}"),
            published_at: at.into(),
            category: "Earnings".into(),
            tickers: tickers.iter().map(|t| t.to_string()).collect(),
        }
    }
    fn ours() -> Vec<OurPost> {
        vec![OurPost {
            slug: "premarket-2026-10-29".into(),
            text: "# Brief\n\n| Ticker | Company |\n|---|---|\n| NVDA | Nvidia |\n".into(),
        }]
    }
    fn report(theirs: &[StoredItem]) -> Report {
        build_report(
            NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
            utc("2026-10-29T11:30:00Z"),
            utc("2026-10-30T11:30:00Z"),
            utc("2026-10-29T04:00:00Z"),
            theirs,
            &ours(),
        )
    }

    #[test]
    fn gaps_covered_and_topics() {
        let theirs = vec![
            item(1, &["KLAC"], "2026-10-29T20:00:00Z"),
            item(2, &["MSFT", "NVDA"], "2026-10-29T21:00:00Z"),
            item(3, &["MSFT"], "2026-10-30T01:00:00Z"),
            item(4, &[], "2026-10-30T02:00:00Z"),
        ];
        let r = report(&theirs);
        let gaps: Vec<&str> = r.uncovered.iter().map(|g| g.ticker.as_str()).collect();
        assert_eq!(gaps, ["MSFT", "KLAC"]);
        assert_eq!(r.uncovered[0].items[0].link, "https://comp.test/3"); // newest first
        assert_eq!(r.covered, ["NVDA"]);
        assert_eq!(r.topics.len(), 1);
        assert_eq!(r.topics[0].link, "https://comp.test/4");
        assert_eq!(r.their_item_count, 4);
        assert_eq!(r.our_post_count, 1);
        assert_eq!(r.our_slugs, ["premarket-2026-10-29"]);
        assert_eq!(r.report_date, "2026-10-30");
        assert_eq!(r.window_start, "2026-10-29T11:30:00Z");

        let d = digest(&r);
        assert!(d.contains("MSFT ×2"), "{d}");
        assert!(d.contains("<https://comp.test/1>"), "{d}");
        assert!(d.contains("Covered by both: NVDA"), "{d}");
        assert!(d.contains("[Earnings] Fixture title 4"), "{d}");
    }

    #[test]
    fn items_outside_window_are_ignored() {
        let r = report(&[
            item(1, &["AMD"], "2026-10-29T11:29:59Z"),
            item(2, &["INTC"], "2026-10-30T11:30:00Z"),
            item(3, &["ARM"], "garbage"),
        ]);
        assert_eq!(r.their_item_count, 0);
        assert!(r.uncovered.is_empty());
    }

    #[test]
    fn digest_fits_cap() {
        let theirs: Vec<StoredItem> = (0..300)
            .map(|n| {
                let t = format!(
                    "Z{}{}",
                    (b'A' + (n / 26) as u8) as char,
                    (b'A' + (n % 26) as u8) as char
                );
                item(n, &[&t], "2026-10-29T20:00:00Z")
            })
            .collect();
        let r = report(&theirs);
        assert_eq!(r.uncovered.len(), 300);
        let d = digest(&r);
        assert!(
            d.chars().count() <= DIGEST_MAX_CHARS,
            "{}",
            d.chars().count()
        );
        let last = d.lines().last().unwrap();
        assert!(last.starts_with("…and "), "{last}");
        let more: usize = last["…and ".len()..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let listed = d.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(more + listed, 300);
    }

    #[test]
    fn long_titles_are_cut() {
        let mut it = item(1, &["MSFT"], "2026-10-29T20:00:00Z");
        it.title = "x".repeat(500);
        let d = digest(&report(&[it]));
        assert!(!d.contains(&"x".repeat(81)));
    }

    #[test]
    fn whole_word_coverage() {
        assert!(mentions("| F | Ford |", "F"));
        assert!(mentions("Shares of NVDA.", "NVDA"));
        assert!(mentions("BRK.B rose", "BRK.B"));
        assert!(!mentions("FORD", "F"));
        assert!(!mentions("BRK.B", "BRK"));
        assert!(!mentions("NVDAX", "NVDA"));
        let r = report(&[item(1, &["F", "IT"], "2026-10-29T20:00:00Z")]);
        assert!(r.covered.is_empty());
        let r = build_report(
            NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
            utc("2026-10-29T11:30:00Z"),
            utc("2026-10-30T11:30:00Z"),
            utc("2026-10-29T04:00:00Z"),
            &[item(1, &["F", "IT"], "2026-10-29T20:00:00Z")],
            &[OurPost {
                slug: "s".into(),
                text: "| F | Ford |\n| IT | Gartner |".into(),
            }],
        );
        assert_eq!(r.covered, ["F", "IT"]);
    }

    #[test]
    fn mentions_are_defused() {
        let mut it = item(1, &[], "2026-10-29T20:00:00Z");
        it.title = "Hi @everyone".into();
        it.category = "@here".into();
        let d = digest(&report(&[it]));
        assert!(!d.contains("@everyone") && !d.contains("@here"), "{d}");
    }

    #[test]
    fn empty_digest() {
        let d = digest(&report(&[]));
        assert!(d.contains("no items"), "{d}");
    }

    #[test]
    fn json_round_trip() {
        let r = report(&[item(1, &["KLAC"], "2026-10-29T20:00:00Z")]);
        let back: Report = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
