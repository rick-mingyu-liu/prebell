//! The cover figure of a published post: two short text lines composed from
//! the facts, the scene prompt for the generated background, and the index
//! snapshot the front page reads. Pure apart from `make_scene`, which stores
//! the scene; the image model sits behind `CoverMaker` and is implemented by
//! the generator.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::earnings::draft::{fmt_value, period_label};
use crate::earnings::facts::{EarningsFacts, Metric, Unit};
use crate::facts::{Field, Mover, Quote};
use crate::numbers::Lang;
use crate::store::Store;

/// The `kv` key the front page reads the latest index snapshot from.
pub const INDEX_LINE_KEY: &str = "index_line";

/// The two lines printed over the cover, per language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverLines {
    pub en: [String; 2],
    pub zh: [String; 2],
}

/// Makes the cover's background scene. One attempt per call; the caller
/// decides whether to retry and falls back to a plain cover on error.
#[async_trait(?Send)]
pub trait CoverMaker {
    /// One attempt. Ok(bytes) is a PNG or JPEG; Err is logged and ignored.
    async fn scene(&self, prompt: &str) -> Result<Vec<u8>, String>;
}

/// The maker used when no image model is configured.
pub struct NoCover;

#[async_trait(?Send)]
impl CoverMaker for NoCover {
    async fn scene(&self, _prompt: &str) -> Result<Vec<u8>, String> {
        Err("cover maker not configured".into())
    }
}

/// The U+2212 minus when `v` is negative and does not round to zero at
/// `decimals` places; empty otherwise.
fn minus_if_negative(v: f64, decimals: usize) -> &'static str {
    if v < 0.0
        && format!("{:.decimals$}", v.abs())
            .bytes()
            .any(|b| matches!(b, b'1'..=b'9'))
    {
        "−"
    } else {
        ""
    }
}

/// `value` rounded to one decimal with a trailing ".0" dropped.
fn one_decimal(value: f64) -> String {
    let s = format!("{value:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

/// "$10.4B", "$850M", "$2.3M", "$1B"; below $1M, whole dollars with
/// thousands separators. Negative values carry U+2212.
pub fn fmt_short_usd(value: f64) -> String {
    let a = value.abs();
    let sign = minus_if_negative(value, 0);
    let rounded = |div: f64| (a / div * 10.0).round() / 10.0;
    if a >= 1e9 || (a >= 1e6 && rounded(1e6) >= 1000.0) {
        format!("{sign}${}B", one_decimal(rounded(1e9)))
    } else if a >= 1e6 {
        format!("{sign}${}M", one_decimal(rounded(1e6)))
    } else {
        let whole = format!("{:.0}", a);
        let mut grouped = String::new();
        for (i, c) in whole.chars().enumerate() {
            if i > 0 && (whole.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        format!("{sign}${grouped}")
    }
}

/// "$7.08", "−$0.50".
pub fn fmt_eps(value: f64) -> String {
    format!("{}${:.2}", minus_if_negative(value, 2), value.abs())
}

/// "7.08美元", "−0.50美元": `fmt_value` signs with an ASCII hyphen.
fn fmt_eps_zh(value: f64) -> String {
    format!("{}{:.2}美元", minus_if_negative(value, 2), value.abs())
}

/// "+3.03%", "−23.67%" (U+2212); a move that rounds to zero is "0.00%".
pub fn fmt_signed_pct(p: f64) -> String {
    let minus = minus_if_negative(p, 2);
    let a = format!("{:.2}", p.abs());
    if !minus.is_empty() {
        format!("−{a}%")
    } else if a.bytes().any(|b| matches!(b, b'1'..=b'9')) {
        format!("+{a}%")
    } else {
        format!("{a}%")
    }
}

/// Earnings cover: revenue, then GAAP EPS, else non-GAAP EPS, else the
/// period label. A missing revenue leaves the company name on line 1.
pub fn earnings_lines(f: &EarningsFacts) -> CoverLines {
    let fig = |m| f.metrics.value(m);
    let line1_en = match fig(Metric::Revenue) {
        Some(r) => format!("Revenue {}", fmt_short_usd(r.value)),
        None => format!("{} results", f.company),
    };
    let line1_zh = match fig(Metric::Revenue) {
        Some(r) => format!("营收 {}", fmt_value(r.value, Unit::Usd, Lang::Zh)),
        None => format!("{}业绩", f.company),
    };
    let (line2_en, line2_zh) = if let Some(e) = fig(Metric::EpsGaap) {
        (
            format!("GAAP EPS {}", fmt_eps(e.value)),
            format!("GAAP每股收益 {}", fmt_eps_zh(e.value)),
        )
    } else if let Some(e) = fig(Metric::EpsNonGaap) {
        (
            format!("Non-GAAP EPS {}", fmt_eps(e.value)),
            format!("非GAAP每股收益 {}", fmt_eps_zh(e.value)),
        )
    } else {
        let en = match &f.fiscal_period {
            Field::Value(p) => match p.quarter {
                Some(q) => format!("Q{q} FY{} results", p.fiscal_year),
                None => format!("FY{} results", p.fiscal_year),
            },
            Field::Missing { .. } => "Earnings results".to_string(),
        };
        let zh = match period_label(f, Lang::Zh) {
            Some(p) => format!("{p}业绩"),
            None => "财报业绩".to_string(),
        };
        (en, zh)
    };
    CoverLines {
        en: [line1_en, line2_en],
        zh: [line1_zh, line2_zh],
    }
}

/// "Dow", "S&P 500", ... for a quote symbol; unknown symbols as they are.
fn index_label(symbol: &str, futures: bool, lang: Lang) -> String {
    let known = match (futures, symbol) {
        (false, "^DJI") => Some(("Dow", "道指")),
        (false, "^GSPC") => Some(("S&P 500", "标普500")),
        (false, "^NDX" | "^IXIC") => Some(("Nasdaq", "纳指")),
        (false, "^RUT") => Some(("Russell 2000", "罗素2000")),
        (true, "ES=F") => Some(("S&P futures", "标普500期货")),
        (true, "NQ=F") => Some(("Nasdaq futures", "纳指期货")),
        (true, "YM=F") => Some(("Dow futures", "道指期货")),
        (true, "RTY=F") => Some(("Russell futures", "罗素2000期货")),
        _ => None,
    };
    match (known, lang) {
        (Some((en, _)), Lang::En) => en.to_string(),
        (Some((_, zh)), Lang::Zh) => zh.to_string(),
        (None, _) => symbol.to_string(),
    }
}

/// Brief cover: the largest index move, then the top gainer and loser.
/// `futures` is true for the premarket brief. Line 1 is empty when no
/// quote is present; line 2 omits an empty side.
pub fn brief_lines(
    indexes: &[(String, Field<Quote>)],
    gainers: &[Mover],
    losers: &[Mover],
    futures: bool,
) -> CoverLines {
    let mut biggest: Option<&Quote> = None;
    for (_, field) in indexes {
        if let Field::Value(q) = field {
            if biggest.is_none_or(|b| q.change_pct.abs() > b.change_pct.abs()) {
                biggest = Some(q);
            }
        }
    }
    let line1 = |lang| match biggest {
        Some(q) => format!(
            "{} {}",
            index_label(&q.symbol, futures, lang),
            fmt_signed_pct(q.change_pct)
        ),
        None => String::new(),
    };
    let gainer = gainers.iter().reduce(|a, b| {
        if b.quote.change_pct > a.quote.change_pct {
            b
        } else {
            a
        }
    });
    let loser = losers.iter().reduce(|a, b| {
        if b.quote.change_pct < a.quote.change_pct {
            b
        } else {
            a
        }
    });
    let side = |m: &Mover| format!("{} {}", m.quote.symbol, fmt_signed_pct(m.quote.change_pct));
    let line2 = [gainer, loser]
        .into_iter()
        .flatten()
        .map(side)
        .collect::<Vec<_>>()
        .join(", ");
    CoverLines {
        en: [line1(Lang::En), line2.clone()],
        zh: [line1(Lang::Zh), line2],
    }
}

/// "png" for a PNG, "jpg" for a JPEG, `None` for anything else.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

/// The image-model prompt for a scene subject: the subject, a fixed style,
/// and cues that keep people, text and charts out of the picture.
pub fn scene_prompt(subject: &str) -> String {
    format!(
        "{subject}, documentary photograph, muted desaturated tones, soft natural light, \
         wide shot, no people, no faces, no text, no logos, no charts"
    )
}

const BRIEF_SCENES: [&str; 5] = [
    "a trading floor seen from above",
    "a stock exchange building exterior at dawn",
    "a financial district skyline at dusk",
    "a newsroom desk with printed pages",
    "a harbour of container ships",
];

/// The premarket and recap scene, rotated by weekday so consecutive days
/// do not share a picture.
pub fn brief_scene_subject(date: NaiveDate) -> String {
    BRIEF_SCENES[date.weekday().num_days_from_monday() as usize % BRIEF_SCENES.len()].to_string()
}

/// The earnings scene: the company's industry, never its logo or people.
pub fn earnings_scene_subject(f: &EarningsFacts) -> String {
    let company: String = f
        .company
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    format!("a wide establishing view of the industry of {company}: its typical facilities, products and equipment")
}

/// The scene step of a build: one `maker` attempt for `subject`, stored as
/// `covers/{slug}.{png|jpg}`. Returns the stored key, or `None`, and the
/// outcome for the diagnostics line: `ok`, `skip` (no maker configured) or
/// `err` (the maker failed, the bytes are not PNG/JPEG, or the write failed).
/// Never fails the build.
pub async fn make_scene(
    store: &dyn Store,
    maker: &dyn CoverMaker,
    slug: &str,
    subject: &str,
) -> (Option<String>, &'static str) {
    let bytes = match maker.scene(&scene_prompt(subject)).await {
        Ok(b) => b,
        Err(e) if e.contains("not configured") => return (None, "skip"),
        Err(_) => return (None, "err"),
    };
    let (ext, mime) = match image_ext(&bytes) {
        Some("png") => ("png", "image/png"),
        Some("jpg") => ("jpg", "image/jpeg"),
        _ => return (None, "err"),
    };
    let key = format!("covers/{slug}.{ext}");
    match store.put_bytes(&key, &bytes, mime).await {
        Ok(()) => (Some(key), "ok"),
        Err(_) => (None, "err"),
    }
}

/// The latest market snapshot behind the front page's index line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexSnapshot {
    /// `premarket` or `recap`.
    pub kind: String,
    /// RFC 3339, UTC, second precision.
    pub as_of_utc: String,
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub symbol: String,
    pub last: f64,
    pub change_pct: f64,
}

/// The snapshot of the present quotes; missing ones are skipped.
pub fn index_snapshot(
    kind: &str,
    as_of: DateTime<Utc>,
    quotes: &[(String, Field<Quote>)],
) -> IndexSnapshot {
    IndexSnapshot {
        kind: kind.to_string(),
        as_of_utc: as_of.to_rfc3339_opts(SecondsFormat::Secs, true),
        entries: quotes
            .iter()
            .filter_map(|(_, f)| match f {
                Field::Value(q) => Some(IndexEntry {
                    symbol: q.symbol.clone(),
                    last: q.last,
                    change_pct: q.change_pct,
                }),
                Field::Missing { .. } => None,
            })
            .collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::earnings::facts::tests::{facts, fig};
    use crate::earnings::facts::FiscalPeriod;
    use crate::store::MemStore;
    use std::cell::RefCell;

    /// The PNG signature: enough for `image_ext`.
    pub(crate) const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    /// Answers every call with `reply` and records each prompt.
    pub(crate) struct FakeCover {
        pub reply: Result<Vec<u8>, String>,
        pub prompts: RefCell<Vec<String>>,
    }
    impl FakeCover {
        pub(crate) fn new(reply: Result<Vec<u8>, String>) -> Self {
            FakeCover {
                reply,
                prompts: RefCell::new(Vec::new()),
            }
        }
    }
    #[async_trait(?Send)]
    impl CoverMaker for FakeCover {
        async fn scene(&self, prompt: &str) -> Result<Vec<u8>, String> {
            self.prompts.borrow_mut().push(prompt.to_string());
            self.reply.clone()
        }
    }

    #[tokio::test]
    async fn make_scene_stores_png_and_jpg_under_the_slug() {
        let s = MemStore::default();
        let png = FakeCover::new(Ok(PNG.to_vec()));
        assert_eq!(
            make_scene(&s, &png, "recap-2026-11-04", "a harbour").await,
            (Some("covers/recap-2026-11-04.png".to_string()), "ok")
        );
        assert_eq!(*png.prompts.borrow(), [scene_prompt("a harbour")]);
        assert_eq!(
            s.bytes.borrow()["covers/recap-2026-11-04.png"],
            (PNG.to_vec(), "image/png".to_string())
        );
        let jpg = FakeCover::new(Ok(vec![0xFF, 0xD8, 0xFF, 0xE0]));
        assert_eq!(
            make_scene(&s, &jpg, "aapl-q4-fy2026-earnings", "x").await,
            (Some("covers/aapl-q4-fy2026-earnings.jpg".to_string()), "ok")
        );
        assert_eq!(
            s.bytes.borrow()["covers/aapl-q4-fy2026-earnings.jpg"].1,
            "image/jpeg"
        );
    }

    #[tokio::test]
    async fn make_scene_skips_without_a_maker_and_errs_on_bad_replies() {
        let s = MemStore::default();
        assert_eq!(make_scene(&s, &NoCover, "p", "x").await, (None, "skip"));
        let boom = FakeCover::new(Err("boom".into()));
        assert_eq!(make_scene(&s, &boom, "p", "x").await, (None, "err"));
        let gif = FakeCover::new(Ok(b"GIF89a".to_vec()));
        assert_eq!(make_scene(&s, &gif, "p", "x").await, (None, "err"));
        assert!(s.bytes.borrow().is_empty());
    }

    fn quote(sym: &str, last: f64, pct: f64) -> Quote {
        Quote {
            symbol: sym.into(),
            last,
            prev_close: last / (1.0 + pct / 100.0),
            change_pct: pct,
            volume: Some(1_000_000),
        }
    }
    fn present(label: &str, sym: &str, last: f64, pct: f64) -> (String, Field<Quote>) {
        (label.into(), Field::Value(quote(sym, last, pct)))
    }
    fn missing(label: &str) -> (String, Field<Quote>) {
        (
            label.into(),
            Field::Missing {
                reason: "HTTP 429".into(),
            },
        )
    }
    fn mover(sym: &str, pct: f64) -> Mover {
        Mover {
            quote: quote(sym, 50.0, pct),
            name: format!("{sym} Inc"),
            reasons: vec![],
        }
    }

    #[test]
    fn short_usd_formats_billions_and_millions() {
        assert_eq!(fmt_short_usd(10_400_000_000.0), "$10.4B");
        assert_eq!(fmt_short_usd(850_000_000.0), "$850M");
        assert_eq!(fmt_short_usd(2_300_000.0), "$2.3M");
        assert_eq!(fmt_short_usd(1_000_000_000.0), "$1B");
        assert_eq!(fmt_short_usd(999_000_000.0), "$999M");
    }

    #[test]
    fn short_usd_handles_rounding_up_negatives_and_small_values() {
        assert_eq!(fmt_short_usd(999_960_000.0), "$1B");
        assert_eq!(fmt_short_usd(-2_300_000.0), "−$2.3M");
        assert_eq!(fmt_short_usd(1_234_567.0), "$1.2M");
        assert_eq!(fmt_short_usd(54_321.0), "$54,321");
        assert_eq!(fmt_short_usd(0.0), "$0");
    }

    #[test]
    fn signed_pct_uses_unicode_minus() {
        assert_eq!(fmt_signed_pct(3.03), "+3.03%");
        assert_eq!(fmt_signed_pct(-23.67), "−23.67%");
        assert_eq!(fmt_signed_pct(0.0), "0.00%");
        assert_eq!(fmt_signed_pct(-0.001), "0.00%");
    }

    #[test]
    fn eps_formats_negative_with_unicode_minus() {
        assert_eq!(fmt_eps(7.08), "$7.08");
        assert_eq!(fmt_eps(-0.5), "−$0.50");
        assert_eq!(fmt_eps(0.0), "$0.00");
    }

    #[test]
    fn image_ext_detects_png_jpg_and_rejects_other() {
        assert_eq!(
            image_ext(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A]),
            Some("png")
        );
        assert_eq!(image_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_ext(b"GIF89a"), None);
        assert_eq!(image_ext(&[0xFF, 0xD8]), None);
        assert_eq!(image_ext(&[]), None);
    }

    #[test]
    fn earnings_lines_prefer_gaap_then_non_gaap_then_period() {
        let mut f = facts();
        f.metrics.revenue = fig(
            10.4e9,
            Unit::Usd,
            "$10.4 billion",
            "revenue of $10.4 billion",
        );
        f.metrics.eps_gaap = fig(7.08, Unit::UsdPerShare, "$7.08", "EPS of $7.08");
        f.metrics.eps_non_gaap = fig(7.5, Unit::UsdPerShare, "$7.50", "non-GAAP EPS of $7.50");
        let l = earnings_lines(&f);
        assert_eq!(l.en, ["Revenue $10.4B", "GAAP EPS $7.08"]);
        assert_eq!(l.zh, ["营收 104亿美元", "GAAP每股收益 7.08美元"]);

        f.metrics.eps_gaap = Field::Missing {
            reason: "absent".into(),
        };
        f.metrics.eps_non_gaap = fig(1.85, Unit::UsdPerShare, "$1.85", "non-GAAP EPS of $1.85");
        let l = earnings_lines(&f);
        assert_eq!(l.en[1], "Non-GAAP EPS $1.85");
        assert_eq!(l.zh[1], "非GAAP每股收益 1.85美元");

        f.metrics.eps_non_gaap = fig(
            -0.5,
            Unit::UsdPerShare,
            "$(0.50)",
            "non-GAAP loss of $(0.50)",
        );
        let l = earnings_lines(&f);
        assert_eq!(l.en[1], "Non-GAAP EPS −$0.50");
        assert_eq!(l.zh[1], "非GAAP每股收益 −0.50美元");
        f.metrics.eps_gaap = fig(-0.5, Unit::UsdPerShare, "$(0.50)", "GAAP loss of $(0.50)");
        let l = earnings_lines(&f);
        assert_eq!(l.en[1], "GAAP EPS −$0.50");
        assert_eq!(l.zh[1], "GAAP每股收益 −0.50美元");
        f.metrics.eps_gaap = Field::Missing {
            reason: "absent".into(),
        };

        f.metrics.eps_non_gaap = Field::Missing {
            reason: "absent".into(),
        };
        f.fiscal_period = Field::Value(FiscalPeriod {
            quarter: Some(3),
            fiscal_year: 2026,
            quote: "fiscal 2026 third quarter".into(),
        });
        let l = earnings_lines(&f);
        assert_eq!(l.en[1], "Q3 FY2026 results");
        assert_eq!(l.zh[1], "2026财年第三季度业绩");
    }

    #[test]
    fn earnings_lines_without_revenue_or_period_still_fill_both_lines() {
        let mut f = facts();
        f.metrics.revenue = Field::Missing {
            reason: "absent".into(),
        };
        f.metrics.eps_gaap = Field::Missing {
            reason: "absent".into(),
        };
        f.fiscal_period = Field::Missing {
            reason: "absent".into(),
        };
        let l = earnings_lines(&f);
        assert_eq!(l.en, ["Apple results", "Earnings results"]);
        assert_eq!(l.zh, ["Apple业绩", "财报业绩"]);
    }

    #[test]
    fn brief_lines_pick_largest_index_move_and_top_movers() {
        let indexes = vec![
            present("S&P 500", "^GSPC", 5850.0, 1.2),
            present("Nasdaq 100", "^NDX", 20100.0, -2.4),
            present("Dow Jones", "^DJI", 43000.0, 3.03),
            missing("Russell 2000"),
        ];
        let gainers = vec![mover("MOS", 17.37), mover("ABC", 9.0)];
        let losers = vec![mover("FIS", -23.67), mover("XYZ", -8.0)];
        let l = brief_lines(&indexes, &gainers, &losers, false);
        assert_eq!(l.en, ["Dow +3.03%", "MOS +17.37%, FIS −23.67%"]);
        assert_eq!(l.zh[0], "道指 +3.03%");
        assert_eq!(l.zh[1], l.en[1]);
    }

    #[test]
    fn brief_lines_omit_an_empty_mover_side_and_empty_inputs() {
        let indexes = vec![present("S&P 500", "^GSPC", 5850.0, -0.4)];
        let l = brief_lines(&indexes, &[], &[mover("FIS", -23.67)], false);
        assert_eq!(l.en, ["S&P 500 −0.40%", "FIS −23.67%"]);
        assert_eq!(l.zh[0], "标普500 −0.40%");
        let l = brief_lines(&[missing("S&P 500")], &[], &[], false);
        assert_eq!(l.en, ["", ""]);
        assert_eq!(l.zh, ["", ""]);
    }

    #[test]
    fn brief_lines_label_futures() {
        let futures = vec![
            present("S&P 500 futures", "ES=F", 5800.0, -1.14),
            present("Nasdaq 100 futures", "NQ=F", 20000.0, 0.2),
            present("Dow futures", "YM=F", 43000.0, 0.1),
            present("Russell 2000 futures", "RTY=F", 2200.0, 0.3),
        ];
        let l = brief_lines(&futures, &[mover("MOS", 5.0)], &[], true);
        assert_eq!(l.en, ["S&P futures −1.14%", "MOS +5.00%"]);
        assert_eq!(l.zh[0], "标普500期货 −1.14%");
    }

    #[test]
    fn index_snapshot_skips_missing_quotes() {
        let quotes = vec![
            present("S&P 500", "^GSPC", 5850.12, 0.51),
            missing("Nasdaq 100"),
            present("Dow Jones", "^DJI", 43000.5, -0.2),
        ];
        let s = index_snapshot(
            "recap",
            "2026-10-29T20:45:07.123Z".parse().unwrap(),
            &quotes,
        );
        assert_eq!(s.kind, "recap");
        assert_eq!(s.as_of_utc, "2026-10-29T20:45:07Z");
        let symbols: Vec<_> = s.entries.iter().map(|e| e.symbol.as_str()).collect();
        assert_eq!(symbols, ["^GSPC", "^DJI"]);
        assert_eq!(s.entries[1].last, 43000.5);
        assert_eq!(s.entries[1].change_pct, -0.2);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<IndexSnapshot>(&json).unwrap(), s);
    }

    #[test]
    fn scene_prompt_contains_no_people_no_text_cues() {
        let p = scene_prompt("a harbour of container ships");
        assert!(p.starts_with("a harbour of container ships"));
        for cue in ["no people", "no faces", "no text", "no logos", "no charts"] {
            assert!(p.contains(cue), "missing {cue}: {p}");
        }
    }

    #[test]
    fn brief_scene_rotates_by_weekday() {
        let mon: NaiveDate = "2026-11-02".parse().unwrap();
        let tue: NaiveDate = "2026-11-03".parse().unwrap();
        let next_mon: NaiveDate = "2026-11-09".parse().unwrap();
        assert_eq!(brief_scene_subject(mon), "a trading floor seen from above");
        assert_ne!(brief_scene_subject(mon), brief_scene_subject(tue));
        assert_eq!(brief_scene_subject(mon), brief_scene_subject(next_mon));
    }

    #[test]
    fn earnings_scene_subject_names_the_company() {
        assert!(earnings_scene_subject(&facts()).contains("the industry of Apple"));
    }

    #[test]
    fn earnings_scene_subject_truncates_and_strips_control_characters() {
        let mut f = facts();
        f.company = format!("A\nB{}", "x".repeat(68));
        let s = earnings_scene_subject(&f);
        let name = s
            .split("the industry of ")
            .nth(1)
            .and_then(|r| r.split(':').next())
            .unwrap();
        assert!(!name.contains('\n'));
        assert_eq!(name.chars().count(), 60);
        assert!(name.starts_with("AB"));
    }

    #[tokio::test]
    async fn no_cover_always_errs() {
        let e = NoCover.scene("anything").await.unwrap_err();
        assert_eq!(e, "cover maker not configured");
    }
}
