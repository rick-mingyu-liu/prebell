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
use crate::providers::{Http, ProviderError, Timer};

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
        self.fetch(url, false).await
    }

    /// `prefix`: keep the first `MAX_BYTES` of a bigger body instead of failing.
    async fn fetch(&self, url: &str, prefix: bool) -> Result<String, ProviderError> {
        self.fetch_max(url, prefix, MAX_BYTES).await
    }

    async fn fetch_max(
        &self,
        url: &str,
        prefix: bool,
        max: usize,
    ) -> Result<String, ProviderError> {
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
        Ok(r.body)
    }
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
        parse_current_feed(&self.get(&current_feed_url(start)).await?)
    }

    async fn current_6k_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
        parse_current_feed(&self.get(&current_form_feed_url("6-K", start)).await?)
    }

    async fn document_head(&self, url: &str, max: usize) -> Result<String, ProviderError> {
        if !url.starts_with(ARCHIVES) {
            return Err(ProviderError::Parse(format!(
                "not an EDGAR archive url: {url}"
            )));
        }
        let text = head_text(&self.fetch_max(url, true, max).await?);
        if text.is_empty() {
            return Err(ProviderError::Empty);
        }
        Ok(text)
    }

    async fn filing_docs(
        &self,
        cik: u64,
        accession: &str,
    ) -> Result<Vec<FilingDoc>, ProviderError> {
        let docs = parse_filing_index(&self.get(&filing_url(cik, accession)).await?);
        if docs.is_empty() {
            return Err(ProviderError::Empty);
        }
        Ok(docs)
    }

    async fn document_text(&self, url: &str) -> Result<String, ProviderError> {
        if !url.starts_with(ARCHIVES) {
            return Err(ProviderError::Parse(format!(
                "not an EDGAR archive url: {url}"
            )));
        }
        let text = html_to_text(&whole_blocks(self.fetch(url, true).await?));
        if text.is_empty() {
            return Err(ProviderError::Empty);
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
    #[derive(Default)]
    pub(crate) struct FakeHttp {
        pub routes: HashMap<String, (u16, String)>,
        pub seen: RefCell<Vec<(String, Headers)>>,
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
            Err(ProviderError::Parse(_))
        ));
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
            Err(ProviderError::Empty)
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
