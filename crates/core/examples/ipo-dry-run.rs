//! `ipo-dry-run`: one live model test of the IPO write-up, run by hand before
//! the PR (Rick, 2026-11-11: "Yes, run it (Recommended)", 2 to 4 calls under
//! $0.10, nothing published).
//!
//! It drafts Anthropic's `range_set` write-up from the real facts bundle (the
//! S-1/A of 2026-11-10; `db/preview-facts/ipo/...` with its made-up news items
//! removed, which a core test keeps equal to the bundle built from the stored
//! state), through the generator's own prompt, client and checks
//! (`ipo::writeup::write`: Claude, `LLM_EFFORT` low, one retry with the
//! rejection reason). English first, then Chinese only if English passed, as
//! the build does.
//!
//! It prints each raw model reply with its token usage, each check verdict
//! with its reasons, and the rendered page of each language that passes. It
//! writes to no store, makes no request but to the Anthropic API (through
//! `curl`, with the key passed on curl's stdin, never in its arguments), stops
//! at 4 model calls, never prints the key, and exits 1 if a language is
//! refused after its retry (2 on a setup error).
//!
//! Run from the worktree: `ANTHROPIC_API_KEY=... cargo run -p prebell-core
//! --example ipo-dry-run --locked`. `LLM_MODEL` overrides the model as in
//! production (unset there).

use std::cell::Cell;
use std::io::Write;
use std::process::{Command, ExitCode, Stdio};

use async_trait::async_trait;
use serde_json::Value;

use prebell_core::claude::DEFAULT_MODEL;
use prebell_core::ipo::milestone::IpoArticleFacts;
use prebell_core::ipo::writeup::{accept, write};
use prebell_core::llm::llm_from_env;
use prebell_core::numbers::Lang;
use prebell_core::providers::{Http, HttpResp, LlmProvider, ProviderError};

const FACTS: &str =
    include_str!("../../../db/preview-facts/ipo/anthropic/range_set-0001628280-26-072484.json");
const MAX_CALLS: usize = 4;
const API: &str = "https://api.anthropic.com/";

/// `Http` over the `curl` command, for the Anthropic API only.
struct Curl {
    calls: Cell<usize>,
}

/// A value for curl's config syntax: quoted, with `\` and `"` escaped.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn net(e: impl std::fmt::Display) -> ProviderError {
    ProviderError::Network(e.to_string())
}

#[async_trait(?Send)]
impl Http for Curl {
    async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        Err(net(format!("dry run: no GET requests ({url})")))
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResp, ProviderError> {
        if !url.starts_with(API) {
            return Err(net("dry run: only the Anthropic API is called"));
        }
        let n = self.calls.get() + 1;
        if n > MAX_CALLS {
            return Err(net(format!(
                "dry run: the cap of {MAX_CALLS} model calls is reached"
            )));
        }
        self.calls.set(n);
        // The request body (prompt and facts, no key) goes through a temp
        // file; the headers, key included, through curl's stdin.
        let path = std::env::temp_dir().join(format!(
            "prebell-ipo-dry-run-{}-{n}.json",
            std::process::id()
        ));
        std::fs::write(&path, serde_json::to_vec(body).map_err(net)?).map_err(net)?;
        let mut cfg = format!(
            "url = {}\nrequest = \"POST\"\nsilent\nshow-error\nmax-time = 600\nwrite-out = \"\\n%{{http_code}}\"\ndata-binary = {}\n",
            quoted(url),
            quoted(&format!("@{}", path.display()))
        );
        for (k, v) in headers {
            cfg.push_str(&format!("header = {}\n", quoted(&format!("{k}: {v}"))));
        }
        let run = || -> Result<std::process::Output, ProviderError> {
            let mut child = Command::new("curl")
                .args(["-K", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(net)?;
            child
                .stdin
                .take()
                .ok_or_else(|| net("no stdin"))?
                .write_all(cfg.as_bytes())
                .map_err(net)?;
            child.wait_with_output().map_err(net)
        };
        let out = run();
        let _ = std::fs::remove_file(&path);
        let out = out?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (resp, status) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
        let status: u16 = status.trim().parse().unwrap_or(0);
        println!("\n--- model call {n} of at most {MAX_CALLS}: HTTP {status}");
        if status == 0 {
            let err = String::from_utf8_lossy(&out.stderr);
            println!("curl failed: {}", err.trim());
            return Err(net("curl failed"));
        }
        let v: Value = serde_json::from_str(resp).unwrap_or(Value::Null);
        if let Some(u) = v.get("usage") {
            println!("usage: {u}");
        }
        if status >= 400 {
            // The error's type and request id only, as the generator logs it.
            println!(
                "error.type={} request_id={}",
                v.pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                v.get("request_id").and_then(Value::as_str).unwrap_or("?")
            );
        } else {
            println!(
                "stop_reason: {}",
                v.get("stop_reason").and_then(Value::as_str).unwrap_or("?")
            );
            let raw = v
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            println!("raw reply:\n{raw}");
        }
        Ok(HttpResp {
            status,
            body: resp.to_string(),
            set_cookie: None,
        })
    }
}

/// The client, with each reply checked as the build checks it, printed.
struct Verdicts<'a> {
    inner: &'a dyn LlmProvider,
    facts: &'a IpoArticleFacts,
    lang: Lang,
    attempt: Cell<usize>,
}

#[async_trait(?Send)]
impl LlmProvider for Verdicts<'_> {
    async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
        let n = self.attempt.get() + 1;
        self.attempt.set(n);
        println!("\n=== {:?} attempt {n}", self.lang);
        if let Some((_, note)) = user.split_once("\n\nYour previous reply had problems.") {
            println!("retry note sent: Your previous reply had problems.{note}");
        }
        let r = self.inner.complete_json(system, user).await;
        match &r {
            Ok(v) => match accept(v, self.facts, self.lang) {
                Ok(_) => println!("verdict: PASS"),
                Err(e) => println!("verdict: REFUSED\n- {}", e.join("\n- ")),
            },
            Err(e) => println!("verdict: REFUSED (model call failed: {e})"),
        }
        r
    }
}

fn warn(m: &str) {
    eprintln!("{m}");
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let Some(key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
    else {
        eprintln!("set ANTHROPIC_API_KEY");
        return ExitCode::from(2);
    };
    let mut facts: IpoArticleFacts = match serde_json::from_str(FACTS) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("facts: {e}");
            return ExitCode::from(2);
        }
    };
    // The real first run has no news items stored.
    facts.items.clear();
    let model = std::env::var("LLM_MODEL").ok().filter(|m| !m.is_empty());
    println!(
        "IPO write-up dry run: {} ({}), model {}, effort low, at most {MAX_CALLS} calls, nothing written",
        facts.run_key,
        facts.as_of,
        model.as_deref().unwrap_or(DEFAULT_MODEL)
    );
    let http = Curl {
        calls: Cell::new(0),
    };
    let llm = match llm_from_env("claude", http, key, model, Some("low".into()), warn) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("llm: {e}");
            return ExitCode::from(2);
        }
    };
    let mut ok = true;
    for lang in [Lang::En, Lang::Zh] {
        let v = Verdicts {
            inner: llm.as_ref(),
            facts: &facts,
            lang,
            attempt: Cell::new(0),
        };
        match write(&v, &facts, lang).await {
            Ok((_, md)) => println!("\n=== {lang:?} PASSED; the rendered page:\n\n{md}"),
            Err(e) => {
                println!(
                    "\n=== {lang:?} REFUSED after the retry:\n- {}",
                    e.join("\n- ")
                );
                ok = false;
                // As the build does: no Chinese call after English failed.
                break;
            }
        }
    }
    if ok {
        println!("\nBoth languages passed.");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
