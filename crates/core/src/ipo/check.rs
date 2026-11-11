//! The hourly filing check (`ipo:check`, queued by an earnings poll): one
//! EDGAR submissions JSON per tracked IPO, the new forms that matter into
//! the timeline, and the cover of the newest registration statement or
//! final prospectus read once (`cover`), then the status and date line
//! recomputed (`resolve`).
//!
//! Free plan per run: at most `MAX_TRACKED` submissions requests plus
//! `MAX_COVER_READS` cover reads (13), paced like the poll (it shares the
//! poll's serial queue, so the two never read EDGAR at once); D1: one
//! state read, one batch write and the kv marker.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::cover::{extract, COVER_PREFIX_BYTES};
use super::resolve::changed_rows;
use super::store::{FactRow, FilingRow, IpoState, IpoStore, IpoWrite};
use super::{is_final, is_kept, is_registration, iso, Tracked, KV_CHECKED, MAX_TRACKED};
use crate::earnings::cik10;
use crate::earnings::store::EarningsStore;
use crate::edgar::{filing_url, Edgar, Filings};
use crate::jobs::alert_once_today;
use crate::providers::{Http, ProviderError, Timer};
use crate::store::Store;

/// Covers read per run at most.
pub const MAX_COVER_READS: usize = 3;
/// A cover that fails this many reads is given up (and alerted).
pub const MAX_READ_ATTEMPTS: i64 = 3;

/// One filing in a submissions JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    pub accession: String,
    pub form: String,
    pub filing_date: String,
    pub primary_doc: String,
}

#[derive(Deserialize)]
struct Root {
    filings: SubFilings,
}
#[derive(Deserialize)]
struct SubFilings {
    recent: Recent,
}
#[derive(Deserialize)]
struct Recent {
    #[serde(rename = "accessionNumber")]
    accession: Vec<String>,
    form: Vec<String>,
    #[serde(rename = "filingDate")]
    filing_date: Vec<String>,
    #[serde(rename = "primaryDocument")]
    primary: Vec<String>,
}

/// The `filings.recent` table of `https://data.sec.gov/submissions/CIK##########.json`.
/// Columns of unequal length are a parse error, never a guess.
pub fn parse_submissions(json: &str) -> Result<Vec<Submission>, ProviderError> {
    let r: Root = serde_json::from_str(json).map_err(|e| ProviderError::Parse(e.to_string()))?;
    let r = r.filings.recent;
    let n = r.accession.len();
    if r.form.len() != n || r.filing_date.len() != n || r.primary.len() != n {
        return Err(ProviderError::Parse(
            "submissions: columns of unequal length".into(),
        ));
    }
    Ok((0..n)
        .map(|i| Submission {
            accession: r.accession[i].clone(),
            form: r.form[i].clone(),
            filing_date: r.filing_date[i].clone(),
            primary_doc: r.primary[i].clone(),
        })
        .collect())
}

pub fn submissions_url(cik: u64) -> String {
    format!("https://data.sec.gov/submissions/CIK{}.json", cik10(cik))
}

/// The primary document's URL on sec.gov.
pub fn doc_url(cik: u64, accession: &str, doc: &str) -> String {
    format!(
        "https://www.sec.gov/Archives/edgar/data/{cik}/{}/{doc}",
        accession.replace('-', "")
    )
}

/// EDGAR as the check reads it.
#[async_trait(?Send)]
pub trait IpoFilings {
    async fn submissions(&self, cik: u64) -> Result<Vec<Submission>, ProviderError>;
    /// The plain text of the first `COVER_PREFIX_BYTES` of a filed document.
    async fn cover_text(&self, url: &str) -> Result<String, ProviderError>;
}

#[async_trait(?Send)]
impl<H: Http, T: Timer> IpoFilings for Edgar<H, T> {
    async fn submissions(&self, cik: u64) -> Result<Vec<Submission>, ProviderError> {
        parse_submissions(&self.get_json(&submissions_url(cik)).await?)
    }
    async fn cover_text(&self, url: &str) -> Result<String, ProviderError> {
        self.document_head(url, COVER_PREFIX_BYTES).await
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    /// Submissions JSONs read.
    pub companies: usize,
    /// Accessions added to the timeline.
    pub new_filings: Vec<String>,
    /// `accession:kinds` for each cover read.
    pub covers: Vec<String>,
    /// `slug=status date` for rows written.
    pub rows: Vec<String>,
    pub throttled: bool,
    pub errors: Vec<String>,
}

fn order(f: &FilingRow) -> (String, String) {
    (f.filing_date.clone(), f.accession.clone())
}

/// The filings to read: per IPO, its newest registration statement or final
/// prospectus while not yet read (older unread ones are superseded and left
/// to the timeline).
fn to_read(st: &IpoState, list: &[Tracked]) -> (Vec<FilingRow>, Vec<FilingRow>) {
    let (mut read, mut superseded) = (Vec::new(), Vec::new());
    for t in list {
        let mut cands: Vec<&FilingRow> = st
            .filings_of(&t.slug)
            .into_iter()
            .filter(|f| is_registration(&f.form) || is_final(&f.form))
            .collect();
        cands.sort_by_key(|f| std::cmp::Reverse(order(f)));
        let Some((newest, older)) = cands.split_first() else {
            continue;
        };
        if matches!(newest.read_state.as_str(), "none" | "pending") {
            read.push((*newest).clone());
        }
        superseded.extend(
            older
                .iter()
                .filter(|f| f.read_state == "pending")
                .map(|f| FilingRow {
                    read_state: "none".into(),
                    ..(*f).clone()
                }),
        );
    }
    (read, superseded)
}

/// One `ipo:check` message.
pub async fn check_tick(
    now: DateTime<Utc>,
    list: &[Tracked],
    ipo: &dyn IpoStore,
    es: &dyn EarningsStore,
    store: &dyn Store,
    src: &dyn IpoFilings,
) -> Result<CheckOutcome, String> {
    let mut out = CheckOutcome::default();
    let mut st = ipo.ipo_state().await?;
    // Marked after the read (a D1 failure retries) and before EDGAR (a
    // failing EDGAR is not asked again every poll).
    es.kv_set(KV_CHECKED, &iso(now), &iso(now)).await?;
    let mut w = IpoWrite::default();
    for t in list.iter().take(MAX_TRACKED) {
        let subs = match src.submissions(t.cik).await {
            Ok(s) => s,
            Err(e) => {
                out.throttled = matches!(e, ProviderError::Http(403 | 429));
                out.errors.push(format!("{}: submissions: {e}", t.slug));
                if out.throttled {
                    break;
                }
                continue;
            }
        };
        out.companies += 1;
        for s in subs.iter().filter(|s| is_kept(&s.form)) {
            if st.filings.iter().any(|f| f.accession == s.accession) {
                continue;
            }
            let f = FilingRow {
                accession: s.accession.clone(),
                slug: t.slug.clone(),
                form: s.form.clone(),
                filing_date: s.filing_date.clone(),
                index_url: filing_url(t.cik, &s.accession),
                doc_url: (!s.primary_doc.is_empty())
                    .then(|| doc_url(t.cik, &s.accession, &s.primary_doc)),
                read_state: "none".into(),
                attempts: 0,
                first_seen_at: iso(now),
            };
            out.new_filings.push(f.accession.clone());
            st.filings.push(f.clone());
            w.filings.push(f);
        }
    }
    let (reads, superseded) = to_read(&st, list);
    let mut changed: Vec<FilingRow> = superseded;
    let mut facts: Vec<FactRow> = Vec::new();
    // The first `MAX_COVER_READS` with a document are read now; the rest wait.
    let mut now_reads: Vec<FilingRow> = Vec::new();
    for mut f in reads {
        if f.doc_url.is_none() {
            f.read_state = "failed".into();
            out.errors.push(format!(
                "{}: {} has no primary document",
                f.slug, f.accession
            ));
            changed.push(f);
        } else if out.throttled || now_reads.len() >= MAX_COVER_READS {
            if f.read_state == "none" {
                f.read_state = "pending".into();
                changed.push(f);
            }
        } else {
            f.attempts += 1;
            f.read_state = "pending".into();
            now_reads.push(f);
        }
    }
    // The attempts are written before the reads, so a read killed for CPU
    // still counts toward `MAX_READ_ATTEMPTS` instead of retrying forever.
    if !now_reads.is_empty() {
        let mut pre = w.clone();
        for f in &now_reads {
            match pre.filings.iter_mut().find(|x| x.accession == f.accession) {
                Some(x) => *x = f.clone(),
                None => pre.filings.push(f.clone()),
            }
        }
        ipo.ipo_write(&pre).await?;
    }
    for mut f in now_reads {
        if out.throttled {
            // Not tried: SEC refused an earlier request.
            f.attempts -= 1;
            changed.push(f);
            continue;
        }
        let url = f.doc_url.clone().unwrap_or_default();
        match src.cover_text(&url).await {
            Ok(text) => {
                let got = extract(&text, &f.form);
                let kinds: Vec<&str> = got.iter().map(|x| x.kind).collect();
                out.covers
                    .push(format!("{}:{}", f.accession, kinds.join(",")));
                if got.is_empty() {
                    let msg = format!(
                        "IPO watch: no cover facts read from the {} filed {} for {} ({}); the page keeps the earlier facts",
                        f.form, f.filing_date, f.slug, f.accession
                    );
                    alert_once_today(now, &format!("ipo-cover:{}", f.slug), "warn", &msg, store)
                        .await?;
                }
                facts.extend(got.into_iter().map(|x| FactRow {
                    accession: f.accession.clone(),
                    kind: x.kind.into(),
                    slug: f.slug.clone(),
                    value_num: x.num,
                    value_text: x.text,
                    quote: x.quote,
                }));
                f.read_state = "read".into();
            }
            Err(e) => {
                if matches!(e, ProviderError::Http(403 | 429)) {
                    out.throttled = true;
                }
                out.errors
                    .push(format!("{}: cover {}: {e}", f.slug, f.accession));
                if f.attempts >= MAX_READ_ATTEMPTS {
                    f.read_state = "failed".into();
                    let msg = format!(
                        "IPO watch: gave up reading the cover of the {} filed {} for {} after {} tries: {e}",
                        f.form, f.filing_date, f.slug, f.attempts
                    );
                    alert_once_today(now, &format!("ipo-cover:{}", f.slug), "error", &msg, store)
                        .await?;
                } else {
                    f.read_state = "pending".into();
                }
            }
        }
        changed.push(f);
    }
    if out.throttled {
        let msg = format!(
            "SEC EDGAR refused requests (403/429) in the IPO check: {}",
            out.errors.last().cloned().unwrap_or_default()
        );
        alert_once_today(now, "sec-throttled", "error", &msg, store).await?;
    }
    for f in changed {
        match w.filings.iter_mut().find(|x| x.accession == f.accession) {
            Some(x) => *x = f,
            None => w.filings.push(f),
        }
    }
    w.facts = facts;
    st.apply(&w);
    w.watch = changed_rows(now, list, &st);
    out.rows = w
        .watch
        .iter()
        .map(|r| {
            format!(
                "{}={} {}",
                r.slug,
                r.status,
                r.first_trade_date.as_deref().unwrap_or("-")
            )
        })
        .collect();
    if !w.is_empty() {
        ipo.ipo_write(&w).await?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::store::MemEarningsStore;
    use crate::edgar::tests::{FakeHttp, RecTimer};
    use crate::ipo::cover::tests::{B4, S1A};
    use crate::ipo::parse_list;
    use crate::ipo::store::MemIpoStore;
    use crate::store::MemStore;

    const SUBS: &str = include_str!("../../tests/fixtures/edgar_ipo_submissions_anthropic.json");
    const S1A_URL: &str =
        "https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/anthropicpbc-sx1ax2.htm";
    const NOW: &str = "2026-11-11T15:00:00Z";

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn anthropic() -> Vec<Tracked> {
        parse_list("anthropic\t2130357\tAnthropic\t-\n").unwrap()
    }
    fn edgar(routes: &[(&str, u16, &str)]) -> Edgar<FakeHttp, RecTimer> {
        let http = FakeHttp {
            routes: routes
                .iter()
                .map(|(u, s, b)| (u.to_string(), (*s, b.to_string())))
                .collect(),
            ..Default::default()
        };
        Edgar::new(http, RecTimer::default(), "support@prebell.site").unwrap()
    }
    /// Requests sent.
    fn sent(e: &Edgar<FakeHttp, RecTimer>) -> Vec<String> {
        e.http
            .seen
            .borrow()
            .iter()
            .map(|(u, _)| u.clone())
            .collect()
    }

    #[test]
    fn the_anthropic_submissions_parse_in_order() {
        let s = parse_submissions(SUBS).unwrap();
        let forms: Vec<&str> = s.iter().map(|x| x.form.as_str()).collect();
        assert_eq!(
            forms,
            ["S-1/A", "S-1/A", "S-1", "DRS/A", "DRS/A", "DRS/A", "DRS/A", "DRS/A", "DRS"]
        );
        assert_eq!(s[0].accession, "0001628280-26-072484");
        assert_eq!(s[0].filing_date, "2026-11-10");
        assert_eq!(s[0].primary_doc, "anthropicpbc-sx1ax2.htm");
        assert_eq!(s[2].accession, "0001628280-26-067168");
        assert_eq!(s[8].filing_date, "2026-06-01");
        assert_eq!(
            submissions_url(2130357),
            "https://data.sec.gov/submissions/CIK0002130357.json"
        );
        assert_eq!(
            doc_url(2130357, &s[0].accession, &s[0].primary_doc),
            S1A_URL
        );
    }

    #[test]
    fn broken_submissions_are_errors() {
        assert!(parse_submissions("{}").is_err());
        assert!(parse_submissions("<html>").is_err());
        let uneven = r#"{"filings":{"recent":{"accessionNumber":["a","b"],"form":["S-1"],"filingDate":["2026-01-01"],"primaryDocument":["x"]}}}"#;
        assert!(parse_submissions(uneven).is_err());
    }

    #[tokio::test]
    async fn the_anthropic_fixtures_give_range_set_with_the_cover_facts_and_the_timeline() {
        let ipo = MemIpoStore::default();
        let es = MemEarningsStore::default();
        let store = MemStore::default();
        let e = edgar(&[(&submissions_url(2130357), 200, SUBS), (S1A_URL, 200, S1A)]);
        let out = check_tick(utc(NOW), &anthropic(), &ipo, &es, &store, &e)
            .await
            .unwrap();
        assert!(out.errors.is_empty(), "{out:?}");
        assert_eq!(out.companies, 1);
        assert_eq!(out.new_filings.len(), 9);
        // Two requests: the submissions JSON and one cover prefix.
        assert_eq!(sent(&e), [submissions_url(2130357), S1A_URL.to_string()]);
        // Paced like the poll; the SEC User-Agent on every request.
        assert_eq!(*e.timer.0.borrow(), [crate::edgar::GAP_MS]);
        for (_, h) in e.http.seen.borrow().iter() {
            assert!(h
                .iter()
                .any(|(k, v)| k == "User-Agent" && v.contains("support@prebell.site")));
        }
        let st = ipo.state.borrow();
        let r = st.row("anthropic").unwrap();
        assert_eq!(r.status, "range_set");
        assert_eq!(r.exchange.as_deref(), Some("Nasdaq Global Select Market"));
        assert_eq!(r.symbol.as_deref(), Some("ANT"));
        assert_eq!(r.facts_accession.as_deref(), Some("0001628280-26-072484"));
        // No filing gives a date; no news yet.
        assert_eq!(r.first_trade_date, None);
        let facts = st.facts_of("0001628280-26-072484");
        let num = |k: &str| facts.iter().find(|f| f.kind == k).and_then(|f| f.value_num);
        assert_eq!(num("shares_offered"), Some(825_000_000.0));
        assert_eq!(num("shares_company"), Some(801_351_354.0));
        assert_eq!(num("shares_selling"), Some(23_648_646.0));
        assert_eq!(
            (num("price_low"), num("price_high")),
            (Some(70.0), Some(78.0))
        );
        // The timeline: every form, only the newest S-1/A read.
        let read: Vec<&str> = st
            .filings
            .iter()
            .filter(|f| f.read_state == "read")
            .map(|f| f.accession.as_str())
            .collect();
        assert_eq!(read, ["0001628280-26-072484"]);
        assert!(st
            .filings
            .iter()
            .all(|f| f.index_url.ends_with("-index.htm")));
        // One state read, the attempt and the result: two writes.
        assert_eq!(ipo.calls.get(), 3);
        drop(st);

        // An hour later nothing is new: one request, nothing written.
        let e2 = edgar(&[(&submissions_url(2130357), 200, SUBS)]);
        let out = check_tick(
            utc("2026-11-11T16:00:00Z"),
            &anthropic(),
            &ipo,
            &es,
            &store,
            &e2,
        )
        .await
        .unwrap();
        assert!(
            out.new_filings.is_empty() && out.covers.is_empty() && out.rows.is_empty(),
            "{out:?}"
        );
        assert_eq!(sent(&e2).len(), 1);
        assert_eq!(ipo.calls.get(), 4);
    }

    #[tokio::test]
    async fn a_424b4_prices_it_and_dates_the_first_trading_day() {
        let ipo = MemIpoStore::default();
        let es = MemEarningsStore::default();
        let store = MemStore::default();
        let subs = SUBS
            .replacen(
                r#""accessionNumber":["#,
                r#""accessionNumber":["0001628280-26-074000","#,
                1,
            )
            .replacen(r#""form":["#, r#""form":["424B4","#, 1)
            .replacen(r#""filingDate":["#, r#""filingDate":["2026-11-13","#, 1)
            .replacen(
                r#""primaryDocument":["#,
                r#""primaryDocument":["b4.htm","#,
                1,
            );
        // The other columns are longer now: the parser checks only these four.
        let b4_url = doc_url(2130357, "0001628280-26-074000", "b4.htm");
        let e = edgar(&[(&submissions_url(2130357), 200, &subs), (&b4_url, 200, B4)]);
        let out = check_tick(
            utc("2026-11-13T15:00:00Z"),
            &anthropic(),
            &ipo,
            &es,
            &store,
            &e,
        )
        .await
        .unwrap();
        assert!(out.errors.is_empty(), "{out:?}");
        let st = ipo.state.borrow();
        let r = st.row("anthropic").unwrap();
        assert_eq!(r.status, "priced");
        // The made-up cover states its trading day: confirmed by the filing.
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-05"));
        assert_eq!(r.date_status.as_deref(), Some("confirmed"));
        assert_eq!(r.date_source_name.as_deref(), Some("424B4"));
    }

    #[tokio::test]
    async fn a_failing_cover_retries_then_gives_up_with_an_alert() {
        let ipo = MemIpoStore::default();
        let es = MemEarningsStore::default();
        let store = MemStore::default();
        let routes = [
            (submissions_url(2130357), 200, SUBS.to_string()),
            (S1A_URL.to_string(), 500, String::new()),
        ];
        let routes: Vec<(&str, u16, &str)> = routes
            .iter()
            .map(|(u, s, b)| (u.as_str(), *s, b.as_str()))
            .collect();
        for (i, at) in [
            "2026-11-11T15:00:00Z",
            "2026-11-11T16:00:00Z",
            "2026-11-11T17:00:00Z",
        ]
        .iter()
        .enumerate()
        {
            let out = check_tick(utc(at), &anthropic(), &ipo, &es, &store, &edgar(&routes))
                .await
                .unwrap();
            assert_eq!(out.errors.len(), 1, "{out:?}");
            let st = ipo.state.borrow();
            let f = st
                .filings
                .iter()
                .find(|f| f.accession == "0001628280-26-072484")
                .unwrap();
            let want = if i < 2 { "pending" } else { "failed" };
            assert_eq!((f.read_state.as_str(), f.attempts), (want, i as i64 + 1));
            // The row still exists: filed, with the timeline.
            assert_eq!(st.row("anthropic").unwrap().status, "filed");
        }
        let alerts = store.alerts.borrow();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0].0, "ipo-cover:anthropic:2026-11-11");
        assert_eq!(alerts[0].1, "error");
        // A fourth run does not read it again.
        let e = edgar(&routes);
        check_tick(
            utc("2026-11-11T18:00:00Z"),
            &anthropic(),
            &ipo,
            &es,
            &store,
            &e,
        )
        .await
        .unwrap();
        assert_eq!(sent(&e).len(), 1);
    }

    /// Reads the stored attempts while the cover is being read.
    struct Probe<'a> {
        ipo: &'a MemIpoStore,
        seen: std::cell::Cell<i64>,
    }
    #[async_trait(?Send)]
    impl IpoFilings for Probe<'_> {
        async fn submissions(&self, _: u64) -> Result<Vec<Submission>, ProviderError> {
            parse_submissions(SUBS)
        }
        async fn cover_text(&self, _: &str) -> Result<String, ProviderError> {
            let st = self.ipo.state.borrow();
            let f = st
                .filings
                .iter()
                .find(|f| f.accession == "0001628280-26-072484");
            self.seen.set(f.map_or(-1, |f| f.attempts));
            Err(ProviderError::Network("killed".into()))
        }
    }

    #[tokio::test]
    async fn the_attempt_is_stored_before_the_cover_is_read() {
        let ipo = MemIpoStore::default();
        let probe = Probe {
            ipo: &ipo,
            seen: std::cell::Cell::new(0),
        };
        let (es, store) = (MemEarningsStore::default(), MemStore::default());
        check_tick(utc(NOW), &anthropic(), &ipo, &es, &store, &probe)
            .await
            .unwrap();
        // A read killed for CPU at this point still counts.
        assert_eq!(probe.seen.get(), 1);
    }

    #[tokio::test]
    async fn sec_throttling_stops_the_run_and_alerts() {
        let ipo = MemIpoStore::default();
        let es = MemEarningsStore::default();
        let store = MemStore::default();
        let list = parse_list("a\t1\tA\t-\nb\t2\tB\t-\n").unwrap();
        let e = edgar(&[(&submissions_url(1), 429, "")]);
        let out = check_tick(utc(NOW), &list, &ipo, &es, &store, &e)
            .await
            .unwrap();
        assert!(out.throttled);
        // The second company is not asked.
        assert_eq!(sent(&e).len(), 1);
        assert_eq!(store.alerts.borrow()[0].0, "sec-throttled:2026-11-11");
        // Marked anyway, so polls do not queue it again within the hour.
        assert_eq!(
            es.kv.borrow().get(KV_CHECKED).map(String::as_str),
            Some(NOW)
        );
    }

    #[tokio::test]
    async fn a_full_list_stays_under_the_request_budget() {
        // Ten companies, each with a new S-1 whose cover is read: ten
        // submissions and at most three covers.
        let ipo = MemIpoStore::default();
        let es = MemEarningsStore::default();
        let store = MemStore::default();
        let list: Vec<Tracked> = (1..=10)
            .map(|i| Tracked {
                slug: format!("c{i}"),
                cik: i,
                name: format!("C{i}"),
                name_zh: None,
            })
            .collect();
        let mut routes: Vec<(String, u16, String)> = Vec::new();
        for i in 1..=10u64 {
            let acc = format!("0000000000-26-00000{}", i % 10);
            let json = format!(
                r#"{{"filings":{{"recent":{{"accessionNumber":["{acc}"],"form":["S-1"],"filingDate":["2026-11-10"],"primaryDocument":["d.htm"]}}}}}}"#
            );
            routes.push((submissions_url(i), 200, json));
            routes.push((doc_url(i, &acc, "d.htm"), 200, S1A.to_string()));
        }
        let routes: Vec<(&str, u16, &str)> = routes
            .iter()
            .map(|(u, s, b)| (u.as_str(), *s, b.as_str()))
            .collect();
        let e = edgar(&routes);
        let out = check_tick(utc(NOW), &list, &ipo, &es, &store, &e)
            .await
            .unwrap();
        assert_eq!(out.covers.len(), MAX_COVER_READS);
        assert_eq!(sent(&e).len(), 13);
        assert!(sent(&e).len() <= MAX_TRACKED + MAX_COVER_READS);
        // The rest wait as pending for the next hour.
        let pending = ipo
            .state
            .borrow()
            .filings
            .iter()
            .filter(|f| f.read_state == "pending")
            .count();
        assert_eq!(pending, 7);
    }
}
