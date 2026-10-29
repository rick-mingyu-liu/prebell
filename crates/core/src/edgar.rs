//! SEC EDGAR behind the `Filings` trait: company submissions, filing
//! indexes, filed documents and XBRL `companyfacts`.
//!
//! Fair access (https://www.sec.gov/os/webmaster-faq#developers): every
//! request carries a descriptive User-Agent with a contact address, and
//! requests are sequential with a pause between them so one invocation stays
//! under 10 requests/second.

use std::cell::Cell;
use std::sync::OnceLock;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use regex::Regex;
use serde::Deserialize;

use crate::anchor::decode_entities;
use crate::earnings::cik10;
use crate::providers::{Http, ProviderError};

/// Pause before every EDGAR request after the first: at most 8 requests/second.
pub const GAP_MS: u64 = 125;
const _: () = assert!(
    GAP_MS >= 100,
    "EDGAR fair access: at most 10 requests/second"
);
const ARCHIVES: &str = "https://www.sec.gov/Archives/edgar/data/";

/// An async sleep (Workers `Delay` in production, a recorder in tests).
#[async_trait(?Send)]
pub trait Pause {
    async fn pause(&self, ms: u64);
}

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

/// The slice of `companyfacts` the earnings trend uses. Every other concept
/// in the (multi-megabyte) document is skipped by serde without allocation.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CompanyFacts {
    #[serde(default)]
    pub facts: FactsRoot,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FactsRoot {
    #[serde(rename = "us-gaap", default)]
    pub us_gaap: UsGaap,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct UsGaap {
    #[serde(rename = "Revenues")]
    pub revenues: Option<Concept>,
    #[serde(rename = "RevenueFromContractWithCustomerExcludingAssessedTax")]
    pub revenue_from_contracts: Option<Concept>,
    #[serde(rename = "RevenuesNetOfInterestExpense")]
    pub revenues_net_of_interest: Option<Concept>,
    #[serde(rename = "SalesRevenueNet")]
    pub sales_revenue_net: Option<Concept>,
    #[serde(rename = "EarningsPerShareDiluted")]
    pub eps_diluted: Option<Concept>,
    #[serde(rename = "NetIncomeLoss")]
    pub net_income: Option<Concept>,
}

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
    async fn company_facts(&self, cik: u64) -> Result<CompanyFacts, ProviderError>;
}

pub struct Edgar<H: Http, P: Pause> {
    pub http: H,
    pub pause: P,
    user_agent: String,
    used: Cell<bool>,
}

impl<H: Http, P: Pause> Edgar<H, P> {
    /// `contact` is the `SEC_CONTACT` email; SEC blocks anonymous clients.
    pub fn new(http: H, pause: P, contact: &str) -> Result<Self, ProviderError> {
        let contact = contact.trim();
        if contact.is_empty() {
            return Err(ProviderError::Parse("SEC_CONTACT is empty".into()));
        }
        Ok(Self {
            http,
            pause,
            user_agent: format!("PreBell/0.1 (+https://prebell.ai; {contact})"),
            used: Cell::new(false),
        })
    }

    async fn get(&self, url: &str) -> Result<String, ProviderError> {
        if self.used.replace(true) {
            self.pause.pause(GAP_MS).await;
        }
        let headers = [
            ("User-Agent", self.user_agent.as_str()),
            ("Accept-Encoding", "gzip, deflate"),
        ];
        let r = self.http.get(url, &headers).await?;
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recent {
    accession_number: Vec<String>,
    form: Vec<String>,
    #[serde(default)]
    items: Vec<String>,
    filing_date: Vec<String>,
    #[serde(default)]
    report_date: Vec<String>,
    acceptance_date_time: Vec<String>,
    #[serde(default)]
    primary_document: Vec<String>,
}

/// EDGAR's `acceptanceDateTime` ends in `Z` but holds Eastern wall-clock
/// time (filings are accepted 06:00-22:00 ET; a 06:01 acceptance reads
/// `T06:01:..Z`). It is read as America/New_York.
fn parse_accepted(s: &str) -> Option<DateTime<Utc>> {
    let naive = s.trim_end_matches('Z');
    let t = NaiveDateTime::parse_from_str(naive, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    New_York
        .from_local_datetime(&t)
        .earliest()
        .map(|d| d.with_timezone(&Utc))
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

/// Plain text of an HTML document: scripts, styles and comments dropped,
/// block ends and table cells become line breaks and spaces, entities
/// decoded, each line's whitespace collapsed, blank lines removed.
pub fn html_to_text(html: &str) -> String {
    static DROP: OnceLock<Regex> = OnceLock::new();
    static BLOCK: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    let drop = DROP.get_or_init(|| {
        re(r"(?is)<script\b.*?</script\s*>|<style\b.*?</style\s*>|<!--.*?-->|<head\b.*?</head\s*>")
    });
    let block = BLOCK.get_or_init(|| {
        re(r"(?i)<br\s*/?>|</(?:p|div|tr|li|h[1-6]|table|title)\s*>|<(?:p|div|tr|li|table)\b[^>]*>")
    });
    let cell = CELL.get_or_init(|| re(r"(?i)</t[dh]\s*>"));
    let tag = TAG.get_or_init(|| re(r"(?s)<[^>]*>"));
    let s = drop.replace_all(html, " ");
    let s = block.replace_all(&s, "\n");
    let s = cell.replace_all(&s, " ");
    let s = tag.replace_all(&s, "");
    decode_entities(&s)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait(?Send)]
impl<H: Http, P: Pause> Filings for Edgar<H, P> {
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

    async fn company_facts(&self, cik: u64) -> Result<CompanyFacts, ProviderError> {
        let url = format!(
            "https://data.sec.gov/api/xbrl/companyfacts/CIK{}.json",
            cik10(cik)
        );
        let body = self.get(&url).await?;
        serde_json::from_str(&body).map_err(|e| ProviderError::Parse(e.to_string()))
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
    pub(crate) const COMPANYFACTS: &str =
        include_str!("../tests/fixtures/edgar_companyfacts_AAPL.json");

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
            Ok(HttpResp { status, body })
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

    #[derive(Default)]
    pub(crate) struct RecPause(pub RefCell<Vec<u64>>);
    #[async_trait(?Send)]
    impl Pause for RecPause {
        async fn pause(&self, ms: u64) {
            self.0.borrow_mut().push(ms);
        }
    }

    const SUB_URL: &str = "https://data.sec.gov/submissions/CIK0000320193.json";
    const IDX_URL: &str = "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm";
    const EX_URL: &str = "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/a8-kex991q4202609262026.htm";
    const CF_URL: &str = "https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json";

    fn edgar(routes: &[(&str, u16, &str)]) -> Edgar<FakeHttp, RecPause> {
        let http = FakeHttp {
            routes: routes
                .iter()
                .map(|(u, s, b)| (u.to_string(), (*s, b.to_string())))
                .collect(),
            ..Default::default()
        };
        Edgar::new(http, RecPause::default(), "ops@prebell.ai").unwrap()
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
        // 16:30:41 Eastern (EDT) = 20:30:41 UTC.
        assert_eq!(
            f.accepted_at,
            "2026-10-30T20:30:41Z".parse::<DateTime<Utc>>().unwrap()
        );
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
        assert_eq!(*e.pause.0.borrow(), vec![GAP_MS, GAP_MS]);
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
        assert!(Edgar::new(FakeHttp::default(), RecPause::default(), "  ").is_err());
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
        assert!(t.contains("Total net sales $ 102,466 $ 94,930"), "{t}");
        assert!(t.contains("fiscal 2026 fourth quarter ended September 26, 2026"));
        assert!(t.contains("Company\u{2019}s CEO"));
        assert!(t.contains("$102.5 billion"));
        assert!(!t.contains("999.9"), "script text leaked");
        assert!(!t.contains("margin: 0"), "style text leaked");
        assert!(!t.contains('<'));
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
    async fn company_facts_keeps_needed_concepts() {
        let e = edgar(&[(CF_URL, 200, COMPANYFACTS)]);
        let cf = e.company_facts(320193).await.unwrap();
        let g = cf.facts.us_gaap;
        assert!(g.revenues.is_none());
        assert!(!g.revenue_from_contracts.unwrap().units.usd.is_empty());
        assert!(!g.eps_diluted.unwrap().units.usd_per_share.is_empty());
        assert!(g.net_income.is_some());
    }

    #[test]
    fn filing_url_format() {
        assert_eq!(filing_url(320193, "0000320193-26-000070"), IDX_URL);
    }
}
