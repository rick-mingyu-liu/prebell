//! The share card of a post: its SVG cover recomposed at 1200×630 under the
//! PreBell nameplate and rasterised to PNG, because WeChat, X and Facebook
//! do not show SVG previews. Cards carry no headline (the platforms print
//! the page's title beside them), so one card serves both languages and
//! only Latin fonts are embedded.
//!
//! Rendering costs about 5 ms of CPU in wasm, half the Free plan's 10 ms per
//! invocation, so it never shares an invocation with a build: the `cards`
//! job renders at most one missing card per queue message.

use async_trait::async_trait;
use resvg::{tiny_skia, usvg};

use super::graphic::escape;

pub const WIDTH: u32 = 1200;
pub const HEIGHT: u32 = 630;

/// Latest posts the `cards` job looks at for a missing card (one R2 head each).
pub const SCAN: usize = 30;

const PAPER: &str = "#faf8f3";
const INK: &str = "#1a1a1a";
const CAPTION: &str = "#666";
const AMBER: &str = "#e8a317";
/// The cover's 640-unit width scaled to the card's.
const SCALE: f64 = 1200.0 / 640.0;

/// Static cuts of the site's Inter and Newsreader, printable ASCII only
/// (`scripts/build-card-fonts.py`; OFL, licences beside them).
const FONTS: [&[u8]; 4] = [
    include_bytes!("../../data/fonts/inter-500.ttf"),
    include_bytes!("../../data/fonts/inter-600.ttf"),
    include_bytes!("../../data/fonts/inter-700.ttf"),
    include_bytes!("../../data/fonts/newsreader-700.ttf"),
];

/// The root element the cover graphic opens with, and its paper background.
const COVER_ROOT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">"#;
const COVER_PAPER: &str = r##"<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>"##;

/// The R2 key of a cover's card: `covers/<stem>.svg` -> `og/<stem>.png`.
/// Raster covers (older scene images) get no card.
pub fn card_key(cover_key: &str) -> Option<String> {
    let stem = cover_key.strip_prefix("covers/")?.strip_suffix(".svg")?;
    let ok = !stem.is_empty()
        && stem.len() <= 80
        && stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then(|| format!("og/{stem}.png"))
}

/// The card's nameplate: the bell mark and "PreBell" in the serif left, the
/// kicker (ticker and date, both language-neutral) right, a double rule.
fn open_card(kicker: &str) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}" width="{WIDTH}" height="{HEIGHT}" font-family="Inter">"#
    );
    out.push_str(&format!(
        r#"<rect x="0" y="0" width="{WIDTH}" height="{HEIGHT}" fill="{PAPER}"/>"#
    ));
    // The masthead's bell (viewBox 12 8 40 50) at 1.5×.
    out.push_str(&format!(
        r#"<g transform="translate(30 36) scale(1.5)"><path d="M32 12c-1.9 0-3.4 1.5-3.4 3.4v1.2C22.7 18.2 19 23.7 19 30.2v7.6l-4.2 6.1c-.7 1 0 2.4 1.2 2.4h32c1.2 0 1.9-1.4 1.2-2.4L45 37.8v-7.6c0-6.5-3.7-12-9.6-13.6v-1.2c0-1.9-1.5-3.4-3.4-3.4z" fill="none" stroke="{INK}" stroke-width="3.4" stroke-linejoin="round"/><circle cx="32" cy="52.5" r="3.6" fill="{AMBER}"/><path d="M47 15.5l4.5-4.5M17 15.5l-4.5-4.5" stroke="{INK}" stroke-width="2.6" stroke-linecap="round"/></g>"#
    ));
    out.push_str(&format!(
        r#"<text x="118" y="104" font-family="Newsreader" font-size="64" font-weight="700" letter-spacing="-1.3" fill="{INK}">PreBell</text>"#
    ));
    if !kicker.is_empty() {
        out.push_str(&format!(
            r#"<text x="1152" y="100" font-size="26" font-weight="600" letter-spacing="2" fill="{CAPTION}" text-anchor="end">{}</text>"#,
            escape(kicker)
        ));
    }
    out.push_str(&format!(
        r#"<rect x="48" y="132" width="1104" height="3" fill="{INK}"/><rect x="48" y="140" width="1104" height="1" fill="{INK}"/>"#
    ));
    out
}

/// The card for a cover graphic: its chart and caption scaled to the card's
/// width, bottom-aligned (the cover's empty upper part, kept for the page's
/// HTML lines, falls under the nameplate). `None` when `cover_svg` is not a
/// graphic `graphic.rs` drew.
pub fn card_svg(cover_svg: &str, kicker: &str) -> Option<String> {
    let body = cover_svg
        .trim_end()
        .strip_prefix(COVER_ROOT)?
        .strip_suffix("</svg>")?;
    let body = body.strip_prefix(COVER_PAPER).unwrap_or(body);
    let mut out = open_card(kicker);
    let dy = f64::from(HEIGHT) - 360.0 * SCALE;
    out.push_str(&format!(
        r#"<g transform="translate(0 {dy}) scale({SCALE})">{body}</g></svg>"#
    ));
    Some(out)
}

/// The card for pages without a post: the nameplate over the site's
/// address, with an amber bar like the covers'.
pub fn default_card_svg() -> String {
    let mut out = open_card("");
    out.push_str(&format!(
        r#"<rect x="48" y="452" width="96" height="8" fill="{AMBER}"/><text x="48" y="530" font-size="44" font-weight="700" fill="{INK}">prebell.site</text><text x="48" y="590" font-size="22" font-weight="600" letter-spacing="2.5" fill="{CAPTION}">PREMARKET BRIEFS · RECAPS · EARNINGS CHECKS</text></svg>"#
    ));
    out
}

/// Rasterises a card SVG to an opaque RGB PNG with the embedded fonts only.
pub fn render_png(svg: &str) -> Result<Vec<u8>, String> {
    let mut opt = usvg::Options {
        font_family: "Inter".into(),
        ..Default::default()
    };
    let db = opt.fontdb_mut();
    for f in FONTS {
        db.load_font_data(f.to_vec());
    }
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| format!("card svg: {e}"))?;
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT).ok_or("card pixmap")?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    // The paper background is opaque, so premultiplied RGBA is plain RGBA.
    let rgb: Vec<u8> = pixmap
        .data()
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut png = Vec::new();
    let mut enc = png::Encoder::new(&mut png, WIDTH, HEIGHT);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut w = enc.write_header().map_err(|e| format!("card png: {e}"))?;
    w.write_image_data(&rgb)
        .map_err(|e| format!("card png: {e}"))?;
    w.finish().map_err(|e| format!("card png: {e}"))?;
    Ok(png)
}

/// A published post with an SVG cover, as the `cards` job sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverPost {
    pub cover_key: String,
    pub trading_date: String,
    /// The earnings filing's ticker; `None` for briefs.
    pub ticker: Option<String>,
}

/// What the `cards` job reads and writes (D1 and R2 in production).
#[async_trait(?Send)]
pub trait CardStore {
    /// The newest `limit` published posts whose cover is an SVG, newest
    /// (first publish or update) first.
    async fn cover_posts(&self, limit: usize) -> Result<Vec<CoverPost>, String>;
    async fn has_object(&self, key: &str) -> Result<bool, String>;
    async fn get_text(&self, key: &str) -> Result<Option<String>, String>;
    async fn put_png(&self, key: &str, bytes: &[u8]) -> Result<(), String>;
}

/// "AAPL · 2026-11-05", or the date alone for a brief.
fn kicker(p: &CoverPost) -> String {
    match &p.ticker {
        Some(t) => format!("{t} · {}", p.trading_date),
        None => p.trading_date.clone(),
    }
}

/// One `cards` job: renders the newest post's missing card, at most one per
/// invocation (CPU). A cover that is gone or not a graphic is skipped without
/// rendering, so it never holds up the posts behind it. `Ok` is a log line.
pub async fn cards_tick(store: &dyn CardStore) -> Result<String, String> {
    for p in store.cover_posts(SCAN).await? {
        let Some(key) = card_key(&p.cover_key) else {
            continue;
        };
        if store.has_object(&key).await? {
            continue;
        }
        let Some(cover) = store.get_text(&p.cover_key).await? else {
            continue;
        };
        let Some(svg) = card_svg(&cover, &kicker(&p)) else {
            continue;
        };
        let png = render_png(&svg)?;
        store.put_png(&key, &png).await?;
        return Ok(format!("card {key} ({} bytes)", png.len()));
    }
    Ok("no card missing".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::graphic::{brief_graphic, CoverMover, Session};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn cover() -> String {
        let movers = [("AAPL", 6.12), ("AMD", 4.05), ("PYPL", -5.3)].map(|(s, p)| CoverMover {
            symbol: s.into(),
            change_pct: p,
        });
        brief_graphic(&movers, Session::Premarket).unwrap()
    }

    #[test]
    fn card_key_maps_svg_covers_only() {
        assert_eq!(
            card_key("covers/premarket-2026-11-05.svg").as_deref(),
            Some("og/premarket-2026-11-05.png")
        );
        assert_eq!(
            card_key("covers/premarket-2026-11-05-update-1.svg").as_deref(),
            Some("og/premarket-2026-11-05-update-1.png")
        );
        assert_eq!(card_key("covers/recap-2026-10-30.png"), None);
        assert_eq!(card_key("covers/../x.svg"), None);
        assert_eq!(card_key("charts/x.svg"), None);
        assert_eq!(card_key("covers/.svg"), None);
    }

    #[test]
    fn card_svg_keeps_the_chart_under_the_nameplate() {
        let svg = card_svg(&cover(), "2026-11-05").unwrap();
        assert!(
            svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630""#)
        );
        assert!(svg.ends_with("</g></svg>"));
        assert!(svg.contains(">PreBell</text>"));
        assert!(svg.contains(">2026-11-05</text>"));
        assert!(svg.contains(">AAPL</text>"));
        assert!(svg.contains(">BIGGEST PREMARKET MOVES</text>"));
        // The cover's own root and background are dropped; its content is scaled once.
        assert_eq!(svg.matches("<svg").count(), 1);
        assert!(!svg.contains(COVER_PAPER));
        assert!(svg.contains(r#"<g transform="translate(0 -45) scale(1.875)">"#));
        assert_eq!(card_svg("<svg/>", ""), None);
    }

    #[test]
    fn card_svg_takes_earnings_covers_too() {
        let cover =
            crate::cover::earnings_graphic(&crate::earnings::facts::tests::facts()).unwrap();
        let svg = card_svg(&cover, "AAPL · 2026-10-30").unwrap();
        assert!(svg.contains(">AAPL · 2026-10-30</text>"));
        assert!(render_png(&svg).is_ok());
    }

    #[test]
    fn kicker_escapes_and_names_the_ticker() {
        let p = CoverPost {
            cover_key: "covers/x.svg".into(),
            trading_date: "2026-11-05".into(),
            ticker: Some("BRK.B".into()),
        };
        assert_eq!(kicker(&p), "BRK.B · 2026-11-05");
        assert!(card_svg(&cover(), "A<B")
            .unwrap()
            .contains(">A&lt;B</text>"));
    }

    #[test]
    fn render_png_is_a_1200_by_630_opaque_png() {
        for svg in [
            card_svg(&cover(), "2026-11-05").unwrap(),
            default_card_svg(),
        ] {
            let png = render_png(&svg).unwrap();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            let dec = png::Decoder::new(std::io::Cursor::new(&png));
            let reader = dec.read_info().unwrap();
            let info = reader.info();
            assert_eq!((info.width, info.height), (WIDTH, HEIGHT));
            assert_eq!(info.color_type, png::ColorType::Rgb);
            assert!(png.len() < 200_000, "{} bytes", png.len());
        }
    }

    #[test]
    fn render_png_draws_text_with_the_embedded_fonts() {
        // The same card with and without its text must differ: a missing font draws nothing.
        let with = render_png(&default_card_svg()).unwrap();
        let without = render_png(&default_card_svg().replace("prebell.site", "")).unwrap();
        assert_ne!(with, without);
    }

    #[derive(Default)]
    struct Mem {
        posts: Vec<CoverPost>,
        objects: RefCell<BTreeMap<String, Vec<u8>>>,
    }

    #[async_trait(?Send)]
    impl CardStore for Mem {
        async fn cover_posts(&self, limit: usize) -> Result<Vec<CoverPost>, String> {
            Ok(self.posts.iter().take(limit).cloned().collect())
        }
        async fn has_object(&self, key: &str) -> Result<bool, String> {
            Ok(self.objects.borrow().contains_key(key))
        }
        async fn get_text(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self
                .objects
                .borrow()
                .get(key)
                .map(|b| String::from_utf8_lossy(b).into_owned()))
        }
        async fn put_png(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
            self.objects.borrow_mut().insert(key.into(), bytes.to_vec());
            Ok(())
        }
    }

    fn post(stem: &str) -> CoverPost {
        CoverPost {
            cover_key: format!("covers/{stem}.svg"),
            trading_date: "2026-11-05".into(),
            ticker: None,
        }
    }

    #[tokio::test]
    async fn cards_tick_renders_one_missing_card_per_run_and_skips_broken_covers() {
        let s = Mem {
            posts: vec![
                post("gone"),
                post("broken"),
                post("new"),
                post("old"),
                post("done"),
            ],
            ..Default::default()
        };
        {
            let mut o = s.objects.borrow_mut();
            o.insert("covers/broken.svg".into(), b"<svg/>".to_vec());
            for stem in ["new", "old", "done"] {
                o.insert(format!("covers/{stem}.svg"), cover().into_bytes());
            }
            o.insert("og/done.png".into(), b"png".to_vec());
        }
        assert_eq!(
            cards_tick(&s).await.unwrap().split(" (").next(),
            Some("card og/new.png")
        );
        assert!(!s.objects.borrow().contains_key("og/old.png"));
        assert_eq!(
            cards_tick(&s).await.unwrap().split(" (").next(),
            Some("card og/old.png")
        );
        assert_eq!(cards_tick(&s).await.unwrap(), "no card missing");
        assert!(s.objects.borrow()["og/new.png"].starts_with(b"\x89PNG"));
        assert_eq!(s.objects.borrow()["og/done.png"], b"png");
    }
}
