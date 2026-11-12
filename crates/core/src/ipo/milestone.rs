//! IPO milestones, proven by stored data (design
//! `docs/superpowers/specs/2026-11-11-ipo-articles-design.md`, contract
//! `2026-11-11-ipo-articles-contract.md`), and the facts bundle a write-up
//! is drafted from. News never proves a milestone; it is only quoted.
//!
//! Only the latest milestone an IPO has reached is a candidate: one it has
//! moved past without a write-up is never written later, so at launch
//! Anthropic gets its `range_set` (the S-1/A of 2026-11-10) and nothing
//! older.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::resolve::{considered, current, first_session, status};
use super::store::{FactRow, FilingRow, IpoState, ItemRow, TradingRow};
use super::{is_final, is_registration, iso, ymd, Tracked};

/// The facts object's `schema`.
pub const SCHEMA: &str = "ipo-article/v1";
/// News items quoted in one write-up at most, newest first.
pub const MAX_ITEMS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Milestone {
    Filed,
    RangeSet,
    RangeChanged,
    Priced,
    Listed,
    Withdrawn,
}

impl Milestone {
    pub fn as_str(self) -> &'static str {
        match self {
            Milestone::Filed => "filed",
            Milestone::RangeSet => "range_set",
            Milestone::RangeChanged => "range_changed",
            Milestone::Priced => "priced",
            Milestone::Listed => "listed",
            Milestone::Withdrawn => "withdrawn",
        }
    }
    /// The words of the post slug (contract, "Slug words").
    pub fn slug_words(self) -> &'static str {
        match self {
            Milestone::Filed => "filing",
            Milestone::RangeSet => "price-range",
            Milestone::RangeChanged => "new-price-range",
            Milestone::Priced => "priced",
            Milestone::Listed => "first-trading-day",
            Milestone::Withdrawn => "withdrawn",
        }
    }
}

/// A milestone an IPO has reached, with what proves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reached {
    pub milestone: Milestone,
    /// The proving filing's accession, or the session date for `listed`.
    pub key: String,
    /// The filing date, or the session date for `listed`.
    pub as_of: NaiveDate,
    /// The filing whose cover facts the write-up states: the proving filing,
    /// or for `listed` the IPO's facts source at the time.
    pub filing: Option<FilingRow>,
    /// `range_changed`: the previous range milestone's filing.
    pub previous: Option<FilingRow>,
}

impl Reached {
    /// `ipo:<ipo_slug>:<milestone>:<key>`.
    pub fn run_key(&self, slug: &str) -> String {
        format!("ipo:{slug}:{}:{}", self.milestone.as_str(), self.key)
    }
}

fn order(f: &FilingRow) -> (&str, &str) {
    (f.filing_date.as_str(), f.accession.as_str())
}

fn num(st: &IpoState, accession: &str, kind: &str) -> Option<f64> {
    st.facts
        .iter()
        .find(|f| f.accession == accession && f.kind == kind)
        .and_then(|f| f.value_num)
}

fn date(s: &str) -> Option<NaiveDate> {
    s.parse().ok()
}

fn proven(m: Milestone, f: &FilingRow, previous: Option<&FilingRow>) -> Option<Reached> {
    Some(Reached {
        milestone: m,
        key: f.accession.clone(),
        as_of: date(&f.filing_date)?,
        filing: Some(f.clone()),
        previous: previous.cloned(),
    })
}

/// Every milestone `slug` has reached, in stage order (contract,
/// "Milestones"). Only filings that are part of the IPO count, and a cover
/// counts only once it was read.
pub fn reached(st: &IpoState, slug: &str) -> Vec<Reached> {
    let all = st.filings_of(slug);
    let first = first_session(st, slug);
    let mut fs = considered(&all, first);
    fs.sort_by(|a, b| order(a).cmp(&order(b)));
    let mut out = Vec::new();

    // The first public registration statement; a DRS never counts.
    let filed = fs
        .iter()
        .find(|f| matches!(f.form.as_str(), "S-1" | "F-1"))
        .or_else(|| fs.iter().find(|f| is_registration(&f.form)));
    if let Some(f) = filed {
        out.extend(proven(Milestone::Filed, f, None));
    }

    // Read covers with a range: the first sets it; a later one is a
    // milestone only when its range or share count differs from the last
    // range milestone's (an amendment that changes nothing is none). A cover
    // that does not state the shares never counts as a share change.
    let mut last: Option<(&FilingRow, f64, f64, Option<f64>)> = None;
    for f in fs
        .iter()
        .filter(|f| f.read_state == "read" && is_registration(&f.form))
    {
        let (Some(lo), Some(hi)) = (
            num(st, &f.accession, "price_low"),
            num(st, &f.accession, "price_high"),
        ) else {
            continue;
        };
        let shares = num(st, &f.accession, "shares_offered");
        match last {
            None => out.extend(proven(Milestone::RangeSet, f, None)),
            Some((p, plo, phi, psh)) => {
                let shares_differ = shares.zip(psh).is_some_and(|(a, b)| a != b);
                if lo == plo && hi == phi && !shares_differ {
                    continue;
                }
                out.extend(proven(Milestone::RangeChanged, f, Some(p)));
            }
        }
        last = Some((f, lo, hi, shares.or(last.and_then(|l| l.3))));
    }

    if let Some(f) = fs.iter().find(|f| {
        f.read_state == "read"
            && is_final(&f.form)
            && num(st, &f.accession, "price_final").is_some()
    }) {
        out.extend(proven(Milestone::Priced, f, None));
    }

    if let Some(d) = first {
        out.push(Reached {
            milestone: Milestone::Listed,
            key: ymd(d),
            as_of: d,
            filing: current(st, &fs).cloned(),
            previous: None,
        });
    } else if status(&fs, current(st, &fs), st, false) == Some("withdrawn") {
        if let Some(rw) = fs.iter().rev().find(|f| f.form == "RW") {
            out.extend(proven(Milestone::Withdrawn, rw, None));
        }
    }
    out
}

/// The milestone a write-up is due for: the latest one reached.
pub fn latest(st: &IpoState, slug: &str) -> Option<Reached> {
    reached(st, slug)
        .into_iter()
        .max_by(|a, b| (a.milestone, a.as_of).cmp(&(b.milestone, b.as_of)))
}

/// `ipo_filings` fields the bundle carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilingRef {
    pub accession: String,
    pub form: String,
    pub filing_date: String,
    pub index_url: String,
    pub doc_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverFact {
    pub kind: String,
    pub value_num: Option<f64>,
    pub value_text: Option<String>,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Previous {
    pub accession: String,
    pub form: String,
    pub filing_date: String,
    pub index_url: String,
    pub price_low: Option<f64>,
    pub price_high: Option<f64>,
    pub shares_offered: Option<f64>,
}

/// A figure computed by code: shares offered × a price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Derived {
    /// `offering_at_low`, `offering_at_high` or `offering_at_price`.
    pub kind: String,
    pub value_usd: f64,
    pub shares: f64,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub session_date: String,
    pub symbol: String,
    pub open: Option<f64>,
    pub close: f64,
    pub volume: Option<f64>,
    pub ipo_price: Option<f64>,
    pub change_pct: Option<f64>,
    pub source_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub accession: String,
    pub form: String,
    pub filing_date: String,
    pub index_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotedItem {
    pub kind: String,
    pub outlet: String,
    pub url: String,
    pub published_on: String,
    pub quote: String,
    pub speaker: Option<String>,
    pub value_usd: Option<f64>,
    pub value_date: Option<String>,
}

/// The facts object stored in R2 (contract, "The facts object").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpoArticleFacts {
    pub schema: String,
    pub ipo_slug: String,
    pub milestone: Milestone,
    pub key: String,
    pub run_key: String,
    pub name: String,
    pub name_zh: Option<String>,
    pub cik: u64,
    pub symbol: Option<String>,
    pub exchange: Option<String>,
    pub as_of: String,
    pub filing: Option<FilingRef>,
    pub cover: Vec<CoverFact>,
    pub previous: Option<Previous>,
    pub derived: Vec<Derived>,
    pub session: Option<Session>,
    pub timeline: Vec<TimelineEntry>,
    pub items: Vec<QuotedItem>,
    pub generated_at: String,
}

impl IpoArticleFacts {
    /// A cover fact's number.
    pub fn num(&self, kind: &str) -> Option<f64> {
        self.cover
            .iter()
            .find(|c| c.kind == kind)
            .and_then(|c| c.value_num)
    }
    /// A cover fact's text.
    pub fn text(&self, kind: &str) -> Option<&str> {
        self.cover
            .iter()
            .find(|c| c.kind == kind)
            .and_then(|c| c.value_text.as_deref())
    }
    /// `ipo/<ipo_slug>/<milestone>-<key>.json`.
    pub fn facts_key(&self) -> String {
        format!(
            "ipo/{}/{}-{}.json",
            self.ipo_slug,
            self.milestone.as_str(),
            self.key
        )
    }
    pub fn derived(&self, kind: &str) -> Option<&Derived> {
        self.derived.iter().find(|d| d.kind == kind)
    }
}

fn filing_ref(f: &FilingRow) -> FilingRef {
    FilingRef {
        accession: f.accession.clone(),
        form: f.form.clone(),
        filing_date: f.filing_date.clone(),
        index_url: f.index_url.clone(),
        doc_url: f.doc_url.clone(),
    }
}

fn cover_of(st: &IpoState, accession: &str) -> Vec<CoverFact> {
    st.facts_of(accession)
        .into_iter()
        .map(|f: &FactRow| CoverFact {
            kind: f.kind.clone(),
            value_num: f.value_num,
            value_text: f.value_text.clone(),
            quote: f.quote.clone(),
        })
        .collect()
}

fn session_of(t: &TradingRow) -> Session {
    Session {
        session_date: t.session_date.clone(),
        symbol: t.symbol.clone(),
        open: t.open,
        close: t.close,
        volume: t.volume.map(|v| v as f64),
        ipo_price: t.ipo_price,
        change_pct: t.change_pct,
        source_url: t.source_url.clone(),
    }
}

fn quoted(i: &ItemRow) -> QuotedItem {
    QuotedItem {
        kind: i.kind.clone(),
        outlet: i.outlet.clone(),
        url: i.url.clone(),
        published_on: i.published_on.clone(),
        quote: i.quote.clone(),
        speaker: i.speaker.clone(),
        value_usd: i.value_usd,
        value_date: i.value_date.clone(),
    }
}

/// The bundle for `r`, gathered by code from the stored state.
pub fn bundle(st: &IpoState, t: &Tracked, r: &Reached, now: DateTime<Utc>) -> IpoArticleFacts {
    let cover = r
        .filing
        .as_ref()
        .map(|f| cover_of(st, &f.accession))
        .unwrap_or_default();
    let fact_text = |k: &str| {
        cover
            .iter()
            .find(|c| c.kind == k)
            .and_then(|c| c.value_text.clone())
    };
    let fact_num = |k: &str| cover.iter().find(|c| c.kind == k).and_then(|c| c.value_num);
    let row = st.row(&t.slug);
    let symbol = row
        .and_then(|w| w.symbol.clone())
        .or_else(|| fact_text("symbol"));
    let exchange = row
        .and_then(|w| w.exchange.clone())
        .or_else(|| fact_text("exchange"));

    let mut derived = Vec::new();
    if let Some(shares) = fact_num("shares_offered") {
        let mut add = |kind: &str, price: Option<f64>| {
            if let Some(p) = price {
                derived.push(Derived {
                    kind: kind.into(),
                    value_usd: shares * p,
                    shares,
                    price: p,
                });
            }
        };
        if fact_num("price_final").is_some() {
            add("offering_at_price", fact_num("price_final"));
        } else if r.milestone != Milestone::Withdrawn {
            add("offering_at_low", fact_num("price_low"));
            add("offering_at_high", fact_num("price_high"));
        }
    }

    let previous = r.previous.as_ref().map(|p| Previous {
        accession: p.accession.clone(),
        form: p.form.clone(),
        filing_date: p.filing_date.clone(),
        index_url: p.index_url.clone(),
        price_low: num(st, &p.accession, "price_low"),
        price_high: num(st, &p.accession, "price_high"),
        shares_offered: num(st, &p.accession, "shares_offered"),
    });
    let session = (r.milestone == Milestone::Listed)
        .then(|| {
            st.trading_of(&t.slug)
                .into_iter()
                .find(|x| x.session_date == r.key)
                .map(session_of)
        })
        .flatten();

    let all = st.filings_of(&t.slug);
    let mut timeline: Vec<&FilingRow> = considered(&all, first_session(st, &t.slug));
    timeline.sort_by(|a, b| order(b).cmp(&order(a)));
    let mut items: Vec<&ItemRow> = st.items_of(&t.slug);
    items.sort_by(|a, b| (&b.published_on, &b.found_at).cmp(&(&a.published_on, &a.found_at)));

    IpoArticleFacts {
        schema: SCHEMA.into(),
        ipo_slug: t.slug.clone(),
        milestone: r.milestone,
        key: r.key.clone(),
        run_key: r.run_key(&t.slug),
        name: t.name.clone(),
        name_zh: t.name_zh.clone(),
        cik: t.cik,
        symbol,
        exchange,
        as_of: ymd(r.as_of),
        filing: r.filing.as_ref().map(filing_ref),
        cover,
        previous,
        derived,
        session,
        timeline: timeline
            .into_iter()
            .map(|f| TimelineEntry {
                accession: f.accession.clone(),
                form: f.form.clone(),
                filing_date: f.filing_date.clone(),
                index_url: f.index_url.clone(),
            })
            .collect(),
        items: items.into_iter().take(MAX_ITEMS).map(quoted).collect(),
        generated_at: iso(now),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ipo::parse_list;
    use crate::ipo::store::WatchRow;

    pub(crate) const ACC: &str = "0001628280-26-072484";
    const IDX: &str = "https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm";
    const RANGE_QUOTE: &str = "It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00.";
    const SHARES_QUOTE: &str = "We are selling 801,351,354 shares of Class A common stock and the selling stockholders identified in this prospectus are selling an additional 23,648,646 shares of Class A common stock.";
    const LISTING_QUOTE: &str = "We have applied to list our Class A common stock on the Nasdaq Global Select Market under the symbol “ANT.”";

    pub(crate) fn anthropic() -> Tracked {
        parse_list("anthropic\t2130357\tAnthropic\t-\t-\n").unwrap()[0].clone()
    }

    fn filing(acc: &str, form: &str, date: &str, read: &str) -> FilingRow {
        let compact = acc.replace('-', "");
        FilingRow {
            accession: acc.into(),
            slug: "anthropic".into(),
            form: form.into(),
            filing_date: date.into(),
            index_url: format!(
                "https://www.sec.gov/Archives/edgar/data/2130357/{compact}/{acc}-index.htm"
            ),
            doc_url: None,
            read_state: read.into(),
            attempts: i64::from(read == "read"),
            first_seen_at: "2026-11-11T15:00:00Z".into(),
        }
    }

    fn fact(acc: &str, kind: &str, num: Option<f64>, text: Option<&str>, quote: &str) -> FactRow {
        FactRow {
            accession: acc.into(),
            kind: kind.into(),
            slug: "anthropic".into(),
            value_num: num,
            value_text: text.map(Into::into),
            quote: quote.into(),
        }
    }

    /// Production D1 as of 2026-11-11 (db/seed-ipo-preview.sql's real rows):
    /// nine filings, the S-1/A of 2026-11-10 read with nine cover facts, the
    /// symbol ANT, and no news items.
    pub(crate) fn real_state() -> IpoState {
        let mut f = filing(ACC, "S-1/A", "2026-11-10", "read");
        f.index_url = IDX.into();
        f.doc_url = Some("https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/anthropicpbc-sx1ax2.htm".into());
        IpoState {
            watch: vec![WatchRow {
                slug: "anthropic".into(),
                cik: 2130357,
                name: "Anthropic".into(),
                status: "range_set".into(),
                exchange: Some("Nasdaq Global Select Market".into()),
                symbol: Some("ANT".into()),
                facts_accession: Some(ACC.into()),
                checked_at: "2026-11-12T02:05:20Z".into(),
                updated_at: "2026-11-12T02:05:20Z".into(),
                ..Default::default()
            }],
            filings: vec![
                f,
                filing("0001628280-26-068676", "S-1/A", "2026-11-02", "none"),
                filing("0001628280-26-067168", "S-1", "2026-10-26", "none"),
                filing("0002130357-26-000014", "DRS/A", "2026-10-09", "none"),
                filing("0002130357-26-000009", "DRS/A", "2026-10-01", "none"),
                filing("0001628279-26-001111", "DRS/A", "2026-09-04", "none"),
                filing("0001628279-26-000993", "DRS/A", "2026-08-10", "none"),
                filing("0001628279-26-000861", "DRS/A", "2026-07-13", "none"),
                filing("0002130357-26-000005", "DRS", "2026-06-01", "none"),
            ],
            facts: vec![
                fact(ACC, "cover_date", None, Some("2026-11-10"), "Subject to Completion. Dated November 10, 2026."),
                fact(ACC, "shares_offered", Some(825000000.0), None, "Class A Common Stock 825,000,000 Shares"),
                fact(ACC, "shares_company", Some(801351354.0), None, SHARES_QUOTE),
                fact(ACC, "shares_selling", Some(23648646.0), None, SHARES_QUOTE),
                fact(ACC, "price_low", Some(70.0), None, RANGE_QUOTE),
                fact(ACC, "price_high", Some(78.0), None, RANGE_QUOTE),
                fact(ACC, "exchange", None, Some("Nasdaq Global Select Market"), LISTING_QUOTE),
                fact(ACC, "symbol", None, Some("ANT"), LISTING_QUOTE),
                fact(ACC, "option_shares", Some(123750000.0), None, "To the extent that the underwriters sell more than 825,000,000 shares of Class A common stock, the underwriters have the option to purchase up to an additional 123,750,000 shares of Class A common stock from Anthropic, PBC at the initial public offering price less the underwriting discount."),
            ],
            items: vec![],
            trading: vec![],
        }
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn the_real_state_is_at_range_set_from_the_s1a_of_nov_10() {
        let st = real_state();
        let all = reached(&st, "anthropic");
        let names: Vec<_> = all.iter().map(|r| r.milestone).collect();
        assert_eq!(names, [Milestone::Filed, Milestone::RangeSet]);
        // The S-1 of Oct 26 is the filing milestone; a DRS never is.
        assert_eq!(all[0].key, "0001628280-26-067168");
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(
            (r.milestone, r.key.as_str(), ymd(r.as_of)),
            (Milestone::RangeSet, ACC, "2026-11-10".into())
        );
        assert_eq!(
            r.run_key("anthropic"),
            "ipo:anthropic:range_set:0001628280-26-072484"
        );
    }

    #[test]
    fn the_bundle_carries_the_cover_derived_figures_and_timeline() {
        let st = real_state();
        let r = latest(&st, "anthropic").unwrap();
        let b = bundle(&st, &anthropic(), &r, utc("2026-11-11T22:00:00Z"));
        assert_eq!(b.cover.len(), 9);
        assert_eq!(
            (b.symbol.as_deref(), b.exchange.as_deref()),
            (Some("ANT"), Some("Nasdaq Global Select Market"))
        );
        let d: Vec<_> = b
            .derived
            .iter()
            .map(|d| (d.kind.as_str(), d.value_usd))
            .collect();
        assert_eq!(
            d,
            [("offering_at_low", 57.75e9), ("offering_at_high", 64.35e9)]
        );
        assert_eq!(b.timeline.len(), 9);
        assert_eq!(b.timeline[0].accession, ACC);
        assert!(b.items.is_empty() && b.previous.is_none() && b.session.is_none());
        assert_eq!(
            b.facts_key(),
            "ipo/anthropic/range_set-0001628280-26-072484.json"
        );
    }

    fn with_cover(st: &mut IpoState, acc: &str, date: &str, lo: f64, hi: f64, shares: Option<f64>) {
        st.filings.push(filing(acc, "S-1/A", date, "read"));
        st.facts.push(fact(acc, "price_low", Some(lo), None, "q"));
        st.facts.push(fact(acc, "price_high", Some(hi), None, "q"));
        if let Some(s) = shares {
            st.facts
                .push(fact(acc, "shares_offered", Some(s), None, "q"));
        }
        for k in ["exchange", "symbol"] {
            st.facts.push(fact(acc, k, None, Some("x"), "q"));
        }
    }

    #[test]
    fn an_amendment_is_a_milestone_only_when_the_range_or_shares_change() {
        let mut st = real_state();
        // The same terms: nothing new.
        with_cover(
            &mut st,
            "0001628280-26-080001",
            "2026-11-12",
            70.0,
            78.0,
            Some(825e6),
        );
        assert_eq!(
            latest(&st, "anthropic").unwrap().milestone,
            Milestone::RangeSet
        );
        // No share count on the cover: not a share change.
        with_cover(
            &mut st,
            "0001628280-26-080002",
            "2026-11-13",
            70.0,
            78.0,
            None,
        );
        assert_eq!(
            latest(&st, "anthropic").unwrap().milestone,
            Milestone::RangeSet
        );
        // A new range: changed, against the range_set filing.
        with_cover(
            &mut st,
            "0001628280-26-080003",
            "2026-11-14",
            76.0,
            80.0,
            Some(825e6),
        );
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(
            (
                r.milestone,
                r.key.as_str(),
                r.previous.as_ref().map(|p| p.accession.as_str())
            ),
            (Milestone::RangeChanged, "0001628280-26-080003", Some(ACC))
        );
        let b = bundle(&st, &anthropic(), &r, utc("2026-11-14T22:00:00Z"));
        let p = b.previous.unwrap();
        assert_eq!((p.price_low, p.price_high), (Some(70.0), Some(78.0)));
        // More shares at the same range: changed again, against the last change.
        with_cover(
            &mut st,
            "0001628280-26-080004",
            "2026-11-15",
            76.0,
            80.0,
            Some(900e6),
        );
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(
            (r.milestone, r.previous.unwrap().accession.as_str()),
            (Milestone::RangeChanged, "0001628280-26-080003")
        );
    }

    #[test]
    fn pricing_listing_and_withdrawal_are_proven_by_stored_rows() {
        let mut st = real_state();
        let fin = "0001628280-26-090001";
        st.filings.push(filing(fin, "424B4", "2026-11-18", "read"));
        st.facts
            .push(fact(fin, "price_final", Some(80.0), None, "q"));
        st.facts
            .push(fact(fin, "shares_offered", Some(825e6), None, "q"));
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!((r.milestone, r.key.as_str()), (Milestone::Priced, fin));
        let b = bundle(&st, &anthropic(), &r, utc("2026-11-18T22:00:00Z"));
        assert_eq!(
            b.derived
                .iter()
                .map(|d| (d.kind.as_str(), d.value_usd))
                .collect::<Vec<_>>(),
            [("offering_at_price", 66e9)]
        );
        // A 424B4 not read yet proves nothing.
        let mut unread = real_state();
        unread
            .filings
            .push(filing(fin, "424B4", "2026-11-18", "pending"));
        assert_eq!(
            latest(&unread, "anthropic").unwrap().milestone,
            Milestone::RangeSet
        );

        st.trading.push(TradingRow {
            slug: "anthropic".into(),
            session_date: "2026-11-19".into(),
            symbol: "ANT".into(),
            open: Some(95.0),
            close: 101.5,
            volume: Some(120_000_000),
            ipo_price: Some(80.0),
            change_pct: Some(26.88),
            source_url: "https://finance.yahoo.com/quote/ANT".into(),
            fetched_at: "2026-11-19T21:30:00Z".into(),
        });
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(
            (r.milestone, r.key.as_str()),
            (Milestone::Listed, "2026-11-19")
        );
        let b = bundle(&st, &anthropic(), &r, utc("2026-11-19T22:00:00Z"));
        assert_eq!(b.session.as_ref().map(|s| s.close), Some(101.5));
        assert_eq!(b.filing.as_ref().map(|f| f.form.as_str()), Some("424B4"));
        // A resale withdrawal after listing is not part of the IPO.
        st.filings
            .push(filing("0001628280-26-099999", "RW", "2026-12-20", "none"));
        assert_eq!(
            latest(&st, "anthropic").unwrap().milestone,
            Milestone::Listed
        );

        let mut w = real_state();
        w.filings
            .push(filing("0001628280-26-095555", "RW", "2026-11-20", "none"));
        let r = latest(&w, "anthropic").unwrap();
        assert_eq!(
            (r.milestone, r.key.as_str()),
            (Milestone::Withdrawn, "0001628280-26-095555")
        );
        assert!(bundle(&w, &anthropic(), &r, utc("2026-11-20T22:00:00Z"))
            .derived
            .is_empty());
    }

    #[test]
    fn nothing_is_reached_before_a_public_filing() {
        let mut st = real_state();
        st.filings.retain(|f| f.form.starts_with("DRS"));
        assert!(reached(&st, "anthropic").is_empty());
        assert!(latest(&st, "other").is_none());
    }
}
