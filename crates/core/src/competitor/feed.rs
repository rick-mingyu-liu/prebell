//! A small RSS 2.0 reader for the competitor feed. It returns metadata only:
//! `FeedItem` has no field for descriptions, `content:encoded` or authors,
//! so they can never be stored.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    /// `pubDate` (RFC 2822); `None` when missing or unparseable.
    pub published: Option<DateTime<Utc>>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FeedError {
    #[error("not an RSS feed")]
    NotRss,
}

/// Parses `<rss><channel><item>…` into items. Items without a title or link
/// are dropped. Markup inside CDATA or comments is never read as a tag.
pub fn parse_feed(xml: &str) -> Result<Vec<FeedItem>, FeedError> {
    let channel = children(xml)
        .into_iter()
        .find(|(name, _)| *name == "rss")
        .and_then(|(_, rss)| children(rss).into_iter().find(|(n, _)| *n == "channel"))
        .ok_or(FeedError::NotRss)?
        .1;
    Ok(children(channel)
        .into_iter()
        .filter(|(name, _)| *name == "item")
        .filter_map(|(_, inner)| item(inner))
        .collect())
}

fn item(inner: &str) -> Option<FeedItem> {
    let fields = children(inner);
    let first = |name: &str| {
        fields
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| text(v))
    };
    let title = first("title").filter(|t| !t.is_empty())?;
    // Only http(s) links: they become hrefs on the admin page. A `guid`
    // permalink stands in when `<link>` is missing.
    let web = |u: &String| u.starts_with("https://") || u.starts_with("http://");
    let link = first("link")
        .filter(web)
        .or_else(|| first("guid").filter(web))?;
    let published = first("pubDate").and_then(|d| rfc2822(&d));
    let categories = fields
        .iter()
        .filter(|(n, _)| *n == "category")
        .map(|(_, v)| text(v))
        .filter(|c| !c.is_empty())
        .collect();
    Some(FeedItem {
        title,
        link,
        published,
        categories,
    })
}

/// RFC 2822 date; a wrong day-of-week name (common in hand-built feeds) is
/// ignored rather than failing the whole date.
fn rfc2822(d: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(d)
        .or_else(|e| match d.split_once(", ") {
            Some((_, rest)) => DateTime::parse_from_rfc2822(rest),
            None => Err(e),
        })
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Length of `s` up to and including the first `end`, or `None` if absent.
fn past(s: &str, end: &str) -> Option<usize> {
    s.find(end).map(|i| i + end.len())
}

/// If `s` (starting at `<`) opens something that is not an element (CDATA,
/// comment, processing instruction, doctype), the length to skip.
fn skip_non_element(s: &str) -> Option<Option<usize>> {
    let end = if s.starts_with("<![CDATA[") {
        "]]>"
    } else if s.starts_with("<!--") {
        "-->"
    } else if s.starts_with("<?") {
        "?>"
    } else if s.starts_with("<!") {
        ">"
    } else {
        return None;
    };
    Some(past(s, end))
}

/// Direct child elements of `s` as `(name, inner)`. Stops at the first
/// malformed or unterminated element.
fn children(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = s[i..].find('<') {
        let at = i + off;
        let rest = &s[at..];
        if let Some(skip) = skip_non_element(rest) {
            let Some(n) = skip else { break };
            i = at + n;
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        if tag.starts_with('/') {
            i = at + gt + 1; // stray end tag
            continue;
        }
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let open_end = at + gt + 1;
        if tag.ends_with('/') {
            out.push((name, ""));
            i = open_end;
            continue;
        }
        let Some((close_at, after)) = find_close(s, open_end) else {
            break;
        };
        out.push((name, &s[open_end..close_at]));
        i = after;
    }
    out
}

/// From just after a start tag, finds the matching end tag by depth.
/// Returns (start of the end tag, index just past it).
fn find_close(s: &str, from: usize) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    let mut i = from;
    while let Some(off) = s[i..].find('<') {
        let at = i + off;
        let rest = &s[at..];
        if let Some(skip) = skip_non_element(rest) {
            i = at + skip?;
            continue;
        }
        let gt = rest.find('>')?;
        let tag = &rest[1..gt];
        if tag.starts_with('/') {
            depth -= 1;
            if depth == 0 {
                return Some((at, at + gt + 1));
            }
        } else if !tag.ends_with('/') {
            depth += 1;
        }
        i = at + gt + 1;
    }
    None
}

/// Text content: CDATA verbatim, entities decoded elsewhere, nested tags
/// dropped, whitespace collapsed.
fn text(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let Some(lt) = rest.find('<') else {
            out.push_str(&decode_entities(rest));
            break;
        };
        out.push_str(&decode_entities(&rest[..lt]));
        let tag = &rest[lt..];
        if let Some(body) = tag.strip_prefix("<![CDATA[") {
            let end = body.find("]]>").unwrap_or(body.len());
            out.push_str(&body[..end]);
            i += lt + "<![CDATA[".len() + (end + 3).min(body.len());
        } else {
            let skip = skip_non_element(tag)
                .flatten()
                .or_else(|| past(tag, ">"))
                .unwrap_or(tag.len());
            i += lt + skip;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        // Entities are short; look for `;` only nearby so a long run of bare
        // `&` stays linear.
        let near = &tail[..tail.len().min(12)];
        let decoded = near.find(';').and_then(|semi| {
            let ent = &tail[1..semi];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent
                    .strip_prefix("#x")
                    .or_else(|| ent.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32)
                    .filter(|&c| c != '\0'),
            };
            ch.map(|c| (c, semi + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/alphalab_feed.xml");

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_fixture_items() {
        let items = parse_feed(FIXTURE).unwrap();
        assert_eq!(items.len(), 5, "{items:#?}");
        let first = &items[0];
        assert_eq!(
            first.title,
            "Fixture: MSFT cloud growth check and $NVDA supply notes"
        );
        assert_eq!(
            first.link,
            "https://www.alphalab.site/posts/fixture-msft-nvda"
        );
        assert_eq!(first.published, Some(utc("2026-10-29T11:05:00Z")));
        assert_eq!(first.categories, vec!["Earnings", "Tech"]);
    }

    #[test]
    fn gmt_dates_and_decoys_in_cdata() {
        let items = parse_feed(FIXTURE).unwrap();
        let klac = &items[1];
        assert_eq!(klac.title, "Fixture: KLA (KLAC) quarterly health check");
        assert_eq!(klac.categories, vec!["Earnings"]);
        assert_eq!(klac.published, Some(utc("2026-10-29T02:30:00Z")));
        assert!(items.iter().all(|i| i.title != "Decoy title"
            && !i.categories.iter().any(|c| c == "Decoy")
            && i.title != "not an item"));
    }

    #[test]
    fn entities_bad_dates_and_dropped_items() {
        let items = parse_feed(FIXTURE).unwrap();
        assert_eq!(items[2].title, "Fixture: S&P 500 weekly macro digest");
        assert_eq!(items[3].title, "Fixture: 台积电（TSM）与微软MSFT展望");
        assert_eq!(items[3].published, None);
        assert!(items.iter().all(|i| !i.title.contains("without a link")));
        assert_eq!(items[4].categories, Vec::<String>::new());
    }

    #[test]
    fn never_returns_bodies() {
        let items = parse_feed(FIXTURE).unwrap();
        let all = format!("{items:?}");
        assert!(!all.contains("Invented"));
        assert!(!all.contains("Fixture Author"));
    }

    #[test]
    fn html_is_not_rss() {
        assert_eq!(
            parse_feed("<html><body>blocked</body></html>"),
            Err(FeedError::NotRss)
        );
        assert_eq!(parse_feed(""), Err(FeedError::NotRss));
    }

    #[test]
    fn wrong_weekday_still_parses() {
        assert_eq!(
            rfc2822("Mon, 29 Oct 2026 02:30:00 GMT"),
            Some(utc("2026-10-29T02:30:00Z"))
        );
        assert_eq!(rfc2822("not a date"), None);
    }

    #[test]
    fn guid_permalink_stands_in_for_link() {
        let xml = "<rss><channel><item><title>T</title><guid>https://x.test/a</guid></item>\
                   <item><title>U</title><guid>tag-123</guid></item></channel></rss>";
        let items = parse_feed(xml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].link, "https://x.test/a");
    }

    #[test]
    fn non_web_links_are_dropped() {
        let xml = "<rss><channel><item><title>T</title><link>javascript:alert(1)</link></item>\
                   <item><title>U</title><link>http://x.test/u</link></item></channel></rss>";
        let items = parse_feed(xml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].link, "http://x.test/u");
    }

    #[test]
    fn numeric_entities() {
        assert_eq!(text("A&#38;B &#x4E2D; &bogus; & x"), "A&B 中 &bogus; & x");
        assert_eq!(text("a&#0;b"), "a&#0;b");
        assert_eq!(text(&"& ".repeat(50_000)).len(), 99_999);
    }

    #[test]
    fn truncated_feed_is_not_rss() {
        let cut = &FIXTURE[..FIXTURE.find("fixture-macro").unwrap()];
        // The channel itself is unterminated, so nothing is readable.
        assert_eq!(parse_feed(cut), Err(FeedError::NotRss));
    }
}
