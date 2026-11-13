//! The cover graphic of an IPO write-up, one per milestone, in the frame and
//! palette of [`super::graphic`]: the price range as a band, a changed range
//! as the previous band over the new one, the offering price as a marker on
//! the last range, the first trading day as three bars, and a filing as the
//! first step of a four-step track. A withdrawal has no graphic.
//!
//! Facts first: every price drawn is a figure of the write-up's facts bundle
//! (`IpoArticleFacts`), formatted by the write-up's own `price`, and the
//! drawn text passes the number check against the write-up's
//! `allowed_numbers` before the graphic is returned. A milestone missing a
//! figure its graphic needs gets no graphic (the post keeps the flat tile and
//! the default share card), never a partial one. Labels are English, as on
//! every other cover: one share card serves both languages.

use super::graphic::{
    caption_and_close, label, label_width, open_svg, rect, text, CAPTION, CHAR_W, GAP, GREY, INK,
    LABEL_SIZE, MARGIN,
};
use crate::ipo::milestone::{IpoArticleFacts, Milestone};
use crate::ipo::writeup::{allowed_numbers, price};
use crate::numbers::{check, Lang};
use std::fmt::Write;

const WIDTH: f64 = 640.0;
/// A price scale never starts left of this (nor ends right of its mirror),
/// so a short label does not stretch a band across the whole cover.
const SCALE_LEFT: f64 = 140.0;
/// Past this the labels beside a band leave it no room: no graphic.
const SCALE_LEFT_MAX: f64 = 280.0;
const BAND_H: f64 = 28.0;
/// The caption's style, also used for the row labels of a changed range.
const SMALL_CAPS: &str = r#"font-size="11" font-weight="600" letter-spacing="1.5""#;

/// The cover graphic of an IPO write-up, or `None` (module docs).
pub fn ipo_graphic(f: &IpoArticleFacts) -> Option<String> {
    let svg = match f.milestone {
        Milestone::Filed => Some(filed()),
        Milestone::RangeSet => range_set(f),
        Milestone::RangeChanged => range_changed(f),
        Milestone::Priced => priced(f),
        Milestone::Listed => listed(f),
        Milestone::Withdrawn => None,
    }?;
    check(&drawn_text(&svg), Lang::En, &allowed_numbers(f)).ok()?;
    Some(svg)
}

/// The content of every `<text>`, one per line (none of it is escaped:
/// prices, fixed English words and captions only).
fn drawn_text(svg: &str) -> String {
    let mut parts: Vec<&str> = svg.split("</text>").collect();
    // What follows the last text is not a text.
    parts.pop();
    parts
        .into_iter()
        .filter_map(|s| s.rfind('>').map(|i| &s[i + 1..]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A price in whole cents, as the labels print it.
fn cents(v: f64) -> i128 {
    (v * 100.0).round() as i128
}

/// A finite price of at least a cent (a smaller one would print "$0.00").
fn positive(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && cents(*v) > 0)
}

/// A range whose low end prints below its high end.
fn range(lo: Option<f64>, hi: Option<f64>) -> Option<(f64, f64)> {
    let (lo, hi) = (positive(lo)?, positive(hi)?);
    (cents(lo) < cents(hi)).then_some((lo, hi))
}

/// Whether two ranges print the same.
fn same_range(a: (f64, f64), b: (f64, f64)) -> bool {
    (cents(a.0), cents(a.1)) == (cents(b.0), cents(b.1))
}

/// The width budgeted for `s` in the small caps of the caption and row
/// labels (11 px, 1.5 px letter spacing).
fn small_caps_width(s: &str) -> f64 {
    s.chars().count() as f64 * (11.0 / LABEL_SIZE * CHAR_W + 1.5)
}

fn en(v: f64) -> String {
    price(v, Lang::En)
}

/// "PER SHARE", or "PER ADS" for American depositary shares.
fn per(f: &IpoArticleFacts) -> &'static str {
    if f.security == "ads" {
        "PER ADS"
    } else {
        "PER SHARE"
    }
}

/// A label centred on `cx`, moved in when it would cross a margin.
fn centred(cx: f64, s: &str) -> f64 {
    let half = label_width(s) / 2.0;
    cx.min(WIDTH - MARGIN - half).max(MARGIN + half)
}

/// A horizontal price scale from `lo` to `hi`, with room beside both ends
/// for the widest of `labels`.
struct Scale {
    lo: f64,
    hi: f64,
    left: f64,
    right: f64,
}

impl Scale {
    fn new(lo: f64, hi: f64, labels: &[&str]) -> Option<Scale> {
        let longest = labels.iter().map(|l| label_width(l)).fold(0.0, f64::max);
        let left = SCALE_LEFT.max(MARGIN + GAP + longest);
        (left <= SCALE_LEFT_MAX && lo < hi).then_some(Scale {
            lo,
            hi,
            left,
            right: WIDTH - left,
        })
    }
    fn x(&self, v: f64) -> f64 {
        self.left + (v - self.lo) / (self.hi - self.lo) * (self.right - self.left)
    }
}

/// A range band at `y` with its two prices beside its ends.
fn band(out: &mut String, s: &Scale, (lo, hi): (f64, f64), y: f64, fill: &str, weight: u16) {
    let (x0, x1) = (s.x(lo), s.x(hi));
    rect(out, x0, y, (x1 - x0).max(1.0), BAND_H, fill);
    let baseline = y + BAND_H / 2.0 + LABEL_SIZE * 0.35;
    label(out, x0 - GAP, baseline, "end", weight, &en(lo));
    label(out, x1 + GAP, baseline, "start", weight, &en(hi));
}

/// `range_set`: the cover's range as one ink band.
fn range_set(f: &IpoArticleFacts) -> Option<String> {
    let r = range(f.num("price_low"), f.num("price_high"))?;
    one_band(r, &format!("IPO PRICE RANGE, {}", per(f)))
}

/// One range as an ink band, under `caption`.
fn one_band((lo, hi): (f64, f64), caption: &str) -> Option<String> {
    let s = Scale::new(lo, hi, &[&en(lo), &en(hi)])?;
    let mut out = open_svg();
    band(&mut out, &s, (lo, hi), 230.0, INK, 700);
    Some(caption_and_close(out, caption))
}

/// `range_changed`: the previous filing's range in grey above the cover's
/// new range in ink, on one scale, each named above its band. A change of
/// the share count alone keeps the range: one ink band, "UNCHANGED".
fn range_changed(f: &IpoArticleFacts) -> Option<String> {
    let p = f.previous.as_ref()?;
    let old = range(p.price_low, p.price_high)?;
    let new = range(f.num("price_low"), f.num("price_high"))?;
    if same_range(old, new) {
        return one_band(new, &format!("IPO PRICE RANGE, UNCHANGED, {}", per(f)));
    }
    let labels = [en(old.0), en(old.1), en(new.0), en(new.1)];
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let s = Scale::new(old.0.min(new.0), old.1.max(new.1), &refs)?;
    let mut out = open_svg();
    let attrs = format!(r#"{SMALL_CAPS} fill="{CAPTION}""#);
    for (r, y, fill, weight, name) in [
        (old, 204.0, GREY, 500, "PREVIOUS RANGE"),
        (new, 270.0, INK, 700, "NEW RANGE"),
    ] {
        // Above the band's start, moved in when it would cross the right margin.
        let x = s.x(r.0).min(WIDTH - MARGIN - small_caps_width(name));
        text(&mut out, x, y - GAP, &attrs, name);
        band(&mut out, &s, r, y, fill, weight);
    }
    Some(caption_and_close(
        out,
        &format!("IPO PRICE RANGE, {}", per(f)),
    ))
}

/// `priced`: the last range (the one the write-up compares the price with)
/// as a grey band, its ends labelled below it, and the offering price as an
/// ink marker labelled above it, inside or outside the band. With no range
/// on record, the price alone.
fn priced(f: &IpoArticleFacts) -> Option<String> {
    let p = positive(f.num("price_final"))?;
    let last = f
        .previous
        .as_ref()
        .and_then(|pr| range(pr.price_low, pr.price_high));
    let Some((lo, hi)) = last else {
        let mut out = open_svg();
        let attrs =
            format!(r#"font-size="44" font-weight="700" fill="{INK}" text-anchor="middle""#);
        text(&mut out, WIDTH / 2.0, 262.0, &attrs, &en(p));
        return Some(caption_and_close(out, &format!("IPO PRICE, {}", per(f))));
    };
    let (l, h, pl) = (en(lo), en(hi), en(p));
    let s = Scale::new(lo.min(p), hi.max(p), &[&l, &h, &pl])?;
    const BAND_Y: f64 = 244.0;
    const MARKER_RISE: f64 = 14.0;
    let mut out = open_svg();
    let (x0, x1) = (s.x(lo), s.x(hi));
    rect(&mut out, x0, BAND_Y, (x1 - x0).max(1.0), BAND_H, GREY);
    // The end labels, centred on the ends unless that makes them touch:
    // then spread evenly about the band's middle.
    let need = (label_width(&l) + label_width(&h)) / 2.0 + GAP;
    let (mut cl, mut ch) = (x0, x1);
    if ch - cl < need {
        let mid = (x0 + x1) / 2.0;
        (cl, ch) = (mid - need / 2.0, mid + need / 2.0);
    }
    let below = BAND_Y + BAND_H + GAP + LABEL_SIZE * 0.8;
    label(&mut out, centred(cl, &l), below, "middle", 500, &l);
    label(&mut out, centred(ch, &h), below, "middle", 500, &h);
    let xp = s.x(p);
    rect(
        &mut out,
        xp - 2.0,
        BAND_Y - MARKER_RISE,
        4.0,
        BAND_H + MARKER_RISE,
        INK,
    );
    label(
        &mut out,
        centred(xp, &pl),
        BAND_Y - MARKER_RISE - GAP,
        "middle",
        700,
        &pl,
    );
    Some(caption_and_close(
        out,
        &format!("IPO PRICE AND THE FILED RANGE, {}", per(f)),
    ))
}

/// `listed`: the offering price, the first session's open and its close as
/// bars on one zero baseline, the close in ink, each with its price above
/// and its name below.
fn listed(f: &IpoArticleFacts) -> Option<String> {
    let s = f.session.as_ref()?;
    // The write-up's "from the $X offering price" is the session's IPO price.
    let ipo = positive(s.ipo_price.or_else(|| f.num("price_final")))?;
    let open = positive(s.open)?;
    let close = positive(Some(s.close))?;
    let bars = [
        ("IPO price", ipo, GREY, 500),
        ("Open", open, GREY, 500),
        ("Close", close, INK, 700),
    ];
    const SLOT: f64 = 120.0;
    const BAR_W: f64 = 84.0;
    const PLOT_TOP: f64 = 180.0;
    const PLOT_BOTTOM: f64 = 296.0;
    // A value label wider than its slot would run into its neighbour's.
    if bars.iter().any(|b| label_width(&en(b.1)) > SLOT - GAP) {
        return None;
    }
    let left = (WIDTH - SLOT * bars.len() as f64) / 2.0;
    let max = bars.iter().map(|b| b.1).fold(0.0, f64::max);
    let mut out = open_svg();
    write!(
        out,
        r#"<line x1="{left:.1}" y1="{PLOT_BOTTOM:.1}" x2="{:.1}" y2="{PLOT_BOTTOM:.1}" stroke="{INK}" stroke-width="1"/>"#,
        WIDTH - left
    )
    .unwrap();
    for (i, (name, v, fill, weight)) in bars.into_iter().enumerate() {
        let cx = left + (i as f64 + 0.5) * SLOT;
        let h = (v / max * (PLOT_BOTTOM - PLOT_TOP)).max(1.0);
        let top = PLOT_BOTTOM - h;
        rect(&mut out, cx - BAR_W / 2.0, top, BAR_W, h, fill);
        let value = en(v);
        label(
            &mut out,
            centred(cx, &value),
            top - GAP,
            "middle",
            weight,
            &value,
        );
        label(
            &mut out,
            centred(cx, name),
            PLOT_BOTTOM + GAP + LABEL_SIZE * 0.8,
            "middle",
            500,
            name,
        );
    }
    Some(caption_and_close(
        out,
        &format!("FIRST TRADING DAY, {}", per(f)),
    ))
}

/// The steps of the `filed` track.
const STEPS: [&str; 4] = ["Filed", "Price range", "Priced", "Trading"];

/// `filed`: four steps, the first filled in ink and named in ink, the rest
/// grey.
fn filed() -> String {
    const TRACK_LEFT: f64 = 64.0;
    const TRACK_Y: f64 = 232.0;
    const TRACK_H: f64 = 12.0;
    const STEP_GAP: f64 = 6.0;
    let n = STEPS.len() as f64;
    let step = (WIDTH - 2.0 * TRACK_LEFT - STEP_GAP * (n - 1.0)) / n;
    let mut out = open_svg();
    let below = TRACK_Y + TRACK_H + GAP + LABEL_SIZE * 0.8;
    for (i, name) in STEPS.iter().enumerate() {
        let x = TRACK_LEFT + i as f64 * (step + STEP_GAP);
        let cx = centred(x + step / 2.0, name);
        if i == 0 {
            rect(&mut out, x, TRACK_Y, step, TRACK_H, INK);
            label(&mut out, cx, below, "middle", 700, name);
        } else {
            rect(&mut out, x, TRACK_Y, step, TRACK_H, GREY);
            let attrs = format!(
                r#"font-size="{LABEL_SIZE}" font-weight="500" fill="{CAPTION}" text-anchor="middle""#
            );
            text(&mut out, cx, below, &attrs, name);
        }
    }
    caption_and_close(out, "IPO PROGRESS")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::card::{card_svg, render_png};
    use crate::cover::graphic::tests::{rects, texts};
    use crate::ipo::milestone::{CoverFact, Previous, Session};
    use regex::Regex;

    const PAPER: &str = "#faf8f3";
    /// The live write-up's facts bundle, as stored in R2.
    const ANTHROPIC: &str = include_str!(
        "../../../../db/preview-facts/ipo/anthropic/range_set-0001628280-26-072484.json"
    );

    fn anthropic() -> IpoArticleFacts {
        serde_json::from_str(ANTHROPIC).unwrap()
    }

    fn fact(kind: &str, v: f64) -> CoverFact {
        CoverFact {
            kind: kind.into(),
            value_num: Some(v),
            value_text: None,
            quote: "q".into(),
        }
    }

    fn previous(lo: Option<f64>, hi: Option<f64>) -> Option<Previous> {
        Some(Previous {
            accession: "0001628280-26-072484".into(),
            form: "S-1/A".into(),
            filing_date: "2026-11-10".into(),
            index_url: "https://www.sec.gov/x-index.htm".into(),
            price_low: lo,
            price_high: hi,
            shares_offered: Some(825e6),
        })
    }

    /// Anthropic's range moved from $70.00–$78.00 to $76.00–$82.00.
    fn range_changed() -> IpoArticleFacts {
        let mut f = anthropic();
        f.milestone = Milestone::RangeChanged;
        f.previous = previous(Some(70.0), Some(78.0));
        for c in &mut f.cover {
            match c.kind.as_str() {
                "price_low" => c.value_num = Some(76.0),
                "price_high" => c.value_num = Some(82.0),
                _ => {}
            }
        }
        f
    }

    /// Priced at $80.00 by a 424B4, against the S-1/A's $70.00–$78.00.
    fn priced() -> IpoArticleFacts {
        let mut f = anthropic();
        f.milestone = Milestone::Priced;
        f.cover = vec![fact("price_final", 80.0), fact("shares_offered", 825e6)];
        f.previous = previous(Some(70.0), Some(78.0));
        f
    }

    /// The first session: IPO price $80.00, open $95.00, close $101.50.
    fn listed() -> IpoArticleFacts {
        let mut f = priced();
        f.milestone = Milestone::Listed;
        f.previous = None;
        f.session = Some(Session {
            session_date: "2026-11-19".into(),
            symbol: "ANT".into(),
            open: Some(95.0),
            close: 101.5,
            volume: Some(120e6),
            ipo_price: Some(80.0),
            change_pct: Some(26.88),
            source_url: "https://finance.yahoo.com/quote/ANT".into(),
        });
        f
    }

    fn filed() -> IpoArticleFacts {
        let mut f = anthropic();
        f.milestone = Milestone::Filed;
        f.cover.retain(|c| !c.kind.starts_with("price_"));
        f
    }

    fn all() -> Vec<(&'static str, IpoArticleFacts)> {
        vec![
            ("filed", filed()),
            ("range_set", anthropic()),
            ("range_changed", range_changed()),
            ("priced", priced()),
            ("listed", listed()),
        ]
    }

    /// Every `<text>` as (x, y, anchor, font size, content).
    fn anchored(svg: &str) -> Vec<(f64, f64, String, f64, String)> {
        let re = Regex::new(r#"<text x="([-\d.]+)" y="([-\d.]+)" ([^>]*)>([^<]*)</text>"#).unwrap();
        let attr = |attrs: &str, name: &str| {
            Regex::new(&format!(r#"{name}="([^"]+)""#))
                .unwrap()
                .captures(attrs)
                .map(|c| c[1].to_string())
        };
        re.captures_iter(svg)
            .map(|c| {
                (
                    c[1].parse().unwrap(),
                    c[2].parse().unwrap(),
                    attr(&c[3], "text-anchor").unwrap_or_else(|| "start".into()),
                    attr(&c[3], "font-size").unwrap().parse().unwrap(),
                    c[4].to_string(),
                )
            })
            .collect()
    }

    /// The horizontal extent of a text, at 10 px a character per 15 px of
    /// font size (letter-spaced small caps at 1.5 px more).
    fn extent(t: &(f64, f64, String, f64, String)) -> (f64, f64) {
        let per = if t.3 < LABEL_SIZE {
            t.3 / LABEL_SIZE * CHAR_W + 1.5
        } else {
            t.3 / LABEL_SIZE * CHAR_W
        };
        let w = per * t.4.chars().count() as f64;
        match t.2.as_str() {
            "middle" => (t.0 - w / 2.0, t.0 + w / 2.0),
            "end" => (t.0 - w, t.0),
            _ => (t.0, t.0 + w),
        }
    }

    /// The shared frame: root, paper, closing tag; every mark inside the
    /// chart area and the side margins; every text inside the margins and
    /// below the chart top, and no two texts on one line overlapping.
    fn assert_in_frame(svg: &str) {
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">"#));
        assert!(
            svg.contains(r##"<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>"##)
                && svg.ends_with("</svg>")
        );
        for r in rects(svg).iter().filter(|r| r.4 != PAPER) {
            assert!(r.0 >= 24.0 && r.0 + r.2 <= 616.0, "{r:?}");
            assert!(r.1 >= 151.2 && r.1 + r.3 <= 335.0, "{r:?}");
            assert!(r.2 > 0.0 && r.3 > 0.0, "{r:?}");
        }
        let ts = anchored(svg);
        for t in &ts {
            let (a, b) = extent(t);
            assert!(a >= 24.0 && b <= 616.0, "{t:?} spans {a}..{b}");
            assert!(t.1 - t.3 * 0.75 >= 151.2 && t.1 <= 360.0, "{t:?}");
        }
        for (i, s) in ts.iter().enumerate() {
            for t in &ts[i + 1..] {
                if (s.1 - t.1).abs() < s.3.max(t.3) {
                    let (a, b) = (extent(s), extent(t));
                    assert!(a.1 <= b.0 || b.1 <= a.0, "{s:?} overlaps {t:?}");
                }
            }
        }
        for gone in ["gradient", "opacity", "filter", "NaN", "inf"] {
            assert!(!svg.contains(gone), "{gone} in {svg}");
        }
    }

    /// Every figure the graphic prints.
    fn figures(svg: &str) -> Vec<String> {
        let re = Regex::new(r"\$[\d,]+\.\d{2}").unwrap();
        texts(svg)
            .iter()
            .flat_map(|t| {
                re.find_iter(&t.2)
                    .map(|m| m.as_str().to_string())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn every_figure_drawn_is_in_the_facts_bundle_as_the_write_up_formats_it() {
        for (name, f) in all() {
            let svg = ipo_graphic(&f).unwrap_or_else(|| panic!("{name}: no graphic"));
            assert_in_frame(&svg);
            let allowed: Vec<String> = allowed_numbers(&f).into_iter().map(en).collect();
            for fig in figures(&svg) {
                assert!(allowed.contains(&fig), "{name}: {fig} not in {allowed:?}");
            }
            // No digit is printed outside those figures.
            let re = Regex::new(r"\$[\d,]+\.\d{2}").unwrap();
            for t in texts(&svg) {
                let rest = re.replace_all(&t.2, "");
                assert!(
                    !rest.contains(|c: char| c.is_ascii_digit()),
                    "{name}: {t:?}"
                );
            }
        }
    }

    #[test]
    fn range_set_draws_the_live_anthropic_range_as_one_ink_band() {
        let f = anthropic();
        let svg = ipo_graphic(&f).unwrap();
        assert_eq!(figures(&svg), ["$70.00", "$78.00"]);
        assert!(svg.contains(">IPO PRICE RANGE, PER SHARE<"));
        let bands: Vec<_> = rects(&svg).into_iter().filter(|r| r.4 != PAPER).collect();
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].4, INK);
        // Each price beside its own end of the band.
        let ts = anchored(&svg);
        let lo = ts.iter().find(|t| t.4 == "$70.00").unwrap();
        let hi = ts.iter().find(|t| t.4 == "$78.00").unwrap();
        assert_eq!((lo.2.as_str(), hi.2.as_str()), ("end", "start"));
        assert!((lo.0 - (bands[0].0 - 8.0)).abs() <= 0.1, "{lo:?}");
        assert!(
            (hi.0 - (bands[0].0 + bands[0].2 + 8.0)).abs() <= 0.1,
            "{hi:?}"
        );
        // ADSs say so.
        let mut ads = f.clone();
        ads.security = "ads".into();
        assert!(ipo_graphic(&ads)
            .unwrap()
            .contains(">IPO PRICE RANGE, PER ADS<"));
    }

    #[test]
    fn range_changed_puts_the_previous_range_in_grey_above_the_new_in_ink() {
        let svg = ipo_graphic(&range_changed()).unwrap();
        assert_eq!(figures(&svg), ["$70.00", "$78.00", "$76.00", "$82.00"]);
        let bands: Vec<_> = rects(&svg).into_iter().filter(|r| r.4 != PAPER).collect();
        assert_eq!(bands.len(), 2);
        let (old, new) = (&bands[0], &bands[1]);
        assert_eq!((old.4.as_str(), new.4.as_str()), (GREY, INK));
        assert!(old.1 + old.3 < new.1, "{old:?} {new:?}");
        // One scale: the new range starts right of the old one and ends past it.
        assert!(new.0 > old.0 && new.0 + new.2 > old.0 + old.2);
        assert!(svg.contains(">PREVIOUS RANGE<") && svg.contains(">NEW RANGE<"));
        // A range_changed without the earlier range has no graphic.
        let mut f = range_changed();
        f.previous = None;
        assert_eq!(ipo_graphic(&f), None);
        f.previous = previous(Some(70.0), None);
        assert_eq!(ipo_graphic(&f), None);
    }

    /// A share-count change alone: the S-1/A of 2026-11-14 keeps $70.00–$78.00.
    fn shares_only() -> IpoArticleFacts {
        let mut f = anthropic();
        f.milestone = Milestone::RangeChanged;
        f.previous = previous(Some(70.0), Some(78.0));
        f
    }

    #[test]
    fn a_share_count_change_alone_draws_the_kept_range_once() {
        let svg = ipo_graphic(&shares_only()).unwrap();
        assert_in_frame(&svg);
        assert_eq!(figures(&svg), ["$70.00", "$78.00"]);
        let bands: Vec<_> = rects(&svg).into_iter().filter(|r| r.4 != PAPER).collect();
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].4, INK);
        assert!(
            svg.contains(">IPO PRICE RANGE, UNCHANGED, PER SHARE<"),
            "{svg}"
        );
        assert!(!svg.contains("NEW RANGE") && !svg.contains("PREVIOUS RANGE"));
        // The same as the range_set band, under its own caption.
        assert_eq!(
            svg.replace(", UNCHANGED,", ","),
            ipo_graphic(&anthropic()).unwrap()
        );
        // Equal in cents counts as kept.
        let mut f = shares_only();
        f.previous = previous(Some(70.001), Some(77.998));
        assert!(ipo_graphic(&f).unwrap().contains("UNCHANGED"));
        let mut ads = shares_only();
        ads.security = "ads".into();
        assert!(ipo_graphic(&ads)
            .unwrap()
            .contains(">IPO PRICE RANGE, UNCHANGED, PER ADS<"));
    }

    #[test]
    fn prices_that_print_alike_or_as_zero_give_no_graphic() {
        // Ends less than a cent apart would print "$70.00" twice.
        let mut f = anthropic();
        for c in &mut f.cover {
            match c.kind.as_str() {
                "price_low" => c.value_num = Some(70.001),
                "price_high" => c.value_num = Some(70.004),
                _ => {}
            }
        }
        assert_eq!(ipo_graphic(&f), None);
        // A sub-cent price would print "$0.00".
        let mut f = priced();
        f.cover[0].value_num = Some(0.004);
        assert_eq!(ipo_graphic(&f), None);
        let mut f = listed();
        f.session.as_mut().unwrap().open = Some(0.001);
        assert_eq!(ipo_graphic(&f), None);
        let mut f = range_changed();
        f.previous = previous(Some(0.001), Some(78.0));
        assert_eq!(ipo_graphic(&f), None);
    }

    #[test]
    fn listed_values_too_wide_for_their_slots_give_no_graphic() {
        let mut f = listed();
        f.session.as_mut().unwrap().close = 1e9;
        assert_eq!(ipo_graphic(&f), None);
        // The widest that fits: 11 characters at 10 px within a 120 px slot.
        let mut f = listed();
        f.session.as_mut().unwrap().close = 99_999.99;
        assert_in_frame(&ipo_graphic(&f).unwrap());
    }

    #[test]
    fn a_row_label_near_the_right_edge_is_moved_in() {
        // The old range high above a much lower new one: "PREVIOUS RANGE"
        // starts near the right end of the scale.
        let mut f = range_changed();
        f.previous = previous(Some(25.8), Some(26.0));
        for c in &mut f.cover {
            match c.kind.as_str() {
                "price_low" => c.value_num = Some(10.0),
                "price_high" => c.value_num = Some(12.0),
                _ => {}
            }
        }
        let svg = ipo_graphic(&f).unwrap();
        assert_in_frame(&svg);
        let label = anchored(&svg)
            .into_iter()
            .find(|t| t.4 == "PREVIOUS RANGE")
            .unwrap();
        assert!(extent(&label).1 <= 616.0, "{label:?}");
    }

    #[test]
    fn priced_marks_the_price_on_the_last_range() {
        let svg = ipo_graphic(&priced()).unwrap();
        assert_eq!(figures(&svg), ["$70.00", "$78.00", "$80.00"]);
        let marks: Vec<_> = rects(&svg).into_iter().filter(|r| r.4 != PAPER).collect();
        let band = marks.iter().find(|r| r.4 == GREY).unwrap();
        let marker = marks.iter().find(|r| r.4 == INK).unwrap();
        // $80.00 is above the range: right of the band's end.
        assert!(marker.0 > band.0 + band.2, "{marker:?} {band:?}");
        let ts = anchored(&svg);
        let p = ts.iter().find(|t| t.4 == "$80.00").unwrap();
        assert!(p.1 < marker.1, "{p:?} above {marker:?}");
        assert!(svg.contains(">IPO PRICE AND THE FILED RANGE, PER SHARE<"));

        // Inside the range, the marker is inside the band.
        let mut f = priced();
        f.cover[0].value_num = Some(74.5);
        let svg = ipo_graphic(&f).unwrap();
        assert_in_frame(&svg);
        let marks = rects(&svg);
        let band = marks.iter().find(|r| r.4 == GREY).unwrap();
        let marker = marks.iter().find(|r| r.4 == INK).unwrap();
        assert!(marker.0 > band.0 && marker.0 + marker.2 < band.0 + band.2);
        assert!(svg.contains(">$74.50<"));
    }

    #[test]
    fn priced_far_outside_a_narrow_range_keeps_its_labels_apart() {
        for p in [400.0, 10.0] {
            let mut f = priced();
            f.cover[0].value_num = Some(p);
            let svg = ipo_graphic(&f).unwrap();
            assert_in_frame(&svg);
        }
    }

    #[test]
    fn priced_without_a_range_shows_the_price_alone() {
        let mut f = priced();
        f.previous = None;
        let svg = ipo_graphic(&f).unwrap();
        assert_in_frame(&svg);
        assert_eq!(figures(&svg), ["$80.00"]);
        assert!(svg.contains(">IPO PRICE, PER SHARE<"));
        assert!(rects(&svg).iter().all(|r| r.4 == PAPER));
        // A range on record but broken is no range: the price alone too.
        f.previous = previous(Some(78.0), Some(70.0));
        assert_eq!(figures(&ipo_graphic(&f).unwrap()), ["$80.00"]);
    }

    #[test]
    fn listed_draws_the_ipo_price_the_open_and_the_close() {
        let svg = ipo_graphic(&listed()).unwrap();
        assert_eq!(figures(&svg), ["$80.00", "$95.00", "$101.50"]);
        let bars: Vec<_> = rects(&svg).into_iter().filter(|r| r.4 != PAPER).collect();
        assert_eq!(bars.len(), 3);
        let fills: Vec<&str> = bars.iter().map(|b| b.4.as_str()).collect();
        assert_eq!(fills, [GREY, GREY, INK]);
        // One baseline, heights in proportion to the prices.
        assert!(bars
            .iter()
            .all(|b| (b.1 + b.3 - (bars[0].1 + bars[0].3)).abs() <= 0.1));
        assert!((bars[0].3 / bars[2].3 - 80.0 / 101.5).abs() < 0.01);
        for name in [
            ">IPO price<",
            ">Open<",
            ">Close<",
            ">FIRST TRADING DAY, PER SHARE<",
        ] {
            assert!(svg.contains(name), "{name}");
        }
        // The offering price from the final cover when the session has none.
        let mut f = listed();
        f.session.as_mut().unwrap().ipo_price = None;
        assert_eq!(figures(&ipo_graphic(&f).unwrap())[0], "$80.00");
    }

    #[test]
    fn filed_fills_the_first_of_four_steps() {
        let svg = ipo_graphic(&filed()).unwrap();
        let steps: Vec<_> = rects(&svg).into_iter().filter(|r| r.4 != PAPER).collect();
        let fills: Vec<&str> = steps.iter().map(|s| s.4.as_str()).collect();
        assert_eq!(fills, [INK, GREY, GREY, GREY]);
        assert!(steps.windows(2).all(|w| w[0].0 + w[0].2 < w[1].0));
        let names: Vec<String> = texts(&svg).into_iter().map(|t| t.2).collect();
        assert_eq!(
            names,
            ["Filed", "Price range", "Priced", "Trading", "IPO PROGRESS"]
        );
    }

    #[test]
    fn a_milestone_without_its_figures_has_no_graphic() {
        let mut withdrawn = anthropic();
        withdrawn.milestone = Milestone::Withdrawn;
        assert_eq!(ipo_graphic(&withdrawn), None);

        let mut f = anthropic();
        f.cover.retain(|c| c.kind != "price_high");
        assert_eq!(ipo_graphic(&f), None);
        let mut f = anthropic();
        for c in &mut f.cover {
            if c.kind == "price_low" {
                c.value_num = Some(78.0);
            }
        }
        assert_eq!(ipo_graphic(&f), None, "a range with equal ends");
        let mut f = anthropic();
        f.cover
            .iter_mut()
            .find(|c| c.kind == "price_low")
            .unwrap()
            .value_num = Some(f64::NAN);
        assert_eq!(ipo_graphic(&f), None);

        let mut f = priced();
        f.cover.retain(|c| c.kind != "price_final");
        assert_eq!(ipo_graphic(&f), None);
        let mut f = priced();
        f.cover[0].value_num = Some(0.0);
        assert_eq!(ipo_graphic(&f), None);

        let mut f = listed();
        f.session = None;
        assert_eq!(ipo_graphic(&f), None);
        let mut f = listed();
        f.session.as_mut().unwrap().open = None;
        assert_eq!(ipo_graphic(&f), None);
        let mut f = listed();
        f.session.as_mut().unwrap().ipo_price = None;
        f.cover.retain(|c| c.kind != "price_final");
        assert_eq!(ipo_graphic(&f), None);
    }

    #[test]
    fn labels_too_wide_for_a_band_give_no_graphic() {
        let mut f = anthropic();
        for c in &mut f.cover {
            match c.kind.as_str() {
                "price_low" => c.value_num = Some(1e20),
                "price_high" => c.value_num = Some(2e20),
                _ => {}
            }
        }
        assert_eq!(ipo_graphic(&f), None);
    }

    #[test]
    fn drawn_text_reads_every_label() {
        let svg = ipo_graphic(&priced()).unwrap();
        assert_eq!(
            drawn_text(&svg),
            "$70.00\n$78.00\n$80.00\nIPO PRICE AND THE FILED RANGE, PER SHARE"
        );
    }

    #[test]
    fn the_preview_cover_is_the_graphic_of_the_preview_facts() {
        let svg = include_str!(
            "../../../../db/preview-facts/covers/anthropic-ipo-price-range-2026-11-10.svg"
        );
        assert_eq!(Some(svg.trim_end().to_string()), ipo_graphic(&anthropic()));
        let seed = include_str!("../../../../db/seed-ipo-articles-preview.sql");
        assert!(seed.contains("'covers/anthropic-ipo-price-range-2026-11-10.svg'"));
    }

    #[test]
    fn every_ipo_cover_renders_as_a_share_card() {
        for (name, f) in all() {
            let svg = card_svg(&ipo_graphic(&f).unwrap(), "ANTHROPIC IPO · 2026-11-10").unwrap();
            let png = render_png(&svg).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{name}");
            // Every character printed has a glyph in every embedded font.
            let chars: Vec<char> = texts(&svg).iter().flat_map(|t| t.2.chars()).collect();
            for font in crate::cover::card::FONTS {
                let face = ttf_parser::Face::parse(font, 0).unwrap();
                let missing: Vec<&char> = chars
                    .iter()
                    .filter(|&&c| c != ' ' && face.glyph_index(c).is_none())
                    .collect();
                assert!(missing.is_empty(), "{name}: missing glyphs {missing:?}");
            }
        }
    }
}
