//! The cover graphic of a published post: an SVG chart of the story's own
//! figures on paper, written by string formatting.
//!
//! Every graphic shares one frame: a 640×360 viewBox on paper, the chart in
//! the lower 58% with 24 px clear at both sides (the upper 42% is left for
//! the HTML lines), and a grey caption bottom-left. All text goes through
//! [`escape`]; all numbers through the formatters in [`crate::cover`].

use std::fmt::Write;

use super::{fmt_eps, fmt_short_usd, fmt_signed_pct};
use crate::earnings::charts::{series, Bar, Series, MIN_BARS};
use crate::earnings::facts::{numbers_in_facts, EarningsFacts};
use crate::facts::Mover;
use crate::numbers::{check, Lang};

const INK: &str = "#1a1a1a";
const UP: &str = "#15803d";
const DOWN: &str = "#b91c1c";
const GREY: &str = "#d9d5cc";
const CAPTION: &str = "#666";

/// Side margin kept clear of every mark and label.
const MARGIN: f64 = 24.0;
/// Top of the chart area: the lower 58% of the 360-unit height.
const CHART_TOP: f64 = 151.2;
/// Caption baseline, bottom-left.
const CAPTION_Y: f64 = 350.0;
/// Label size, and the width budgeted per label character at that size
/// (generous for Inter, so an estimate never overruns the margin).
const LABEL_SIZE: f64 = 15.0;
const CHAR_W: f64 = 10.0;
/// Space between a bar and its labels.
const GAP: f64 = 8.0;

/// One mover as the brief cover draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverMover {
    pub symbol: String,
    pub change_pct: f64,
}

/// Which brief the cover belongs to; it picks the caption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Premarket,
    Recap,
}

/// The brief's movers for the cover: gainers then losers, in list order.
pub fn cover_movers(gainers: &[Mover], losers: &[Mover]) -> Vec<CoverMover> {
    gainers
        .iter()
        .chain(losers)
        .map(|m| CoverMover {
            symbol: m.quote.symbol.clone(),
            change_pct: m.quote.change_pct,
        })
        .collect()
}

/// XML-escapes text content and attribute values.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The root element every graphic opens with (the share card strips it).
pub(crate) const ROOT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">"#;
/// The paper background, the root's first child.
pub(crate) const BACKGROUND: &str =
    r##"<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>"##;

/// The root element and the paper background.
fn open_svg() -> String {
    format!("{ROOT}{BACKGROUND}")
}

/// One rect in the shape the tests parse.
fn rect(out: &mut String, x: f64, y: f64, w: f64, h: f64, fill: &str) {
    write!(
        out,
        r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="{fill}"/>"#
    )
    .unwrap();
}

/// One text; `attrs` follows x and y, `content` is escaped here.
fn text(out: &mut String, x: f64, y: f64, attrs: &str, content: &str) {
    write!(
        out,
        r#"<text x="{x:.1}" y="{y:.1}" {attrs}>{}</text>"#,
        escape(content)
    )
    .unwrap();
}

/// A label in ink at the label size, anchored `anchor`.
fn label(out: &mut String, x: f64, y: f64, anchor: &str, weight: u16, content: &str) {
    let attrs = format!(
        r#"font-size="{LABEL_SIZE}" font-weight="{weight}" fill="{INK}" text-anchor="{anchor}""#
    );
    text(out, x, y, &attrs, content);
}

/// The grey, letter-spaced caption bottom-left, then the closing tag.
fn caption_and_close(mut out: String, caption: &str) -> String {
    let attrs =
        format!(r#"font-size="11" font-weight="600" letter-spacing="1.5" fill="{CAPTION}""#);
    text(&mut out, MARGIN, CAPTION_Y, &attrs, caption);
    out.push_str("</svg>");
    out
}

/// The width budgeted for `s` at the label size.
fn label_width(s: &str) -> f64 {
    s.chars().count() as f64 * CHAR_W
}

/// The brief cover: the biggest movers as a diverging bar chart. Gainers
/// (largest first) sit above losers (largest loss first), at most four of
/// each; a move that is non-finite or rounds to 0.00% is skipped. None when
/// no move is left, so the post keeps the flat tile.
pub fn brief_graphic(movers: &[CoverMover], session: Session) -> Option<String> {
    let drawable =
        |m: &&CoverMover| m.change_pct.is_finite() && fmt_signed_pct(m.change_pct) != "0.00%";
    let mut up: Vec<&CoverMover> = movers
        .iter()
        .filter(drawable)
        .filter(|m| m.change_pct > 0.0)
        .collect();
    let mut down: Vec<&CoverMover> = movers
        .iter()
        .filter(drawable)
        .filter(|m| m.change_pct < 0.0)
        .collect();
    up.sort_by(|a, b| b.change_pct.total_cmp(&a.change_pct));
    down.sort_by(|a, b| a.change_pct.total_cmp(&b.change_pct));
    up.truncate(4);
    down.truncate(4);
    let rows: Vec<(&CoverMover, String)> = up
        .into_iter()
        .chain(down)
        .map(|m| (m, fmt_signed_pct(m.change_pct)))
        .collect();
    if rows.is_empty() {
        return None;
    }

    const AXIS_X: f64 = 320.0;
    const ROWS_TOP: f64 = 156.0;
    const ROWS_BOTTOM: f64 = 332.0;
    const PITCH: f64 = 21.0;
    const BAR_H: f64 = 15.0;
    const BAR_MAX: f64 = 220.0;
    // The largest move gets one length on both sides, short enough that the
    // longest value label still ends inside the margin.
    let longest = rows.iter().map(|(_, v)| label_width(v)).fold(0.0, f64::max);
    let full = BAR_MAX.min(AXIS_X - MARGIN - GAP - longest).max(1.0);
    let max_abs = rows
        .iter()
        .map(|(m, _)| m.change_pct.abs())
        .fold(0.0, f64::max);
    // Rows are centred in the space above the caption.
    let block = rows.len() as f64 * PITCH;
    let top = ROWS_TOP + (ROWS_BOTTOM - ROWS_TOP - block) / 2.0;

    let mut out = open_svg();
    write!(
        out,
        r#"<line x1="{AXIS_X:.1}" y1="{:.1}" x2="{AXIS_X:.1}" y2="{:.1}" stroke="{INK}" stroke-width="1"/>"#,
        (top - 4.0).max(CHART_TOP),
        top + block + 4.0
    )
    .unwrap();
    for (i, (m, value)) in rows.iter().enumerate() {
        let y = top + i as f64 * PITCH + (PITCH - BAR_H) / 2.0;
        let baseline = y + BAR_H / 2.0 + LABEL_SIZE * 0.35;
        // A floor of one unit keeps the smallest move visible as a stub.
        let w = (full * m.change_pct.abs() / max_abs).max(1.0);
        if m.change_pct > 0.0 {
            rect(&mut out, AXIS_X, y, w, BAR_H, UP);
            label(&mut out, AXIS_X - GAP, baseline, "end", 700, &m.symbol);
            label(&mut out, AXIS_X + w + GAP, baseline, "start", 500, value);
        } else {
            rect(&mut out, AXIS_X - w, y, w, BAR_H, DOWN);
            label(&mut out, AXIS_X + GAP, baseline, "start", 700, &m.symbol);
            label(&mut out, AXIS_X - w - GAP, baseline, "end", 500, value);
        }
    }
    let caption = match session {
        Session::Premarket => "BIGGEST PREMARKET MOVES",
        Session::Recap => "BIGGEST MOVES AT THE CLOSE",
    };
    Some(caption_and_close(out, caption))
}

/// Most slots the earnings cover draws, gaps included; older ones drop off.
const MAX_SLOTS: usize = 8;

/// The earnings cover: the earnings chart's revenue series (else its
/// diluted EPS series) as bars on one zero baseline, in grey. When the last
/// slot is this release's 8-K bar it is drawn in ink with its value above
/// it, or below it for a loss; with no release bar every bar is grey and no
/// value is labelled, so the cover never shows a figure the lines do not.
/// None when neither series has `MIN_BARS` bars in its window or the value
/// label fails the number check, so the post keeps the flat tile.
pub fn earnings_graphic(f: &EarningsFacts) -> Option<String> {
    earnings_graphic_checked(f, &numbers_in_facts(f))
}

/// [`earnings_graphic`] against an explicit list of allowed numbers.
fn earnings_graphic_checked(f: &EarningsFacts, allowed: &[f64]) -> Option<String> {
    earnings_svg(&series(f), allowed)
}

/// A series' last `MAX_SLOTS` slots, starting on a bar rather than a gap.
fn window(slots: &[Option<Bar>]) -> &[Option<Bar>] {
    let mut slots = &slots[slots.len().saturating_sub(MAX_SLOTS)..];
    while let [None, rest @ ..] = slots {
        slots = rest;
    }
    slots
}

/// The earnings cover drawn from the first series with `MIN_BARS` bars in
/// its window.
fn earnings_svg(all: &[Series], allowed: &[f64]) -> Option<String> {
    let (s, slots) = all.iter().find_map(|s| {
        let w = window(&s.slots);
        (w.iter().flatten().count() >= MIN_BARS).then_some((s, w))
    })?;
    let Some(Some(last)) = slots.last() else {
        return None;
    };
    let (value, what) = match s.name {
        "revenue" => (fmt_short_usd(last.value), "REVENUE"),
        _ => (fmt_eps(last.value), "DILUTED EPS"),
    };
    // The ink bar and its value only for this release's own quarter, which `series` puts last.
    let value = if s.latest_is_release {
        check(&value, Lang::En, allowed).ok()?;
        Some(value)
    } else {
        None
    };

    const WIDTH: f64 = 640.0;
    // Room above for the value label and below for the period label.
    const PLOT_TOP: f64 = 180.0;
    const PLOT_BOTTOM: f64 = 300.0;
    /// Widest slot (bar plus gap), so a short series is not a wall of fat bars.
    const SLOT_MAX: f64 = 96.0;
    const BAR_SHARE: f64 = 0.7;
    let n = slots.len();
    let values = || slots.iter().flatten().map(|b| b.value);
    let lo = values().fold(0.0, f64::min);
    let mut hi = values().fold(0.0, f64::max);
    if hi == lo {
        // An all-zero series: a unit span keeps the baseline at the bottom.
        hi = 1.0;
    }
    let y = |v: f64| PLOT_TOP + (hi - v) / (hi - lo) * (PLOT_BOTTOM - PLOT_TOP);
    let zero = y(0.0);
    let slot = ((WIDTH - 2.0 * MARGIN) / n as f64).min(SLOT_MAX);
    let left = (WIDTH - slot * n as f64) / 2.0;
    let bar_w = slot * BAR_SHARE;
    // A label centred on `cx`, moved in when it would cross a margin.
    let centred = |cx: f64, s: &str| {
        let half = label_width(s) / 2.0;
        cx.min(WIDTH - MARGIN - half).max(MARGIN + half)
    };

    let mut out = open_svg();
    write!(
        out,
        r#"<line x1="{left:.1}" y1="{zero:.1}" x2="{:.1}" y2="{zero:.1}" stroke="{INK}" stroke-width="1"/>"#,
        left + slot * n as f64
    )
    .unwrap();
    for (i, b) in slots.iter().enumerate() {
        let Some(b) = b else { continue };
        let x = left + i as f64 * slot + (slot - bar_w) / 2.0;
        let cx = x + bar_w / 2.0;
        // A floor of one unit keeps a zero quarter visible as a stub.
        let h = (y(b.value) - zero).abs().max(1.0);
        let (top, bottom) = if b.value >= 0.0 {
            (zero - h, zero)
        } else {
            (zero, zero + h)
        };
        let ink = value.as_ref().filter(|_| i == n - 1);
        let fill = if ink.is_some() { INK } else { GREY };
        rect(&mut out, x, top, bar_w, h, fill);
        let below = bottom + GAP + LABEL_SIZE * 0.8;
        if i == 0 {
            label(
                &mut out,
                centred(cx, &b.label),
                below,
                "middle",
                500,
                &b.label,
            );
        }
        if let Some(value) = ink {
            let ly = if b.value >= 0.0 { top - GAP } else { below };
            label(&mut out, centred(cx, value), ly, "middle", 700, value);
        }
    }
    Some(caption_and_close(
        out,
        &format!("{what}, LAST {n} QUARTERS"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::facts::tests::{facts, fig};
    use crate::earnings::facts::{QuarterPoint, Trend, Unit};
    use crate::facts::{Field, Quote};
    use regex::Regex;

    const UP: &str = "#15803d";
    const DOWN: &str = "#b91c1c";
    const PAPER: &str = "#faf8f3";

    fn m(sym: &str, pct: f64) -> CoverMover {
        CoverMover {
            symbol: sym.to_string(),
            change_pct: pct,
        }
    }

    /// Every `<rect>` as (x, y, width, height, fill).
    fn rects(svg: &str) -> Vec<(f64, f64, f64, f64, String)> {
        let re = Regex::new(
            r#"<rect x="([-\d.]+)" y="([-\d.]+)" width="([-\d.]+)" height="([-\d.]+)" fill="([^"]+)"/>"#,
        )
        .unwrap();
        re.captures_iter(svg)
            .map(|c| {
                (
                    c[1].parse().unwrap(),
                    c[2].parse().unwrap(),
                    c[3].parse().unwrap(),
                    c[4].parse().unwrap(),
                    c[5].to_string(),
                )
            })
            .collect()
    }

    /// Every `<text>` as (x, y, content).
    fn texts(svg: &str) -> Vec<(f64, f64, String)> {
        let re = Regex::new(r#"<text x="([-\d.]+)" y="([-\d.]+)"[^>]*>([^<]*)</text>"#).unwrap();
        re.captures_iter(svg)
            .map(|c| {
                (
                    c[1].parse().unwrap(),
                    c[2].parse().unwrap(),
                    c[3].to_string(),
                )
            })
            .collect()
    }

    fn bars(svg: &str) -> Vec<(f64, f64, f64, f64, String)> {
        rects(svg)
            .into_iter()
            .filter(|r| r.4 == UP || r.4 == DOWN)
            .collect()
    }

    fn text_x(svg: &str, content: &str) -> f64 {
        texts(svg)
            .into_iter()
            .find(|t| t.2 == content)
            .unwrap_or_else(|| panic!("no text {content:?} in {svg}"))
            .0
    }

    #[test]
    fn brief_graphic_has_the_root_background_and_session_caption() {
        let svg = brief_graphic(&[m("MOS", 20.49), m("FIS", -11.77)], Session::Premarket).unwrap();
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">"#));
        assert!(
            svg.contains(r##"<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>"##)
                && svg.ends_with("</svg>")
        );
        assert!(
            svg.contains(">BIGGEST PREMARKET MOVES<")
                && svg.contains(">+20.49%<")
                && svg.contains(">−11.77%<")
        );
        for gone in ["gradient", "opacity", "filter"] {
            assert!(!svg.contains(gone), "{gone} in {svg}");
        }
        let recap = brief_graphic(&[m("MOS", 1.0)], Session::Recap).unwrap();
        assert!(recap.contains(">BIGGEST MOVES AT THE CLOSE<"));
    }

    #[test]
    fn brief_graphic_orders_and_colours_movers() {
        let svg = brief_graphic(
            &[m("B", 3.0), m("L1", -2.0), m("A", 9.0), m("L2", -7.5)],
            Session::Premarket,
        )
        .unwrap();
        let mut bs = bars(&svg);
        assert_eq!(bs.len(), 4);
        bs.sort_by(|a, b| a.1.total_cmp(&b.1));
        let fills: Vec<&str> = bs.iter().map(|b| b.4.as_str()).collect();
        assert_eq!(fills, [UP, UP, DOWN, DOWN]);
        for b in &bs {
            if b.4 == UP {
                assert!((b.0 - 320.0).abs() <= 0.1, "{b:?}");
            } else {
                assert!((b.0 + b.2 - 320.0).abs() <= 0.1, "{b:?}");
            }
        }
        // Each bar's ticker shares its row: A, B, L2, L1 by ascending y.
        let ts = texts(&svg);
        let row_of = |sym: &str| ts.iter().find(|t| t.2 == sym).unwrap().1;
        let ys: Vec<f64> = ["A", "B", "L2", "L1"].iter().map(|s| row_of(s)).collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
        for (b, y) in bs.iter().zip(&ys) {
            assert!(*y > b.1 && *y <= b.1 + b.3 + 2.0, "{b:?} vs {y}");
        }
        let values: Vec<&str> = ts
            .iter()
            .map(|t| t.2.as_str())
            .filter(|c| c.ends_with('%'))
            .collect();
        assert_eq!(values, ["+9.00%", "+3.00%", "−7.50%", "−2.00%"]);
    }

    #[test]
    fn brief_graphic_caps_four_per_side() {
        let gainers = (1..=6).map(|i| m(&format!("G{i}"), 7.0 - i as f64));
        let losers = (1..=6).map(|i| m(&format!("L{i}"), -(7.0 - i as f64)));
        let movers: Vec<CoverMover> = gainers.chain(losers).collect();
        let svg = brief_graphic(&movers, Session::Recap).unwrap();
        let bs = bars(&svg);
        assert_eq!(bs.iter().filter(|b| b.4 == UP).count(), 4);
        assert_eq!(bs.iter().filter(|b| b.4 == DOWN).count(), 4);
        for kept in [">G1<", ">G4<", ">L1<", ">L4<"] {
            assert!(svg.contains(kept), "{kept} missing");
        }
        for gone in [">G5<", ">G6<", ">L5<", ">L6<"] {
            assert!(!svg.contains(gone), "{gone} in {svg}");
        }
    }

    #[test]
    fn brief_graphic_skips_zero_and_non_finite_moves() {
        let svg = brief_graphic(
            &[
                m("ZERO", 0.0),
                m("TINY", -0.001),
                m("NANX", f64::NAN),
                m("INFX", f64::INFINITY),
                m("KEEP", 1.0),
            ],
            Session::Premarket,
        )
        .unwrap();
        assert_eq!(bars(&svg).len(), 1);
        for gone in [">ZERO<", ">TINY<", ">NANX<", ">INFX<", "NaN", "inf"] {
            assert!(!svg.contains(gone), "{gone} in {svg}");
        }
        assert!(svg.contains(">KEEP<"));
        assert_eq!(brief_graphic(&[m("ZERO", 0.0)], Session::Premarket), None);
        assert_eq!(brief_graphic(&[], Session::Recap), None);
    }

    #[test]
    fn brief_graphic_scales_bars_inside_the_chart_area() {
        let svg = brief_graphic(
            &[m("BIG", 20.0), m("HALF", 10.0), m("LOSS", -20.0)],
            Session::Premarket,
        )
        .unwrap();
        let ts = texts(&svg);
        let bs = bars(&svg);
        let width_of = |sym: &str| {
            let y = ts.iter().find(|t| t.2 == sym).unwrap().1;
            bs.iter()
                .find(|b| y > b.1 && y <= b.1 + b.3 + 2.0)
                .unwrap()
                .2
        };
        let (big, half, loss) = (width_of("BIG"), width_of("HALF"), width_of("LOSS"));
        assert!((big - loss).abs() <= 0.1, "{big} vs {loss}");
        assert!((half - big / 2.0).abs() <= 0.2, "{half} vs {big}");
        for r in rects(&svg).iter().filter(|r| r.4 != PAPER) {
            assert!(r.0 >= 24.0 && r.0 + r.2 <= 616.0, "{r:?}");
            assert!(r.1 >= 151.2 && r.1 + r.3 <= 360.0, "{r:?}");
        }
        for t in &ts {
            assert!(t.1 >= 151.2, "{t:?}");
        }
    }

    #[test]
    fn brief_graphic_keeps_long_labels_inside_the_margins() {
        let svg = brief_graphic(
            &[m("GOOGL", 1234.56), m("BRKBX", -987.65)],
            Session::Premarket,
        )
        .unwrap();
        let up = text_x(&svg, "+1234.56%");
        assert!(up <= 526.0, "{up}");
        // The far end of each start/end-anchored value label, at 10 px a character.
        assert!(
            up + 10.0 * "+1234.56%".chars().count() as f64 <= 616.0,
            "{up}"
        );
        let down = text_x(&svg, "−987.65%");
        assert!(down >= 114.0, "{down}");
        assert!(
            down - 10.0 * "−987.65%".chars().count() as f64 >= 24.0,
            "{down}"
        );
        let end = svg.find(">−987.65%</text>").unwrap();
        let element = &svg[svg[..end].rfind("<text").unwrap()..end];
        assert!(element.contains(r#"text-anchor="end""#), "{element}");
    }

    #[test]
    fn brief_graphic_never_draws_a_negative_width() {
        // A label longer than the room beside the axis leaves no bar length;
        // the bar is still a one-unit stub, never a negative width.
        let svg = brief_graphic(&[m("HUGE", 1e30), m("SMALL", 1.0)], Session::Recap).unwrap();
        for b in bars(&svg) {
            assert!(b.2 >= 1.0, "{b:?}");
        }
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }

    #[test]
    fn brief_graphic_escapes_text() {
        let svg = brief_graphic(&[m("A&B<", 2.0)], Session::Premarket).unwrap();
        assert!(svg.contains("A&amp;B&lt;"));
        assert!(!svg.contains("A&B<"));
    }

    #[test]
    fn cover_movers_takes_gainers_then_losers() {
        let mover = |sym: &str, pct: f64| Mover {
            quote: Quote {
                symbol: sym.to_string(),
                last: 50.0,
                prev_close: 50.0,
                change_pct: pct,
                volume: None,
            },
            name: format!("{sym} Inc"),
            reasons: vec![],
            fallback: None,
        };
        assert_eq!(
            cover_movers(&[mover("MOS", 17.37)], &[mover("FIS", -23.67)]),
            vec![m("MOS", 17.37), m("FIS", -23.67)]
        );
    }

    const INK: &str = "#1a1a1a";
    const GREY: &str = "#d9d5cc";

    fn qp(end: &str, value: f64) -> QuarterPoint {
        QuarterPoint {
            period_end: end.parse().unwrap(),
            value,
            form: "10-Q".into(),
            filed: "2026-08-01".parse().unwrap(),
        }
    }

    /// The earnings cover's bars: grey, and the one in ink.
    fn earnings_bars(svg: &str) -> Vec<(f64, f64, f64, f64, String)> {
        rects(svg)
            .into_iter()
            .filter(|r| r.4 == GREY || r.4 == INK)
            .collect()
    }

    /// The shared frame: root, paper, closing tag, and every mark and label
    /// inside the chart area and the side margins (centred labels measured
    /// at 10 px a character).
    fn assert_in_frame(svg: &str) {
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">"#));
        assert!(
            svg.contains(r##"<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>"##)
                && svg.ends_with("</svg>")
        );
        for r in rects(svg).iter().filter(|r| r.4 != PAPER) {
            assert!(r.0 >= 24.0 && r.0 + r.2 <= 616.0, "{r:?}");
            assert!(r.1 >= 151.2 && r.1 + r.3 <= 360.0, "{r:?}");
            assert!(r.2 > 0.0 && r.3 > 0.0, "{r:?}");
        }
        for t in texts(svg) {
            assert!(t.1 >= 151.2 && t.1 <= 360.0, "{t:?}");
            if !t.2.contains("QUARTERS") {
                let half = 5.0 * t.2.chars().count() as f64;
                assert!(t.0 - half >= 24.0 && t.0 + half <= 616.0, "{t:?}");
            }
        }
        for gone in ["gradient", "opacity", "filter", "NaN", "inf"] {
            assert!(!svg.contains(gone), "{gone} in {svg}");
        }
    }

    #[test]
    fn earnings_graphic_draws_revenue_with_the_release_in_ink() {
        let svg = earnings_graphic(&facts()).unwrap();
        assert_in_frame(&svg);
        // 2026-03 and 2026-06 grey, the release in ink, rightmost.
        assert_eq!(rects(&svg).iter().filter(|r| r.4 == GREY).count(), 2);
        assert_eq!(rects(&svg).iter().filter(|r| r.4 == INK).count(), 1);
        let bs = earnings_bars(&svg);
        let ink = bs.iter().find(|b| b.4 == INK).unwrap();
        assert!(bs.iter().all(|b| b.0 <= ink.0), "{bs:?}");
        assert!(
            svg.contains(">$102.5B<")
                && svg.contains(">2026-03<")
                && svg.contains(">REVENUE, LAST 3 QUARTERS<")
        );
        assert!(!svg.contains("8-K") && !svg.contains(">2026-06<"));
        // The value label is centred over the ink bar, above it.
        let ts = texts(&svg);
        let value = ts.iter().find(|t| t.2 == "$102.5B").unwrap();
        assert!((value.0 - (ink.0 + ink.2 / 2.0)).abs() <= 0.1, "{value:?}");
        assert!(value.1 < ink.1, "{value:?} {ink:?}");
    }

    #[test]
    fn earnings_graphic_inks_the_latest_quarter_when_the_10q_came_with_the_release() {
        let mut f = facts();
        // The 10-Q for the release quarter was filed with the 8-K, so the
        // release is the trend's last XBRL quarter, not an extra 8-K bar.
        f.filing_date = "2026-07-31".parse().unwrap();
        // Holds even when the press release's quarter was not verified.
        f.fiscal_period = Field::Missing { reason: "x".into() };
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        assert_eq!(rects(&svg).iter().filter(|r| r.4 == GREY).count(), 1);
        assert_eq!(rects(&svg).iter().filter(|r| r.4 == INK).count(), 1);
        let bs = earnings_bars(&svg);
        let ink = bs.iter().find(|b| b.4 == INK).unwrap();
        assert!(bs.iter().all(|b| b.0 <= ink.0), "{bs:?}");
        assert!(svg.contains(">$99.8B<") && svg.contains(">REVENUE, LAST 2 QUARTERS<"));
    }

    #[test]
    fn earnings_graphic_no_ink_when_the_release_is_not_charted() {
        // The release quarter unverified and the trend a quarter behind: the
        // last bar is an older quarter, so nothing is inked or labelled.
        let mut f = facts();
        f.fiscal_period = Field::Missing { reason: "x".into() };
        let svg = earnings_graphic(&f).unwrap();
        assert_eq!(rects(&svg).iter().filter(|r| r.4 == INK).count(), 0);
        assert!(!svg.contains(">$99.8B<"));
    }

    #[test]
    fn earnings_graphic_falls_back_to_diluted_eps() {
        let mut f = facts();
        f.metrics.revenue = Field::Missing { reason: "x".into() };
        if let Field::Value(t) = &mut f.trend {
            t.revenue.truncate(1);
        }
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        assert!(svg.contains(">DILUTED EPS, LAST 3 QUARTERS<") && svg.contains(">$1.85<"));
        assert!(!svg.contains("REVENUE"));
    }

    #[test]
    fn earnings_graphic_none_when_both_series_are_short() {
        let mut f = facts();
        f.trend = Field::Value(Trend::default());
        assert_eq!(earnings_graphic(&f), None);
        f.trend = Field::Missing { reason: "x".into() };
        assert_eq!(earnings_graphic(&f), None);
    }

    #[test]
    fn earnings_graphic_none_when_the_label_fails_the_check() {
        let f = facts();
        assert_eq!(earnings_graphic_checked(&f, &[]), None);
        let checked = earnings_graphic_checked(&f, &numbers_in_facts(&f));
        assert!(checked.is_some());
        assert_eq!(checked, earnings_graphic(&f));
    }

    #[test]
    fn earnings_graphic_keeps_the_last_eight_slots_and_starts_on_a_bar() {
        let mut f = facts();
        if let Field::Value(t) = &mut f.trend {
            // A gap after 2024-09, then six quarters, then the release: 9 slots.
            t.revenue = [
                "2024-09-28",
                "2025-03-29",
                "2025-06-28",
                "2025-09-27",
                "2025-12-27",
                "2026-03-28",
                "2026-06-27",
            ]
            .iter()
            .map(|d| qp(d, 100e9))
            .collect();
        }
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        let bs = earnings_bars(&svg);
        assert_eq!(bs.len(), 7);
        assert!(svg.contains(">REVENUE, LAST 7 QUARTERS<"));
        assert!(svg.contains(">2025-03<") && !svg.contains(">2024-09<"));
        assert!(!svg.contains("8-K"));
        // The period label sits under the leftmost bar.
        let first = bs.iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
        let x = text_x(&svg, "2025-03");
        assert!(
            (x - (first.0 + first.2 / 2.0)).abs() <= 0.1,
            "{x} {first:?}"
        );
    }

    #[test]
    fn earnings_graphic_draws_negative_eps_below_the_baseline() {
        let mut f = facts();
        f.metrics.revenue = Field::Missing { reason: "x".into() };
        if let Field::Value(t) = &mut f.trend {
            t.revenue.clear();
            t.eps_diluted = vec![qp("2026-03-28", -0.40), qp("2026-06-27", -0.31)];
        }
        f.metrics.eps_gaap = fig(
            -0.25,
            Unit::UsdPerShare,
            "$(0.25)",
            "diluted loss per share of $(0.25)",
        );
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        let bs = earnings_bars(&svg);
        assert_eq!(bs.len(), 3);
        // All three hang from one baseline; the deepest loss is the longest.
        assert!(bs.iter().all(|b| (b.1 - bs[0].1).abs() <= 0.1), "{bs:?}");
        let mut by_x = bs.clone();
        by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(by_x[0].3 > by_x[1].3 && by_x[1].3 > by_x[2].3, "{by_x:?}");
        assert_eq!(by_x[2].4, INK);
        assert!(svg.contains(">−$0.25<") && svg.contains(">DILUTED EPS, LAST 3 QUARTERS<"));
        // The value label is under the hanging ink bar.
        let value = texts(&svg).into_iter().find(|t| t.2 == "−$0.25").unwrap();
        assert!(value.1 > by_x[2].1 + by_x[2].3, "{value:?}");
    }

    #[test]
    fn earnings_graphic_without_a_release_has_no_ink_bar() {
        // A full-year-only release: no verified quarter, so no 8-K bar in either series.
        let mut f = facts();
        if let Field::Value(p) = &mut f.fiscal_period {
            p.quarter = None;
        }
        assert!(series(&f).iter().all(|s| s.release.is_none()));
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        let bs = earnings_bars(&svg);
        assert_eq!(bs.len(), 2, "{bs:?}");
        assert!(bs.iter().all(|b| b.4 == GREY), "{bs:?}");
        assert!(rects(&svg).iter().all(|r| r.4 != INK), "{svg}");
        assert!(texts(&svg).iter().all(|t| !t.2.contains('$')), "{svg}");
        assert!(svg.contains(">2026-03<") && svg.contains(">REVENUE, LAST 2 QUARTERS<"));
    }

    fn bar(label: &str, value: f64) -> Option<Bar> {
        Some(Bar {
            label: label.into(),
            value,
        })
    }

    #[test]
    fn earnings_graphic_falls_back_to_eps_when_the_revenue_window_is_short() {
        // Revenue has two bars, but only one in its last eight slots. `bars()` caps a gap at three
        // slots, so this is built by hand.
        let mut revenue = vec![bar("2024-03", 90e9)];
        revenue.extend(std::iter::repeat_with(|| None).take(7));
        revenue.push(bar("2026-06", 100e9));
        let all = [
            Series {
                name: "revenue",
                slots: revenue,
                release: None,
                latest_is_release: false,
            },
            Series {
                name: "eps",
                slots: vec![bar("2026-03", 1.5), bar("8-K", 1.85)],
                release: Some(1.85),
                latest_is_release: true,
            },
        ];
        let svg = earnings_svg(&all, &[1.85]).unwrap();
        assert_in_frame(&svg);
        assert!(svg.contains(">DILUTED EPS, LAST 2 QUARTERS<") && svg.contains(">$1.85<"));
        assert!(!svg.contains("REVENUE"));
        // With no EPS series to fall back on, the short revenue window draws nothing.
        assert_eq!(earnings_svg(&all[..1], &[]), None);
    }

    #[test]
    fn earnings_graphic_caption_counts_slots_including_gaps() {
        let mut f = facts();
        if let Field::Value(t) = &mut f.trend {
            // 2026-03 is missing: slots 2025-12, gap, 2026-06, release.
            t.revenue = vec![qp("2025-12-27", 95e9), qp("2026-06-27", 100e9)];
        }
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        assert_eq!(earnings_bars(&svg).len(), 3);
        assert!(svg.contains(">REVENUE, LAST 4 QUARTERS<"), "{svg}");
    }

    #[test]
    fn earnings_graphic_with_an_all_zero_series_has_no_nan() {
        let mut f = facts();
        f.metrics.revenue = Field::Missing { reason: "x".into() };
        if let Field::Value(t) = &mut f.trend {
            t.revenue.clear();
            t.eps_diluted = vec![qp("2026-03-28", 0.0), qp("2026-06-27", 0.0)];
        }
        f.metrics.eps_gaap = fig(
            0.0,
            Unit::UsdPerShare,
            "$0.00",
            "diluted earnings per share of $0.00",
        );
        let svg = earnings_graphic(&f).unwrap();
        assert_in_frame(&svg);
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
        assert!(svg.contains(">$0.00<"));
        assert_eq!(earnings_bars(&svg).len(), 3);
    }
}
