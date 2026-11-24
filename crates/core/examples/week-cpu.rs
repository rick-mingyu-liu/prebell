//! The week post's Wasm CPU probe (not shipped): exports that render and
//! check the peak-week bundle (60 checks, 200 companies), for timing under
//! node with `--cpu-prof`. Build: `cargo build -p prebell-core --release
//! --example week-cpu --target wasm32-unknown-unknown`.

use prebell_core::week::{job::render_both, Bundle};

static BUNDLE: &str = include_str!("../tests/fixtures/week/2027-01-08.bundle.json");

/// Parses the bundle only; returns the number of listed checks.
#[no_mangle]
pub extern "C" fn week_parse() -> u32 {
    let b: Bundle = serde_json::from_str(BUNDLE).expect("bundle");
    b.listed.len() as u32
}

/// Parses, renders and checks both languages; returns the pages' bytes.
#[no_mangle]
pub extern "C" fn week_render() -> u32 {
    let b: Bundle = serde_json::from_str(BUNDLE).expect("bundle");
    let [en, zh] = render_both(&b).expect("pages pass the guard");
    (en.2.len() + zh.2.len()) as u32
}
