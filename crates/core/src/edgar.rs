//! SEC EDGAR behind the `Filings` trait: company submissions, filing
//! indexes, filed documents and XBRL `companyconcept` documents.
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
const ARCHIVES: &str = "https://www.sec.gov/Archives/edgar/data/";
/// Largest EDGAR response read; a bigger body fails as `TooLarge` without
/// being read (and parsed) in full.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// One row of a submissions document's `filings.recent` table.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentFiling {
    pub accession: String,
    pub form: String,
    /// Comma-separated 8-K items, e.g. `"2.02,9.01"`; empty for other forms.
    pub items: String,
    pub filing_date: String,
    pub report_date: String,
    pub accepted_at: DateTime<Utc>,
    pub primary_doc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Submissions {
    pub cik: u64,
    pub name: String,
    pub tickers: Vec<String>,
    pub recent: Vec<RecentFiling>,
}

/// A document listed in a filing index.
#[derive(Debug, Clone, PartialEq)]
pub struct FilingDoc {
    pub name: String,
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
    async fn submissions(&self, cik: u64) -> Result<Submissions, ProviderError>;
    async fn filing_docs(&self, cik: u64, accession: &str)
        -> Result<Vec<FilingDoc>, ProviderError>;
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
            user_agent: format!("PreBell/0.1 (+https://prebell.ai; {contact})"),
            used: Cell::new(false),
        })
    }

    async fn get(&self, url: &str) -> Result<String, ProviderError> {
        if self.used.replace(true) {
            self.timer
                .sleep(std::time::Duration::from_millis(GAP_MS))
                .await;
        }
        let headers = [
            ("User-Agent", self.user_agent.as_str()),
            ("Accept-Encoding", "gzip, deflate"),
        ];
        let r = self.http.get_capped(url, &headers, MAX_BYTES).await?;
        if r.status != 200 {
            return Err(ProviderError::Http(r.status));
        }
        Ok(r.body)
    }
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

#[derive(Deserialize)]
struct SubRoot {
    cik: serde_json::Value,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tickers: Vec<String>,
    filings: SubFilings,
}
#[derive(Deserialize)]
struct SubFilings {
    recent: Recent,
}
/// Rows of `filings.recent` kept (newest first). The rest of each column is
/// skipped without allocating: a big filer lists 1,000 rows, and 34 such
/// documents per poll must fit the Free plan's CPU budget.
pub const MAX_RECENT_ROWS: usize = 100;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recent {
    #[serde(deserialize_with = "newest")]
    accession_number: Vec<String>,
    #[serde(deserialize_with = "newest")]
    form: Vec<String>,
    #[serde(default, deserialize_with = "newest")]
    items: Vec<String>,
    #[serde(deserialize_with = "newest")]
    filing_date: Vec<String>,
    #[serde(default, deserialize_with = "newest")]
    report_date: Vec<String>,
    #[serde(deserialize_with = "newest")]
    acceptance_date_time: Vec<String>,
    #[serde(default, deserialize_with = "newest")]
    primary_document: Vec<String>,
}

/// The first `MAX_RECENT_ROWS` strings of a JSON array; later ones are
/// skipped unparsed.
fn newest<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an array of strings")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while out.len() < MAX_RECENT_ROWS {
                match seq.next_element::<String>()? {
                    Some(v) => out.push(v),
                    None => return Ok(out),
                }
            }
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(out)
        }
    }
    d.deserialize_seq(V)
}

/// EDGAR's `acceptanceDateTime` is RFC 3339 UTC (`2026-07-30T20:30:28.000Z`
/// is Apple's 4:30 PM EDT release); callers convert to ET for dates.
fn parse_accepted(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

pub fn parse_submissions(body: &str) -> Result<Submissions, ProviderError> {
    let root: SubRoot =
        serde_json::from_str(body).map_err(|e| ProviderError::Parse(e.to_string()))?;
    let cik = match &root.cik {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_u64(),
        _ => None,
    }
    .ok_or_else(|| ProviderError::Parse("submissions: bad cik".into()))?;
    let r = root.filings.recent;
    let col = |v: &Vec<String>, i: usize| v.get(i).cloned().unwrap_or_default();
    let mut recent = Vec::with_capacity(r.accession_number.len());
    for i in 0..r.accession_number.len() {
        // A row without a readable acceptance time cannot be dated; skip it.
        let Some(accepted_at) = r
            .acceptance_date_time
            .get(i)
            .and_then(|s| parse_accepted(s))
        else {
            continue;
        };
        recent.push(RecentFiling {
            accession: col(&r.accession_number, i),
            form: col(&r.form, i),
            items: col(&r.items, i),
            filing_date: col(&r.filing_date, i),
            report_date: col(&r.report_date, i),
            accepted_at,
            primary_doc: col(&r.primary_document, i),
        });
    }
    Ok(Submissions {
        cik,
        name: root.name,
        tickers: root.tickers,
        recent,
    })
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
            name: html_to_text(&a[2]),
            doc_type: html_to_text(cells[3]),
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
/// decoded; each line's whitespace collapsed; blank lines removed. Signs and
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
    async fn submissions(&self, cik: u64) -> Result<Submissions, ProviderError> {
        let url = format!("https://data.sec.gov/submissions/CIK{}.json", cik10(cik));
        parse_submissions(&self.get(&url).await?)
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
        let text = html_to_text(&self.get(url).await?);
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

    pub(crate) const SUBMISSIONS: &str =
        include_str!("../tests/fixtures/edgar_submissions_AAPL.json");
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

    const SUB_URL: &str = "https://data.sec.gov/submissions/CIK0000320193.json";
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
        Edgar::new(http, RecTimer::default(), "ops@prebell.ai").unwrap()
    }

    #[tokio::test]
    async fn submissions_zip_columns() {
        let e = edgar(&[(SUB_URL, 200, SUBMISSIONS)]);
        let s = e.submissions(320193).await.unwrap();
        assert_eq!(s.cik, 320193);
        assert_eq!(s.tickers, vec!["AAPL"]);
        assert_eq!(s.recent.len(), 7);
        let f = &s.recent[1];
        assert_eq!(f.accession, "0000320193-26-000070");
        assert_eq!(f.form, "8-K");
        assert_eq!(f.items, "2.02,9.01");
        assert_eq!(f.primary_doc, "aapl-20261030.htm");
        // EDGAR's acceptanceDateTime is UTC: 20:30:41Z is 16:30:41 EDT.
        assert_eq!(
            f.accepted_at,
            "2026-10-30T20:30:41Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn acceptance_time_is_utc() {
        let et_time = |s: &str| {
            crate::clock::et(parse_accepted(s).unwrap())
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        // Apple's 4:30 PM ET releases, as EDGAR lists them (EDT, then EST).
        assert_eq!(et_time("2026-07-30T20:30:28.000Z"), "2026-07-30 16:30:28");
        assert_eq!(et_time("2026-01-29T21:30:33.000Z"), "2026-01-29 16:30:33");
        // After 8 PM ET the UTC date is the next day; the ET date is not.
        assert_eq!(et_time("2026-10-31T00:45:00.000Z"), "2026-10-30 20:45:00");
        assert_eq!(
            parse_accepted("2026-10-31T00:45:00.000Z"),
            Some("2026-10-31T00:45:00Z".parse().unwrap())
        );
        assert_eq!(parse_accepted("2026-10-31 00:45"), None);
    }

    #[test]
    fn submissions_keep_only_the_newest_rows() {
        // Big filers list up to 1,000 recent rows; only the newest matter
        // (the detection window is 96 h), and the rest are skipped unparsed
        // to stay inside the Free plan's CPU budget.
        let n = MAX_RECENT_ROWS + 50;
        let col = |f: &dyn Fn(usize) -> String| {
            let v: Vec<String> = (0..n).map(f).collect();
            serde_json::to_string(&v).unwrap()
        };
        let body = format!(
            r#"{{"cik":"1","name":"X","tickers":["X"],"filings":{{"recent":{{
            "accessionNumber":{},"form":{},"items":{},"filingDate":{},"reportDate":{},
            "acceptanceDateTime":{},"primaryDocument":{}}}}}}}"#,
            col(&|i| format!("0000000001-26-{i:06}")),
            col(&|_| "4".into()),
            col(&|_| String::new()),
            col(&|_| "2026-10-30".into()),
            col(&|_| "2026-10-30".into()),
            col(&|_| "2026-10-30T20:30:41.000Z".into()),
            col(&|_| "x.htm".into()),
        );
        let s = parse_submissions(&body).unwrap();
        assert_eq!(s.recent.len(), MAX_RECENT_ROWS);
        assert_eq!(s.recent[0].accession, "0000000001-26-000000");
    }

    #[tokio::test]
    async fn user_agent_and_pacing() {
        let e = edgar(&[
            (SUB_URL, 200, SUBMISSIONS),
            (IDX_URL, 200, INDEX),
            (EX_URL, 200, EX991),
        ]);
        e.submissions(320193).await.unwrap();
        e.filing_docs(320193, "0000320193-26-000070").await.unwrap();
        e.document_text(EX_URL).await.unwrap();
        let seen = e.http.seen.borrow();
        assert_eq!(seen.len(), 3);
        for (_, h) in seen.iter() {
            let ua = h.iter().find(|(k, _)| k == "User-Agent").unwrap();
            assert_eq!(ua.1, "PreBell/0.1 (+https://prebell.ai; ops@prebell.ai)");
        }
        // No pause before the first request, one before each later one.
        assert_eq!(*e.timer.0.borrow(), vec![GAP_MS, GAP_MS]);
    }

    #[tokio::test]
    async fn oversized_bodies_are_not_read() {
        let big = "x".repeat(MAX_BYTES + 1);
        let e = edgar(&[(SUB_URL, 200, &big)]);
        assert_eq!(
            e.submissions(320193).await,
            Err(ProviderError::TooLarge(MAX_BYTES))
        );
    }

    #[tokio::test]
    async fn http_errors() {
        let e = edgar(&[(SUB_URL, 403, "Request Rate Threshold Exceeded")]);
        assert_eq!(e.submissions(320193).await, Err(ProviderError::Http(403)));
        let e = edgar(&[(SUB_URL, 429, "")]);
        assert_eq!(e.submissions(320193).await, Err(ProviderError::Http(429)));
        let e = edgar(&[(SUB_URL, 200, "<html>maintenance</html>")]);
        assert!(matches!(
            e.submissions(320193).await,
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
