//! `card-png default` or `card-png <cover.svg> <kicker>`: writes a share
//! card PNG to stdout. The default card is committed as
//! `apps/web/public/og-default.png`; the second form previews a post's card.

use prebell_core::cover::card::{card_svg, default_card_svg, render_png};
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let svg = match args.as_slice() {
        [d] if d == "default" => Ok(default_card_svg()),
        [path, kicker] => std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {path}: {e}"))
            .and_then(|s| {
                card_svg(&s, kicker).ok_or_else(|| format!("{path}: not a cover graphic"))
            }),
        _ => {
            eprintln!("usage: card-png default | card-png <cover.svg> <kicker>");
            return ExitCode::from(2);
        }
    };
    match svg.and_then(|s| render_png(&s)) {
        Ok(png) => {
            std::io::stdout().write_all(&png).expect("stdout");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
