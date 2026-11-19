//! SEC EDGAR behind the `Filings` trait: the current 8-K filings feed,
//! filing indexes, filed documents and XBRL `companyconcept` documents.
//!
//! Fair access (https://www.sec.gov/os/webmaster-faq#developers): every
//! request carries a descriptive User-Agent with a contact address, and
//! requests are sequential with a pause between them so one invocation stays
//! under 10 requests/second.

use std::cell::Cell;
use std::sync::OnceLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Deserialize;

use crate::anchor::decode_entities;
use crate::earnings::cik10;
use crate::llm::one_line;
use crate::providers::{Http, HttpResp, ProviderError, Timer};

/// Pause before every EDGAR request after the first: at most 8 requests/second.
pub const GAP_MS: u64 = 125;
const _: () = assert!(
    GAP_MS >= 100,
    "EDGAR fair access: at most 10 requests/second"
);
/// The build queue's pause: a build runs beside a poll (its own queue), so
/// the two together stay under 10 requests/second.
pub const BUILD_GAP_MS: u64 = 250;
const ARCHIVES: &str = "https://www.sec.gov/Archives/edgar/data/";
/// Largest EDGAR response read; a bigger body fails as `TooLarge` without
/// being read (and parsed) in full. A press release is the exception: its
/// first `MAX_BYTES` are kept (filings styled inline run past 3 MB, mostly
/// `style` attributes, and the extraction reads only the opening text).
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// One entry of EDGAR's current-filings feed (`getcurrent`, Atom).
#[derive(Debug, Clone, PartialEq)]
pub struct FeedEntry {
    /// `8-K` or `8-K/A`.
    pub form: String,
    pub cik: u64,
    pub company: String,
    pub accession: String,
    /// EDGAR's filing date, `YYYY-MM-DD`.
    pub filing_date: String,
    /// 8-K item numbers, e.g. `["2.02", "9.01"]`.
    pub items: Vec<String>,
    /// The entry's `updated`: the acceptance time (ET with its offset).
    pub accepted_at: DateTime<Utc>,
    /// The filing index page.
    pub index_url: String,
}

/// A document listed in a filing index.
#[derive(Debug, Clone, PartialEq)]
pub struct FilingDoc {
    pub name: String,
    /// The index's Description column ("FY2027 SEMI-ANNUAL FINANCIAL SUMMARY").
    pub description: String,
    /// EDGAR document type, e.g. `8-K`, `EX-99.1`.
    pub doc_type: String,
    /// Absolute `https://www.sec.gov/Archives/...` URL.
    pub url: String,
}

/// One XBRL concept of one company (`companyconcept`): only the unit
/// tables the trend reads; labels and descriptions are skipped.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Concept {
    #[serde(default)]
    pub units: Units,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Units {
    #[serde(rename = "USD", default)]
    pub usd: Vec<FactEntry>,
    #[serde(rename = "USD/shares", default)]
    pub usd_per_share: Vec<FactEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FactEntry {
    /// Absent for instant facts.
    pub start: Option<String>,
    pub end: String,
    pub val: f64,
    #[serde(default)]
    pub form: String,
    #[serde(default)]
    pub filed: String,
}

#[async_trait(?Send)]
pub trait Filings {
    /// One page (`FEED_PAGE` entries from `start`, newest first) of the
    /// current 8-K filings feed.
    async fn current_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError>;
    async fn filing_docs(&self, cik: u64, accession: &str)
        -> Result<Vec<FilingDoc>, ProviderError>;
    /// One page of the current 6-K filings feed (foreign private issuers),
    /// like `current_filings`. No 6-K filings unless implemented.
    async fn current_6k_filings(&self, _start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
        Ok(vec![])
    }
    /// The first `max` bytes of a filed document as plain text: enough to
    /// recognise a release without reading (and parsing) all of it.
    async fn document_head(&self, url: &str, _max: usize) -> Result<String, ProviderError> {
        self.document_text(url).await
    }
    /// A filed document as plain text (`html_to_text`).
    async fn document_text(&self, url: &str) -> Result<String, ProviderError>;
    /// One us-gaap concept, or `None` when the company never tagged it (404).
    async fn company_concept(&self, cik: u64, tag: &str) -> Result<Option<Concept>, ProviderError>;
}

/// `https://data.sec.gov/api/xbrl/companyconcept/CIK##########/us-gaap/<tag>.json`.
pub fn concept_url(cik: u64, tag: &str) -> String {
    format!(
        "https://data.sec.gov/api/xbrl/companyconcept/CIK{}/us-gaap/{tag}.json",
        cik10(cik)
    )
}

pub struct Edgar<H: Http, T: Timer> {
    pub http: H,
    pub timer: T,
    user_agent: String,
    used: Cell<bool>,
    gap_ms: u64,
}

impl<H: Http, T: Timer> Edgar<H, T> {
    /// `contact` is the `SEC_CONTACT` email; SEC blocks anonymous clients.
    pub fn new(http: H, timer: T, contact: &str) -> Result<Self, ProviderError> {
        let contact = contact.trim();
        if contact.is_empty() {
            return Err(ProviderError::Parse("SEC_CONTACT is empty".into()));
        }
        Ok(Self {
            http,
            timer,
            user_agent: format!("PreBell/0.1 (+https://prebell.site; {contact})"),
            used: Cell::new(false),
            gap_ms: GAP_MS,
        })
    }

    /// A longer pause between requests (never shorter than `GAP_MS`).
    pub fn paced(mut self, gap_ms: u64) -> Self {
        self.gap_ms = gap_ms.max(GAP_MS);
        self
    }

    async fn get(&self, url: &str) -> Result<String, ProviderError> {
        Ok(self.fetch(url, false).await?.body)
    }

    /// A JSON document (the IPO check's submissions), capped at `MAX_BYTES`.
    pub(crate) async fn get_json(&self, url: &str) -> Result<String, ProviderError> {
        self.get(url).await
    }

    /// `prefix`: keep the first `MAX_BYTES` of a bigger body instead of failing.
    async fn fetch(&self, url: &str, prefix: bool) -> Result<HttpResp, ProviderError> {
        self.fetch_max(url, prefix, MAX_BYTES).await
    }

    /// The reply, when its status is 200.
    async fn fetch_max(
        &self,
        url: &str,
        prefix: bool,
        max: usize,
    ) -> Result<HttpResp, ProviderError> {
        if self.used.replace(true) {
            self.timer
                .sleep(std::time::Duration::from_millis(self.gap_ms))
                .await;
        }
        let headers = [
            ("User-Agent", self.user_agent.as_str()),
            ("Accept-Encoding", "gzip, deflate"),
        ];
        let r = if prefix {
            self.http.get_prefix(url, &headers, max).await?
        } else {
            self.http.get_capped(url, &headers, max).await?
        };
        if r.status != 200 {
            return Err(ProviderError::Http(r.status));
        }
        Ok(r)
    }
}

/// Bytes of a reply's body (or Content-Type, or title) read for an error's
/// text: a 2 MB body costs no more CPU than a short one.
const REPLY_SCAN_BYTES: usize = 512;
/// Characters of a reply's body in an error.
const REPLY_BODY_CHARS: usize = 80;
/// Characters of a reply's body in an error when its title was found.
const REPLY_BODY_CHARS_TITLED: usize = 40;
/// Characters of a reply's Content-Type in an error.
const REPLY_TYPE_CHARS: usize = 60;
/// Characters of a non-feed page's title in an error.
const REPLY_TITLE_CHARS: usize = 80;
/// Bytes of a non-feed body searched for its `<title>`: SEC's HTML pages
/// spend their first 80 characters on DOCTYPE and head boilerplate.
const TITLE_SCAN_BYTES: usize = 4096;
/// Longest text inside `NotFeed`'s parentheses. The stored alert, `SEC EDGAR
/// sent a non-feed reply: current 6-K filings page 0: parse error: not an
/// Atom feed (<text>)`, is then at most 300 characters (`alert_once_today`'s
/// cap, so it is never cut there), and the log line, `earnings poll: current
/// 6-K filings page 0: parse error: not an Atom feed (<text>)`, at most 282.
/// A longer text is cut at its end, so `body began` goes first, then the title.
const NOT_FEED_TEXT_CHARS: usize = 207;

/// The largest char boundary of `s` at or below `max` bytes.
fn char_floor(s: &str, max: usize) -> usize {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// At most `chars` characters from the start of `s`, untrusted text from the
/// network: on one line and sanitised like a model reply's snippet
/// (`llm::one_line`: control, bidirectional and zero-width characters as
/// `?`, links inert), with `…` where it was cut. Never interpreted.
fn reply_text(s: &str, chars: usize) -> String {
    let end = char_floor(s, REPLY_SCAN_BYTES);
    let flat = one_line(&s[..end]);
    if end == s.len() && flat.chars().count() <= chars {
        return flat;
    }
    let head: String = flat.chars().take(chars).collect();
    format!("{head}…")
}

/// `s` (already `reply_text`) in double quotes, with its own `"` as `'` and
/// `)` as `]`, so a header or title cannot close the quote or the error's
/// parentheses and pass for the rest of the line.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'").replace(')', "]"))
}

/// The text between the first `<title` (to its `>`) and the `</title` after
/// it, both inside the first `TITLE_SCAN_BYTES` of `body`, found by plain
/// ASCII-case-insensitive substring search. No entity decoding, no nesting,
/// nothing else read; `None` when the pair is not there.
fn page_title(body: &str) -> Option<&str> {
    let w = &body[..char_floor(body, TITLE_SCAN_BYTES)];
    // Lowercasing ASCII keeps every byte offset.
    let lower = w.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    Some(&w[start..end])
}

/// What came back, for an error's text:
/// `status 200, content-type "text/html; charset=utf-8", 5120 bytes`.
fn reply_facts(r: &HttpResp) -> String {
    let ct = match r.content_type.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => {
            format!("content-type {}", quoted(&reply_text(t, REPLY_TYPE_CHARS)))
        }
        _ => "no content-type".into(),
    };
    format!("status {}, {ct}, {} bytes", r.status, r.body.len())
}

/// The entries of a current-filings feed reply. A reply that is not a feed
/// fails as `NotFeed`, saying what came back, in this order: status,
/// Content-Type, length, the page's title when it has one, how the body
/// began (40 characters after a title, else 80), cut to `NOT_FEED_TEXT_CHARS`.
fn feed_reply(r: &HttpResp) -> Result<Vec<FeedEntry>, ProviderError> {
    parse_current_feed(&r.body).map_err(|_| {
        let mut text = reply_facts(r);
        let title = page_title(&r.body)
            .map(|t| reply_text(t, REPLY_TITLE_CHARS))
            .filter(|t| !t.is_empty());
        if let Some(t) = &title {
            text.push_str(&format!(", title {}", quoted(t)));
        }
        let chars = if title.is_some() {
            REPLY_BODY_CHARS_TITLED
        } else {
            REPLY_BODY_CHARS
        };
        let began = reply_text(&r.body, chars);
        if !began.is_empty() {
            text.push_str(&format!(", body began: {began}"));
        }
        if text.chars().count() > NOT_FEED_TEXT_CHARS {
            text = text.chars().take(NOT_FEED_TEXT_CHARS - 1).collect();
            text.push('…');
        }
        ProviderError::NotFeed(text)
    })
}

/// A 200 reply with nothing usable in it (`EmptyReply`).
fn empty_reply(r: &HttpResp) -> ProviderError {
    ProviderError::EmptyReply(reply_facts(r))
}

/// A press release cut at `MAX_BYTES` ends mid-element, possibly mid-number
/// ("Total revenues 8,8"), and a cut cell must never verify. A body that
/// reached the cap is trimmed to its last complete paragraph, row or block.
fn whole_blocks(mut html: String) -> String {
    // `get_prefix` drops at most 3 bytes of a split character.
    if html.len() + 4 <= MAX_BYTES {
        return html;
    }
    let lower = html.to_ascii_lowercase();
    let end = ["</p>", "</tr>", "</div>", "</table>"]
        .iter()
        .filter_map(|tag| lower.rfind(tag).map(|i| i + tag.len()))
        .max()
        .unwrap_or(0);
    html.truncate(end);
    html
}

/// `0000320193-26-000070` -> `000032019326000070`.
fn acc_nodash(accession: &str) -> String {
    accession.replace('-', "")
}

/// The filing index page posts link to as "the filing".
pub fn filing_url(cik: u64, accession: &str) -> String {
    format!(
        "{ARCHIVES}{cik}/{}/{accession}-index.htm",
        acc_nodash(accession)
    )
}

/// Entries per page of the current-filings feed.
pub const FEED_PAGE: usize = 100;

/// The current 8-K filings feed, newest first, `FEED_PAGE` entries from `start`.
pub fn current_feed_url(start: usize) -> String {
    current_form_feed_url("8-K", start)
}

/// The current filings feed of one form (`8-K`, `6-K`; amendments included).
pub fn current_form_feed_url(form: &str, start: usize) -> String {
    format!(
        "https://www.sec.gov/cgi-bin/browse-edgar?action=getcurrent&type={form}&company=&dateb=&owner=include&start={start}&count={FEED_PAGE}&output=atom"
    )
}

/// Plain text of the opening of a document cut at a byte limit: the cut
/// half-tag is dropped first.
pub fn head_text(html: &str) -> String {
    quick_text(&html[..html.rfind('>').map_or(0, |i| i + 1)])
}

/// Tags that style text inside a word or line, not separate it.
const INLINE_TAGS: [&str; 19] = [
    "a", "abbr", "b", "big", "code", "del", "em", "font", "i", "ins", "s", "small", "span",
    "strike", "strong", "sub", "sup", "tt", "u",
];

/// `tag` (lowercase, from its `<`) opens or closes an inline tag.
fn is_inline_tag(tag: &str) -> bool {
    let name = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    INLINE_TAGS.contains(&name)
}

/// Plain text of HTML on one line, without regexes: scripts, styles,
/// comments, the head and the hidden inline-XBRL header dropped, every tag
/// but an inline one a space, entities decoded, whitespace collapsed. For text that is matched
/// against, not quoted: a cold Worker spends several ms compiling
/// `html_to_text`'s patterns, and the 6-K reader has 10 ms of CPU in all.
pub fn quick_text(html: &str) -> String {
    let bytes = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let mut text_from = 0;
    while let Some(off) = html[i..].find('<') {
        let at = i + off;
        out.push_str(&html[text_from..at]);
        let rest = &lower[at..];
        // Inline tags join their text ("L<span>ondon</span>" is "London");
        // any other tag separates words.
        if !is_inline_tag(rest) {
            out.push(' ');
        }
        let skip_to = |close: &str| {
            rest.find(close)
                .map_or(html.len(), |e| at + e + close.len())
        };
        i = if rest.starts_with("<!--") {
            skip_to("-->")
        } else if ["<script", "<style", "<head", "<ix:header"]
            .iter()
            .any(|t| {
                rest.starts_with(t)
                    && !rest[t.len()..].starts_with(|c: char| c.is_ascii_alphanumeric())
            })
        {
            let name = &rest[1..rest
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(rest.len())];
            let end = format!("</{name}");
            match rest.find(&end) {
                Some(e) => {
                    let after = at + e;
                    lower[after..]
                        .find('>')
                        .map_or(html.len(), |g| after + g + 1)
                }
                None => html.len(),
            }
        } else {
            // To the tag's '>', outside quoted attribute values.
            let mut j = at + 1;
            let mut quote = 0u8;
            while j < bytes.len() {
                let b = bytes[j];
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    break;
                }
                j += 1;
            }
            (j + 1).min(html.len())
        };
        text_from = i;
    }
    out.push_str(&html[text_from.min(html.len())..]);
    decode_entities(&out)
        .replace(
            [
                '\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}', '\u{a0}',
            ],
            " ",
        )
        .replace('\u{ad}', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text between `<tag ...>` and `</tag>` in `s`.
fn element<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = s.find(&format!("<{tag}"))?;
    let body = open + s[open..].find('>')? + 1;
    let end = body + s[body..].find(&format!("</{tag}>"))?;
    Some(&s[body..end])
}

/// The value of `attr="..."` in `s`.
fn attr<'a>(s: &'a str, attr: &str) -> Option<&'a str> {
    let key = format!("{attr}=\"");
    let at = s.find(&key)? + key.len();
    Some(&s[at..at + s[at..].find('"')?])
}

/// One `<entry>`; `None` when a field is missing or malformed.
fn feed_entry(e: &str) -> Option<FeedEntry> {
    // "8-K - Apple Inc. (0000320193) (Filer)"
    let title = decode_entities(element(e, "title")?);
    let (form, rest) = title.split_once(" - ")?;
    let open = rest
        .rfind(" (Filer)")
        .and_then(|end| rest[..end].rfind(" (").map(|o| (o, end)))?;
    let cik = rest[open.0 + 2..open.1].strip_suffix(')')?.parse().ok()?;
    let company = rest[..open.0].trim().to_string();
    let summary = decode_entities(element(e, "summary")?);
    let filed = summary
        .split("Filed:</b>")
        .nth(1)?
        .split_whitespace()
        .next()?;
    let items = summary
        .split("Item ")
        .skip(1)
        .filter_map(|s| s.split(':').next())
        .map(str::trim)
        .filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(String::from)
        .collect();
    let id = element(e, "id")?;
    let accession = id.split("accession-number=").nth(1)?.trim().to_string();
    let accepted_at = DateTime::parse_from_rfc3339(element(e, "updated")?.trim())
        .ok()?
        .with_timezone(&Utc);
    let link = e.find("<link")?;
    let index_url = attr(&e[link..], "href")?.to_string();
    if !index_url.starts_with(ARCHIVES) || accession.len() != 20 {
        return None;
    }
    Some(FeedEntry {
        form: form.trim().to_string(),
        cik,
        company,
        accession,
        filing_date: filed.to_string(),
        items,
        accepted_at,
        index_url,
    })
}

/// Entries of one current-filings feed page, in feed order. Plain string
/// scanning of a ~70 KB page: no JSON or XML tree, so a poll's CPU stays
/// well inside the Free plan's 10 ms.
pub fn parse_current_feed(xml: &str) -> Result<Vec<FeedEntry>, ProviderError> {
    if !xml.contains("<feed") {
        return Err(ProviderError::Parse("not an Atom feed".into()));
    }
    Ok(xml
        .split("<entry>")
        .skip(1)
        .filter_map(|e| feed_entry(e.split("</entry>").next()?))
        .collect())
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

/// Rows of the `-index.htm` document tables: (Seq, Description, Document, Type, Size).
pub fn parse_filing_index(html: &str) -> Vec<FilingDoc> {
    static ROW: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static HREF: OnceLock<Regex> = OnceLock::new();
    let row = ROW.get_or_init(|| re(r"(?is)<tr[^>]*>(.*?)</tr>"));
    let cell = CELL.get_or_init(|| re(r"(?is)<td[^>]*>(.*?)</td>"));
    let href = HREF.get_or_init(|| re(r#"(?is)<a\s[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#));
    let mut out = Vec::new();
    for r in row.captures_iter(html) {
        let cells: Vec<&str> = cell
            .captures_iter(&r[1])
            .map(|c| c.get(1).map_or("", |m| m.as_str()))
            .collect();
        if cells.len() < 4 {
            continue;
        }
        let Some(a) = href.captures(cells[2]) else {
            continue;
        };
        // Inline XBRL documents link through the viewer: `/ix?doc=/Archives/...`.
        let path = a[1].trim_start_matches("/ix?doc=");
        if !path.starts_with("/Archives/edgar/data/") {
            continue;
        }
        out.push(FilingDoc {
            name: quick_text(&a[2]),
            description: quick_text(cells[1]),
            doc_type: quick_text(cells[3]),
            url: format!("https://www.sec.gov{path}"),
        });
    }
    out
}

/// The press release: type `EX-99.1`, else the first `EX-99*` exhibit.
pub fn exhibit_991(docs: &[FilingDoc]) -> Option<&FilingDoc> {
    let is_doc = |d: &&FilingDoc| {
        let n = d.name.to_ascii_lowercase();
        n.ends_with(".htm") || n.ends_with(".html") || n.ends_with(".txt")
    };
    docs.iter()
        .filter(is_doc)
        .find(|d| d.doc_type.eq_ignore_ascii_case("EX-99.1"))
        .or_else(|| {
            docs.iter()
                .filter(is_doc)
                .find(|d| d.doc_type.to_ascii_uppercase().starts_with("EX-99"))
        })
}

/// Plain text of an HTML document: scripts, styles, comments and the hidden
/// inline-XBRL header dropped; each table row on one line (block tags inside
/// cells become spaces), other block ends become line breaks; entities
/// decoded; zero-width characters (SPG pads empty cells with "&#8203;")
/// read as spaces and soft hyphens dropped; each line's whitespace
/// collapsed; blank lines removed. Signs and
/// units that filer software puts in their own cells are re-attached
/// ("$ (0.25 )" -> "$(0.25)", "47.2 %" -> "47.2%").
pub fn html_to_text(html: &str) -> String {
    static DROP: OnceLock<Regex> = OnceLock::new();
    static ROW: OnceLock<Regex> = OnceLock::new();
    static BLOCK: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    static GLUE_AFTER: OnceLock<Regex> = OnceLock::new();
    static GLUE_BEFORE: OnceLock<Regex> = OnceLock::new();
    let drop = DROP.get_or_init(|| {
        re(r"(?is)<script\b.*?</script\s*>|<style\b.*?</style\s*>|<!--.*?-->|<head\b.*?</head\s*>|<ix:header\b.*?</ix:header\s*>")
    });
    let row = ROW.get_or_init(|| re(r"(?is)<tr\b.*?</tr\s*>"));
    let block = BLOCK.get_or_init(|| {
        re(r"(?i)<br\s*/?>|</(?:p|div|tr|li|h[1-6]|table|title)\s*>|<(?:p|div|tr|li|table)\b[^>]*>")
    });
    let cell = CELL.get_or_init(|| re(r"(?i)</t[dh]\s*>"));
    let tag = TAG.get_or_init(|| re(r"(?s)<[^>]*>"));
    let glue_after = GLUE_AFTER.get_or_init(|| re(r"([$(]) +([\d(])"));
    let glue_before = GLUE_BEFORE.get_or_init(|| re(r"(\d) +([)%])"));
    let s = drop.replace_all(html, " ");
    let s = row.replace_all(&s, |c: &regex::Captures| {
        let one_line = block.replace_all(&c[0], " ").replace(['\n', '\r'], " ");
        format!("\n{one_line}\n")
    });
    let s = block.replace_all(&s, "\n");
    let s = cell.replace_all(&s, " ");
    let s = tag.replace_all(&s, "");
    decode_entities(&s)
        .replace(
            ['\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}'],
            " ",
        )
        .replace('\u{ad}', "")
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = glue_after.replace_all(&l, "$1$2");
            let l = glue_after.replace_all(&l, "$1$2");
            glue_before.replace_all(&l, "$1$2").into_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait(?Send)]
impl<H: Http, T: Timer> Filings for Edgar<H, T> {
    async fn current_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
        feed_reply(&self.fetch(&current_feed_url(start), false).await?)
    }

    async fn current_6k_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
        feed_reply(
            &self
                .fetch(&current_form_feed_url("6-K", start), false)
                .await?,
        )
    }

    async fn document_head(&self, url: &str, max: usize) -> Result<String, ProviderError> {
        if !url.starts_with(ARCHIVES) {
            return Err(ProviderError::Parse(format!(
                "not an EDGAR archive url: {url}"
            )));
        }
        let r = self.fetch_max(url, true, max).await?;
        let text = head_text(&r.body);
        if text.is_empty() {
            return Err(empty_reply(&r));
        }
        Ok(text)
    }

    async fn filing_docs(
        &self,
        cik: u64,
        accession: &str,
    ) -> Result<Vec<FilingDoc>, ProviderError> {
        let r = self.fetch(&filing_url(cik, accession), false).await?;
        let docs = parse_filing_index(&r.body);
        if docs.is_empty() {
            return Err(empty_reply(&r));
        }
        Ok(docs)
    }

    async fn document_text(&self, url: &str) -> Result<String, ProviderError> {
        if !url.starts_with(ARCHIVES) {
            return Err(ProviderError::Parse(format!(
                "not an EDGAR archive url: {url}"
            )));
        }
        let r = self.fetch(url, true).await?;
        let empty = empty_reply(&r);
        let text = html_to_text(&whole_blocks(r.body));
        if text.is_empty() {
            return Err(empty);
        }
        Ok(text)
    }

    async fn company_concept(&self, cik: u64, tag: &str) -> Result<Option<Concept>, ProviderError> {
        if !tag.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ProviderError::Parse(format!("bad concept tag {tag:?}")));
        }
        match self.get(&concept_url(cik, tag)).await {
            Ok(body) => serde_json::from_str(&body)
                .map(Some)
                .map_err(|e| ProviderError::Parse(e.to_string())),
            Err(ProviderError::Http(404)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub(crate) const FEED: &str = include_str!("../tests/fixtures/edgar_current_8k.atom");
    pub(crate) const INDEX: &str = include_str!("../tests/fixtures/edgar_index_AAPL.htm");
    pub(crate) const EX991: &str = include_str!("../tests/fixtures/edgar_ex991_AAPL.htm");

    type Headers = Vec<(String, String)>;

    /// URL -> (status, body); records each request's URL and headers.
    /// Every reply carries `content_type`.
    #[derive(Default)]
    pub(crate) struct FakeHttp {
        pub routes: HashMap<String, (u16, String)>,
        pub seen: RefCell<Vec<(String, Headers)>>,
        pub content_type: Option<String>,
    }
    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
        ) -> Result<HttpResp, ProviderError> {
            self.seen.borrow_mut().push((
                url.into(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            let (status, body) = self
                .routes
                .get(url)
                .cloned()
                .unwrap_or((404, "Not Found".into()));
            Ok(HttpResp {
                status,
                body,
                set_cookie: None,
                content_type: self.content_type.clone(),
            })
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            unreachable!("EDGAR only GETs")
        }
    }

    /// Records each sleep in milliseconds.
    #[derive(Default)]
    pub(crate) struct RecTimer(pub RefCell<Vec<u64>>);
    #[async_trait(?Send)]
    impl Timer for RecTimer {
        async fn sleep(&self, d: std::time::Duration) {
            self.0.borrow_mut().push(d.as_millis() as u64);
        }
    }

    const FEED_URL: &str = "https://www.sec.gov/cgi-bin/browse-edgar?action=getcurrent&type=8-K&company=&dateb=&owner=include&start=0&count=100&output=atom";
    const IDX_URL: &str = "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm";
    const EX_URL: &str = "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/a8-kex991q4202609262026.htm";
    const CC_URL: &str = "https://data.sec.gov/api/xbrl/companyconcept/CIK0000320193/us-gaap/EarningsPerShareDiluted.json";

    fn edgar(routes: &[(&str, u16, &str)]) -> Edgar<FakeHttp, RecTimer> {
        let http = FakeHttp {
            routes: routes
                .iter()
                .map(|(u, s, b)| (u.to_string(), (*s, b.to_string())))
                .collect(),
            ..Default::default()
        };
        Edgar::new(http, RecTimer::default(), "ops@prebell.site").unwrap()
    }

    #[test]
    fn feed_entries_parse() {
        let f = parse_current_feed(FEED).unwrap();
        assert_eq!(f.len(), 4);
        let a = &f[0];
        assert_eq!(a.form, "8-K");
        assert_eq!(a.cik, 320193);
        assert_eq!(a.company, "Apple Inc.");
        assert_eq!(a.accession, "0000320193-26-000070");
        assert_eq!(a.filing_date, "2026-10-30");
        assert_eq!(a.items, ["2.02", "9.01"]);
        assert_eq!(a.index_url, IDX_URL);
        // `updated` is the acceptance time in ET with its offset: 16:30:41
        // EDT is 20:30:41Z (it matches the index page's "Accepted").
        assert_eq!(
            a.accepted_at,
            "2026-10-30T20:30:41Z".parse::<DateTime<Utc>>().unwrap()
        );
        // Real entries: Meta (no 2.02), an 8-K/A, an entity decoded.
        let meta = &f[2];
        assert_eq!(
            (meta.cik, meta.company.as_str()),
            (1326801, "Meta Platforms, Inc.")
        );
        assert_eq!(meta.items, ["1.01", "3.03", "5.03", "9.01"]);
        assert_eq!(
            meta.accepted_at,
            "2026-10-29T21:08:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(f[3].form, "8-K/A");
        let w = parse_current_feed(&FEED.replace("KOSS CORP", "W&amp;T OFFSHORE INC")).unwrap();
        assert_eq!(w[1].company, "W&T OFFSHORE INC");
    }

    #[test]
    fn feed_shapes() {
        // An empty page is fine; a page that is not a feed is an error.
        let empty = "<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><title>Latest Filings</title></feed>";
        assert_eq!(parse_current_feed(empty), Ok(vec![]));
        assert!(matches!(
            parse_current_feed("<html>maintenance</html>"),
            Err(ProviderError::Parse(_))
        ));
        // An entry missing a field is skipped, not guessed.
        let broken = FEED.replacen("<updated>2026-10-30T16:30:41-04:00</updated>", "", 1);
        assert_eq!(parse_current_feed(&broken).unwrap().len(), 3);
    }

    #[test]
    fn feed_url_pages() {
        assert_eq!(current_feed_url(0), FEED_URL);
        assert!(current_feed_url(200).contains("&start=200&count=100&"));
    }

    #[test]
    fn quick_text_joins_inline_tags_and_splits_blocks() {
        assert_eq!(
            quick_text("<p>L<span style=\"x\">ondon</span>, <b>August 26</b>, 2026</p><p>Next</p>"),
            "London, August 26, 2026 Next"
        );
        assert_eq!(
            quick_text("<td>Revenue</td><td>RMB 1,234</td>"),
            "Revenue RMB 1,234"
        );
        assert_eq!(
            quick_text("a<br>b<div>c</div><!-- x -->d<script>e</script>f"),
            "a b c d f"
        );
        assert_eq!(quick_text("<header>x</header><a href='y>z'>k</a>"), "x k");
    }

    #[tokio::test]
    async fn user_agent_and_pacing() {
        let e = edgar(&[
            (FEED_URL, 200, FEED),
            (IDX_URL, 200, INDEX),
            (EX_URL, 200, EX991),
        ]);
        assert_eq!(e.current_filings(0).await.unwrap().len(), 4);
        e.filing_docs(320193, "0000320193-26-000070").await.unwrap();
        e.document_text(EX_URL).await.unwrap();
        let seen = e.http.seen.borrow();
        assert_eq!(seen.len(), 3);
        for (_, h) in seen.iter() {
            let ua = h.iter().find(|(k, _)| k == "User-Agent").unwrap();
            assert_eq!(
                ua.1,
                "PreBell/0.1 (+https://prebell.site; ops@prebell.site)"
            );
        }
        // No pause before the first request, one before each later one.
        assert_eq!(*e.timer.0.borrow(), vec![GAP_MS, GAP_MS]);
    }

    #[tokio::test]
    async fn a_build_client_paces_slower() {
        let e = edgar(&[(IDX_URL, 200, INDEX), (EX_URL, 200, EX991)]).paced(BUILD_GAP_MS);
        e.filing_docs(320193, "0000320193-26-000070").await.unwrap();
        e.document_text(EX_URL).await.unwrap();
        assert_eq!(*e.timer.0.borrow(), vec![BUILD_GAP_MS]);
        // Never faster than fair access allows.
        let e = edgar(&[(IDX_URL, 200, INDEX), (EX_URL, 200, EX991)]).paced(10);
        e.filing_docs(320193, "0000320193-26-000070").await.unwrap();
        e.document_text(EX_URL).await.unwrap();
        assert_eq!(*e.timer.0.borrow(), vec![GAP_MS]);
    }

    #[tokio::test]
    async fn oversized_bodies_are_not_read() {
        let big = "x".repeat(MAX_BYTES + 1);
        let e = edgar(&[(FEED_URL, 200, &big)]);
        assert_eq!(
            e.current_filings(0).await,
            Err(ProviderError::TooLarge(MAX_BYTES))
        );
    }

    #[tokio::test]
    async fn http_errors() {
        let e = edgar(&[(FEED_URL, 403, "Request Rate Threshold Exceeded")]);
        assert_eq!(e.current_filings(0).await, Err(ProviderError::Http(403)));
        let e = edgar(&[(FEED_URL, 429, "")]);
        assert_eq!(e.current_filings(0).await, Err(ProviderError::Http(429)));
        let e = edgar(&[(FEED_URL, 200, "<html>maintenance</html>")]);
        assert!(matches!(
            e.current_filings(0).await,
            Err(ProviderError::NotFeed(_))
        ));
    }

    /// `edgar` whose replies carry `content_type`.
    fn typed(
        routes: &[(&str, u16, &str)],
        content_type: Option<&str>,
    ) -> Edgar<FakeHttp, RecTimer> {
        let mut e = edgar(routes);
        e.http.content_type = content_type.map(String::from);
        e
    }

    /// A feed reply that is not a feed says what came back: status,
    /// Content-Type, body length, the page's title when it has one, and how
    /// the body began (2026-11-19: 9 polls in 12 logged only "not an Atom
    /// feed").
    #[tokio::test]
    async fn a_non_feed_reply_says_what_came_back() {
        let six_k = current_form_feed_url("6-K", 0);
        let err = |body: &str, ct: Option<&str>| {
            let e = typed(&[(FEED_URL, 200, body), (&six_k, 200, body)], ct);
            async move {
                let a = e.current_filings(0).await.unwrap_err().to_string();
                let b = e.current_6k_filings(0).await.unwrap_err().to_string();
                assert_eq!(a, b, "both feeds read the same way");
                a
            }
        };
        let html = "<!DOCTYPE html>\n<html><head><title>SEC.gov | Request Rate Threshold Exceeded</title></head><body>Your request rate is too high.</body></html>";
        assert_eq!(
            err(html, Some("text/html; charset=utf-8")).await,
            format!(
                "parse error: not an Atom feed (status 200, content-type \"text/html; charset=utf-8\", {} bytes, title \"SEC.gov | Request Rate Threshold Exceeded\", body began: <!DOCTYPE html> <html><head><title>SEC.g…)",
                html.len()
            )
        );
        assert_eq!(
            err("", None).await,
            "parse error: not an Atom feed (status 200, no content-type, 0 bytes)"
        );
        assert_eq!(
            err("{\"error\":\"service unavailable\"}", Some("application/json")).await,
            "parse error: not an Atom feed (status 200, content-type \"application/json\", 31 bytes, body began: {\"error\":\"service unavailable\"})"
        );
        // A real feed is still read.
        let e = typed(&[(FEED_URL, 200, FEED)], Some("application/atom+xml"));
        assert_eq!(e.current_filings(0).await.unwrap().len(), 4);
    }

    /// The body is untrusted network text: it reaches the log and the alert
    /// webhook only on one line, with links inert and zero-width characters
    /// replaced (as a model reply's snippet, PR #74), cut to 80 characters,
    /// and only its first `REPLY_SCAN_BYTES` are looked at.
    #[tokio::test]
    async fn a_hostile_non_feed_body_is_sanitised_and_cut() {
        let err = |body: String, ct: &str| {
            let e = typed(&[(FEED_URL, 200, &body)], Some(ct));
            async move { e.current_filings(0).await.unwrap_err().to_string() }
        };
        let m = err(
            "See https://evil.test/a\r\n\tand [click](http://x.test) n\u{200b}o\u{2060}w\u{feff}\u{202e}!".into(),
            "text/html",
        )
        .await;
        assert!(
            m.ends_with(
                "body began: See https[:]//evil.test/a and [click] (http[:]//x.test) n?o?w??!)"
            ),
            "{m}"
        );
        assert!(!m.contains("://") && !m.contains("](") && !m.contains('\n') && !m.contains('\r'));
        // A very long body: 80 characters, its full length in bytes.
        let long = format!("<html>{}", "a".repeat(MAX_BYTES - 6));
        let m = err(long.clone(), "text/html").await;
        assert!(m.contains(&format!("{} bytes", long.len())), "{m}");
        let began = m.split("body began: ").nth(1).unwrap();
        assert_eq!(began.chars().count(), 80 + "…)".chars().count(), "{m}");
        // A long, hostile Content-Type is sanitised and cut too.
        let ct = format!("text/html; x=\"https://evil.test\"\n{}", "b".repeat(400));
        let m = err(format!("<p>{}</p>", "\u{4e2d}".repeat(1000)), &ct).await;
        assert!(
            m.contains("content-type \"text/html; x='https[:]//evil.test' bbb"),
            "{m}"
        );
        assert!(!m.contains('\n') && !m.contains("://"), "{m}");
    }

    /// A Content-Type cannot pass for the rest of the line: its `"` and `)`
    /// are replaced, so it never closes its quote or the parentheses.
    #[tokio::test]
    async fn a_content_type_cannot_imitate_the_line() {
        let e = typed(
            &[(FEED_URL, 200, "<p>down</p>")],
            Some("text/html\", 0 bytes, body began: fake) (x"),
        );
        assert_eq!(
            e.current_filings(0).await.unwrap_err().to_string(),
            "parse error: not an Atom feed (status 200, content-type \"text/html', 0 bytes, body began: fake] (x\", 11 bytes, body began: <p>down</p>)"
        );
    }

    /// The title is found by plain, ASCII-case-insensitive substring search
    /// in the first 4096 bytes: a `<title` (to its `>`) and a `</title` after
    /// it. Nothing is decoded or parsed.
    #[test]
    fn page_title_is_a_plain_search_in_the_first_4096_bytes() {
        assert_eq!(page_title("<html><body>x</body></html>"), None);
        // Unclosed.
        assert_eq!(page_title("<html><head><title>Maintenance"), None);
        assert_eq!(page_title("<html><head><title"), None);
        // Mixed case, with attributes; entities stay as they are.
        assert_eq!(
            page_title("<HTML><HEAD><TiTlE lang=\"en\">Down &amp; out</tItLe>"),
            Some("Down &amp; out")
        );
        // Past the first 4096 bytes, wholly or only its end.
        assert_eq!(
            page_title(&format!("{}<title>Late</title>", " ".repeat(4096))),
            None
        );
        assert_eq!(
            page_title(&format!("<title>{}</title>", "a".repeat(4090))),
            None
        );
        // Non-ASCII at the 4096-byte cut: never split, no panic. `</title`
        // ends at byte 4095 here, inside the window...
        let t = format!("x{}", "\u{4e2d}".repeat(1360));
        assert_eq!(page_title(&format!("<title>{t}</title>")), Some(t.as_str()));
        // ...and here 4096 falls inside a character and `</title` is beyond it.
        assert_eq!(
            page_title(&format!("<title>x{}</title>", "\u{4e2d}".repeat(1363))),
            None
        );
    }

    /// A non-feed page's title: after a long DOCTYPE it is still found, and
    /// the body's start is then 40 characters. Without a title (none,
    /// unclosed, past 4096 bytes) the body's start is 80 characters.
    #[tokio::test]
    async fn a_non_feed_page_says_its_title() {
        let err = |body: String| {
            let e = typed(&[(FEED_URL, 200, &body)], Some("text/html"));
            async move { e.current_filings(0).await.unwrap_err().to_string() }
        };
        let page = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\">\n<head>\n<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\" />\n<TITLE>SEC.gov | Request Rate Threshold Exceeded</TITLE>\n</head><body><h1>Your Request Originates from an Undeclared Automated Tool</h1></body></html>";
        assert_eq!(
            err(page.into()).await,
            format!(
                "parse error: not an Atom feed (status 200, content-type \"text/html\", {} bytes, title \"SEC.gov | Request Rate Threshold Exceeded\", body began: <!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML…)",
                page.len()
            )
        );
        // No title, an unclosed one, one past 4096 bytes: 80 characters.
        for body in [
            "<html><body>Service Unavailable. Please try again later; we apologise for the inconvenience caused.</body></html>".to_string(),
            "<html><head><title>Service Unavailable. Please try again later; we apologise for the inconvenience".to_string(),
            format!("<html>{}<title>Late</title></html>", "x".repeat(4096)),
        ] {
            let m = err(body).await;
            assert!(!m.contains("title \""), "{m}");
            let began = m.split("body began: ").nth(1).unwrap();
            assert_eq!(began.chars().count(), 80 + "…)".chars().count(), "{m}");
        }
    }

    /// A hostile title is sanitised like the body (links inert, zero-width
    /// and control characters replaced, one line), cut to 80 characters,
    /// and cannot close its quote or the parentheses. A 5,000-character
    /// title does not close inside the first 4096 bytes, so it is no title.
    #[tokio::test]
    async fn a_hostile_title_is_sanitised_and_cut() {
        let err = |body: String| {
            let e = typed(&[(FEED_URL, 200, &body)], Some("text/html"));
            async move { e.current_filings(0).await.unwrap_err().to_string() }
        };
        let hostile = "See https://evil.test [click](http://x.test)\r\n\tz\u{200b}w\u{2060}j\u{feff}\u{202e}\" ), body began: fake ";
        let m = err(format!("<title>{hostile}{}</title>", "A".repeat(3000))).await;
        let title = m
            .split(", title \"")
            .nth(1)
            .and_then(|t| t.split("\", body began: ").next())
            .unwrap();
        assert_eq!(
            title,
            format!(
                "See https[:]//evil.test [click] (http[:]//x.test] z?w?j??' ], body began: fake A…"
            ),
            "{m}"
        );
        assert_eq!(title.chars().count(), 80 + 1, "{m}");
        assert!(!m.contains("://") && !m.contains("](") && !m.contains('\n') && !m.contains('\r'));
        // 5,000 characters: not closed within 4096 bytes, so no title.
        let m = err(format!("<title>{hostile}{}</title>", "A".repeat(5000))).await;
        assert!(!m.contains(", title \""), "{m}");
        assert!(
            m.contains("body began: <title>See https[:]//evil.test"),
            "{m}"
        );
    }

    /// The worst case, a hostile Content-Type, title and body in a 2 MB reply:
    /// the error text inside the parentheses is cut at its end (body first,
    /// then the title), so the log line is at most 282 characters, the stored
    /// alert at most 300 (`alert_once_today` never cuts it) and the webhook
    /// message (`error: sec-not-feed:<date>: ` before it) at most 332.
    #[tokio::test]
    async fn the_worst_case_fits_the_log_and_the_alert() {
        let ct = format!("text/html; {}", "\u{4e2d}\")".repeat(200));
        let body = format!(
            "<title>{}</title>{}",
            "T\u{200b}\"".repeat(800),
            "x".repeat(MAX_BYTES - 7 - 4000 - 8)
        );
        assert_eq!(body.len(), MAX_BYTES);
        let e = typed(&[(FEED_URL, 200, &body)], Some(&ct));
        let m = e.current_filings(0).await.unwrap_err().to_string();
        let inner = m
            .strip_prefix("parse error: not an Atom feed (")
            .and_then(|r| r.strip_suffix(')'))
            .unwrap();
        assert_eq!(inner.chars().count(), NOT_FEED_TEXT_CHARS, "{m}");
        assert!(inner.ends_with('…'), "{m}");
        // Status, Content-Type, length and the title's start survive.
        assert!(
            inner.starts_with("status 200, content-type \"text/html; "),
            "{m}"
        );
        assert!(inner.contains("\", 2097152 bytes, title \"T?'T?'"), "{m}");
        let log = format!("earnings poll: current 6-K filings page 0: {m}");
        let alert = format!("SEC EDGAR sent a non-feed reply: current 6-K filings page 0: {m}");
        let hook = format!("error: sec-not-feed:2026-11-19: {alert}");
        assert_eq!(log.chars().count(), 282, "{log}");
        assert_eq!(alert.chars().count(), 300, "{alert}");
        assert_eq!(hook.chars().count(), 332, "{hook}");
    }

    /// An index or document with nothing usable in it says what came back.
    #[tokio::test]
    async fn an_empty_result_says_what_came_back() {
        let page = "<html><body>No documents</body></html>";
        let e = typed(
            &[(IDX_URL, 200, page), (EX_URL, 200, "")],
            Some("text/html"),
        );
        let facts = format!(
            "status 200, content-type \"text/html\", {} bytes",
            page.len()
        );
        assert_eq!(
            e.filing_docs(320193, "0000320193-26-000070").await,
            Err(ProviderError::EmptyReply(facts.clone()))
        );
        assert_eq!(
            e.filing_docs(320193, "0000320193-26-000070")
                .await
                .unwrap_err()
                .to_string(),
            format!("empty result ({facts})")
        );
        assert_eq!(
            e.document_text(EX_URL).await.unwrap_err().to_string(),
            "empty result (status 200, content-type \"text/html\", 0 bytes)"
        );
        assert_eq!(
            e.document_head(EX_URL, 4096).await.unwrap_err().to_string(),
            "empty result (status 200, content-type \"text/html\", 0 bytes)"
        );
    }

    #[test]
    fn empty_contact_rejected() {
        assert!(Edgar::new(FakeHttp::default(), RecTimer::default(), "  ").is_err());
    }

    #[tokio::test]
    async fn index_finds_exhibit_991() {
        let e = edgar(&[(IDX_URL, 200, INDEX)]);
        let docs = e.filing_docs(320193, "0000320193-26-000070").await.unwrap();
        assert_eq!(docs[0].doc_type, "8-K");
        assert_eq!(
            docs[0].url,
            "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/aapl-20261030.htm"
        );
        let ex = exhibit_991(&docs).unwrap();
        assert_eq!(ex.url, EX_URL);
        assert_eq!(ex.name, "a8-kex991q4202609262026.htm");
        assert!(docs.iter().any(|d| d.doc_type == "EX-101.SCH"));
    }

    #[test]
    fn exhibit_fallback_and_absence() {
        let doc = |name: &str, t: &str| FilingDoc {
            description: String::new(),
            name: name.into(),
            doc_type: t.into(),
            url: format!("{ARCHIVES}1/2/{name}"),
        };
        let docs = vec![
            doc("a.htm", "8-K"),
            doc("pr.htm", "EX-99.2"),
            doc("x.jpg", "EX-99.1"),
        ];
        assert_eq!(exhibit_991(&docs).unwrap().name, "pr.htm");
        assert!(exhibit_991(&[doc("a.htm", "8-K")]).is_none());
    }

    #[test]
    fn html_text() {
        let t = html_to_text(EX991);
        assert!(t.contains("Total net sales $102,466 $94,930"), "{t}");
        assert!(t.contains("Diluted $1.85 $0.97"), "{t}");
        assert!(t.contains("ended September 26, 2026"), "nbsp: {t}");
        assert!(t.contains("fiscal 2026 fourth quarter ended September 26, 2026"));
        assert!(t.contains("Company\u{2019}s CEO"));
        assert!(t.contains("$102.5 billion"));
        assert!(!t.contains("999.9"), "script text leaked");
        assert!(!t.contains("margin: 0"), "style text leaked");
        assert!(!t.contains('<'));
    }

    #[test]
    fn html_text_real_filer_markup() {
        let html = r#"<div style="display:none"><ix:header><xbrli:context id="c1">2026-09-26</xbrli:context></ix:header></div>
<table><tr><td><p style="margin:0"><span>Total net sales</span></p></td><td><div><p>$</p></div></td>
<td><p><ix:nonFraction name="us-gaap:Revenues" scale="6">102,466</ix:nonFraction></p></td></tr>
<tr><td><p>Diluted</p></td><td>$</td><td>(0.25</td><td>)</td></tr>
<tr><td>Gross margin</td><td>47.2</td><td>%</td></tr></table>"#;
        let t = html_to_text(html);
        assert_eq!(
            t,
            "Total net sales $102,466\nDiluted $(0.25)\nGross margin 47.2%"
        );
        // SPG pads every empty cell with a zero-width space.
        let html = r#"<div>(Dollars in thousands, except per share amounts)<font>&#8203;<font> </font></font></div>
<table><tr><td>&#8203;</td><td>&#8203;</td><td colspan="10"><div><font>For the Three Months</font><br ><font>Ended September&#160;30, </font></div></td></tr>
<tr><td>&#8203;</td><td><div><font>Total revenue </font></div></td><td>&#8203;</td><td>&#8203;</td><td>1,814,556 </td><td>&#8203;</td></tr>
<tr><td>&#8203;</td><td>Net income</td><td>$</td><td>&#8203;</td><td>(8,259</td><td>&#8203;</td><td>)</td></tr>
<tr><td>&#8203;</td><td>&#8203;</td></tr></table>"#;
        let t = html_to_text(html);
        assert_eq!(
            t,
            "(Dollars in thousands, except per share amounts)\nFor the Three Months Ended September 30,\nTotal revenue 1,814,556\nNet income $(8,259)"
        );
        // Other zero-width characters space cells apart too; a soft hyphen
        // splits no word.
        let html = "<table><tr><td>Total&#8204;</td><td>&#8205;revenues</td><td>&#8288;</td><td>1,234</td></tr></table><p>Consoli&#173;dated net in\u{ad}come</p>";
        assert_eq!(
            html_to_text(html),
            "Total revenues 1,234\nConsolidated net income"
        );
    }

    #[tokio::test]
    async fn empty_index_is_empty() {
        let e = edgar(&[(IDX_URL, 200, "<html><body>No documents</body></html>")]);
        assert_eq!(
            e.filing_docs(320193, "0000320193-26-000070").await,
            Err(ProviderError::EmptyReply(
                "status 200, no content-type, 38 bytes".into()
            ))
        );
    }

    #[tokio::test]
    async fn a_large_press_release_keeps_its_opening() {
        let url = "https://www.sec.gov/Archives/edgar/data/1063761/000110465926123325/ex99-1.htm";
        let body = format!(
            "<p>Simon Property Group Reports Third Quarter 2026 Results</p>{}",
            format!("<p style=\"{}\">x</p>", "margin:0;".repeat(40)).repeat(MAX_BYTES / 300)
        );
        assert!(body.len() > MAX_BYTES);
        let e = edgar(&[(url, 200, &body)]);
        let text = e.document_text(url).await.unwrap();
        assert!(text.starts_with("Simon Property Group Reports Third Quarter 2026 Results"));
    }

    #[test]
    fn a_cut_press_release_ends_on_a_whole_row() {
        // The cap falls inside "8,893": the half row is dropped, not "8,8".
        let head = "<table><tr><td>Total revenues</td><td>1,234</td></tr>".to_string();
        let filler = "x".repeat(MAX_BYTES - head.len() - "<tr><td>Net income</td><td>8,8".len());
        let cut = format!("{head}{filler}<tr><td>Net income</td><td>8,8");
        assert_eq!(cut.len(), MAX_BYTES);
        let kept = whole_blocks(cut);
        assert!(kept.ends_with("</tr>"), "{}", &kept[kept.len() - 20..]);
        assert!(!html_to_text(&kept).contains("8,8"));
        // A body under the cap is untouched.
        assert_eq!(whole_blocks("<p>a 8,8".into()), "<p>a 8,8");
    }

    #[tokio::test]
    async fn document_text_only_from_archives() {
        let e = edgar(&[]);
        assert!(matches!(
            e.document_text("https://evil.test/x.htm").await,
            Err(ProviderError::Parse(_))
        ));
        assert!(e.http.seen.borrow().is_empty());
    }

    #[tokio::test]
    async fn company_concept_reads_units_and_404_is_none() {
        let e = edgar(&[(CC_URL, 200, crate::earnings::xbrl::tests::EPS)]);
        let c = e
            .company_concept(320193, "EarningsPerShareDiluted")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(c.units.usd_per_share.len(), 7);
        assert!(c.units.usd.is_empty());
        // Never tagged: 404.
        assert_eq!(e.company_concept(320193, "Revenues").await, Ok(None));
        let seen: Vec<String> = e.http.seen.borrow().iter().map(|s| s.0.clone()).collect();
        assert_eq!(seen[0], CC_URL);
        assert!(seen[1].ends_with("/CIK0000320193/us-gaap/Revenues.json"));
        // Throttling and oversized bodies are errors, not "never tagged".
        let e = edgar(&[(CC_URL, 429, "")]);
        assert_eq!(
            e.company_concept(320193, "EarningsPerShareDiluted").await,
            Err(ProviderError::Http(429))
        );
        let big = format!("{{\"units\":{{}},\"x\":\"{}\"}}", "x".repeat(MAX_BYTES));
        let e = edgar(&[(CC_URL, 200, &big)]);
        assert_eq!(
            e.company_concept(320193, "EarningsPerShareDiluted").await,
            Err(ProviderError::TooLarge(MAX_BYTES))
        );
        // Tags are path segments: nothing but letters and digits.
        assert!(e.company_concept(320193, "../x").await.is_err());
    }

    #[test]
    fn filing_url_format() {
        assert_eq!(filing_url(320193, "0000320193-26-000070"), IDX_URL);
    }
}
