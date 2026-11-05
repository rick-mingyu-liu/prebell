//! `cover-graphic <content_type> <facts.json>`: prints a post's data-graphic
//! cover as SVG. Exit 3 means the facts have no graphic. Used by
//! `scripts/backfill-graphics.sh`.

use prebell_core::cover::graphic_for_facts;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [kind, path] = args.as_slice() else {
        eprintln!("usage: cover-graphic <content_type> <facts.json>");
        return ExitCode::from(2);
    };
    let out = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {path}: {e}"))
        .and_then(|json| graphic_for_facts(kind, &json));
    match out {
        Ok(Some(svg)) => {
            println!("{svg}");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            eprintln!("no graphic for these facts");
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
