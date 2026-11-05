//! `cover-lines <content_type> <facts.json>`: prints a post's cover lines as
//! `{"en":[l1,l2],"zh":[l1,l2]}`. Used by `scripts/backfill-covers.sh`.
//! `cover-lines --prompt <content_type> <facts.json>`: prints the scene image
//! prompt the generator would send. Used by `scripts/backfill-scenes.sh`.

use prebell_core::cover::{lines_for_facts, scene_prompt_for_facts};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (prompt, kind, path) = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [kind, path] => (false, kind, path),
        ["--prompt", kind, path] => (true, kind, path),
        _ => {
            eprintln!("usage: cover-lines [--prompt] <content_type> <facts.json>");
            return ExitCode::from(2);
        }
    };
    let out = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {path}: {e}"))
        .and_then(|json| {
            if prompt {
                scene_prompt_for_facts(kind, &json)
            } else {
                lines_for_facts(kind, &json)
                    .map(|l| serde_json::json!({ "en": l.en, "zh": l.zh }).to_string())
            }
        });
    match out {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
