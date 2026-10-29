# Plan 5 — Claude API provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The owner can switch the LLM from DeepSeek to Claude by config alone: `LLM_PROVIDER=claude` plus the secret `ANTHROPIC_API_KEY` makes every LLM call (reason extraction and EN/ZH drafting) go to the Anthropic Messages API. DeepSeek stays the default and keeps working unchanged.

**Architecture:** A new `core::claude` module holds `Claude<H: Http>`, a second `LlmProvider` implementation next to `DeepSeek<H>`. Provider selection moves from `deepseek.rs` into a small new `core::llm` module (`llm_from_env`, `api_key_var`, shared `strip_fence`), so neither provider module knows about the other. The generator Worker asks `core` which secret the chosen provider needs and passes an optional `LLM_MODEL` override. Nothing downstream of `LlmProvider` changes: facts, anchoring, drafting and `numbers::check` see the same `serde_json::Value`.

**Tech Stack:** as Plan 1. No new crates: the Messages API is called with raw HTTP through the existing `Http` trait (there is no official Anthropic Rust SDK, and the Worker already owns `fetch`).

**Spec:** `docs/spec.md` (AI section: "DeepSeek API first, switching to the Claude API afterwards. The provider must be swappable by config." and providers: "Config selects provider and model."). Plan 1: `docs/plan-1.md`.

## Global Constraints

- Everything in Plan 1's Global Constraints still holds. In particular: every number an article publishes must pass `numbers::check`; missing data is never guessed; never use AlphaLab content.
- Endpoint: `POST https://api.anthropic.com/v1/messages`. Headers: `x-api-key: <ANTHROPIC_API_KEY>`, `anthropic-version: 2023-06-01`, `content-type: application/json`.
- Request body: exactly `model`, `max_tokens`, `system`, `messages: [{"role":"user","content": <user>}]`. No `temperature` (Claude Sonnet 5.5 rejects non-default sampling values; API version `2026-10-21` rejects the field outright), no `thinking`, no beta headers, so the body stays valid for any configured model.
- `max_tokens = 16000`: current models think adaptively by default and thinking counts toward `max_tokens`; 16000 keeps a non-streaming request well inside HTTP timeouts while leaving room for a long ZH draft.
- Default model `claude-sonnet-5-5`; `LLM_MODEL` (Worker var, optional, empty = default) overrides the model for either provider.
- Response handling, in this order: HTTP status ≥ 400 → `ProviderError::Http(status)`; body not JSON → `Parse`; `stop_reason == "max_tokens"` → `Parse("truncated")`; `stop_reason == "refusal"` → `Parse("refusal: <category>")`; read the **first block with `type == "text"`** (responses can start with `thinking` blocks; read by type, never by position); no text block → `Empty`; strip a ```` ```json ```` fence; must parse to a JSON object, else `Parse`.
- Selection: `LLM_PROVIDER` ∈ {`deepseek` (default), `claude`}; unknown → error at queue start (as today). Secret per provider: `deepseek` → `DEEPSEEK_API_KEY`, `claude` → `ANTHROPIC_API_KEY`.
- Secrets only via `wrangler secret put` / `.dev.vars`. No real keys exist: tests use hand-written fixtures built from the documented response shape.
- Never deploy, never touch remote Cloudflare resources.

## Review Focus

1. **Thinking blocks before text.** A Sonnet 5.5 response may be `[{"type":"thinking",...},{"type":"text",...}]`. Taking `content[0].text` would silently return `Empty`. Test in Task 1.
2. **Truncation and refusal are errors, not partial JSON.** `stop_reason: "max_tokens"` with half an object, and `stop_reason: "refusal"` with an apology, must never reach the drafter as a value. Tests in Task 1.
3. **Header names and the API key.** The key goes in `x-api-key` (not `Authorization: Bearer`), and `anthropic-version` is present. Test in Task 1 asserts the exact headers.
4. **DeepSeek unchanged.** Moving `llm_from_env` must not change DeepSeek's request or parsing. Its existing tests keep passing untouched (except the dispatch test, which moves to `llm.rs`).
5. **Secret routing in the Worker.** With `LLM_PROVIDER=claude` the Worker must read `ANTHROPIC_API_KEY`, not `DEEPSEEK_API_KEY`. The mapping lives in `core` (`api_key_var`) so it is unit-tested; the Worker only calls it.

---

## File Structure

```
crates/core/src/
  lib.rs          # + pub mod claude; pub mod llm;
  llm.rs          # NEW: llm_from_env (moved from deepseek.rs), api_key_var, strip_fence (moved)
  claude.rs       # NEW: Claude<H> LlmProvider over the Messages API
  deepseek.rs     # llm_from_env + strip_fence removed (moved to llm.rs)
crates/core/tests/fixtures/
  claude_ok.json            # NEW: text block holding a JSON object
  claude_thinking_ok.json   # NEW: empty thinking block, then text block
  claude_bad_json.json      # NEW: text block holding prose
  claude_max_tokens.json    # NEW: stop_reason max_tokens, half an object
  claude_refusal.json       # NEW: stop_reason refusal with stop_details
crates/generator/src/lib.rs # use core::llm; secret by api_key_var; optional LLM_MODEL
crates/generator/wrangler.toml # comment documenting LLM_PROVIDER / LLM_MODEL values
docs/runbook.md             # NEW: switching the LLM provider
docs/plan-5.md              # this plan
```

---

### Task 1: Claude `LlmProvider` (`core::claude`)

**Files:** Create `crates/core/src/llm.rs` (only `strip_fence` in this task), `crates/core/src/claude.rs`, the five fixtures above; modify `crates/core/src/lib.rs`, `crates/core/src/deepseek.rs` (use `crate::llm::strip_fence`).

**Interfaces:**
- Consumes: `Http`, `HttpResp`, `LlmProvider`, `ProviderError` (Plan 1 Task 6).
- Produces:
  - `pub(crate) fn llm::strip_fence(s: &str) -> &str` (moved verbatim from `deepseek.rs`).
  - `pub const claude::DEFAULT_MODEL: &str = "claude-sonnet-5-5";`
  - `pub struct Claude<H> { pub http: H, pub api_key: String, pub model: String }`, `Claude::new(http, api_key, model)`.
  - `impl<H: Http> LlmProvider for Claude<H>` with the request and response rules in Global Constraints.

- [ ] **Step 1: Failing tests** (same `FakeHttp` pattern as `deepseek.rs`):
  - `ok_fixture_returns_object`: `claude_ok.json` → `v["headline"] == "Futures up"`.
  - `thinking_block_before_text_is_skipped`: `claude_thinking_ok.json` → object returned.
  - `non_json_text_is_parse_error`: `claude_bad_json.json` → `Parse(_)`.
  - `non_object_json_is_parse_error`: text `[1,2]` → `Parse(_)`.
  - `fenced_json_accepted`: ```` ```json\n{"a":1}\n``` ```` and ```` ```\n{"a":2}\n``` ````.
  - `max_tokens_is_truncated_parse_error`: `claude_max_tokens.json` → `Err(Parse("truncated"))`.
  - `refusal_is_parse_error`: `claude_refusal.json` → `Err(Parse("refusal: cyber"))`.
  - `no_text_block_and_http_error`: `{"content":[],"stop_reason":"end_turn"}` → `Empty`; status 529 → `Http(529)`; status 401 → `Http(401)`.
  - `request_shape_and_headers`: URL `https://api.anthropic.com/v1/messages`; headers include `("x-api-key","ak")`, `("anthropic-version","2023-06-01")`, `("content-type","application/json")` and no `Authorization`; body `model == "claude-sonnet-5-5"`, `max_tokens == 16000`, `system == "sys"`, `messages == [{"role":"user","content":"usr"}]`, and the body has no `temperature` key.
- [ ] **Step 2:** `cargo test -p prebell-core claude` → FAIL (module missing).
- [ ] **Step 3:** Implement. Move `strip_fence` to `llm.rs`; DeepSeek imports it.
- [ ] **Step 4:** `cargo test -p prebell-core` → PASS (DeepSeek tests unchanged).
- [ ] **Step 5: Commit** `feat(core): Claude Messages API LLM provider`.

---

### Task 2: Provider selection by config (`core::llm` + generator)

**Files:** Modify `crates/core/src/llm.rs`, `crates/core/src/deepseek.rs` (remove `llm_from_env` and its test), `crates/generator/src/lib.rs`, `crates/generator/wrangler.toml`.

**Interfaces:**
- Consumes: `DeepSeek`, `Claude`, `claude::DEFAULT_MODEL`.
- Produces:
  - `pub fn llm::api_key_var(provider: &str) -> Option<&'static str>` — `"deepseek"` → `Some("DEEPSEEK_API_KEY")`, `"claude"` → `Some("ANTHROPIC_API_KEY")`, else `None`.
  - `pub fn llm::llm_from_env<H: Http + 'static>(provider: &str, http: H, api_key: String, model: Option<String>) -> Result<Box<dyn LlmProvider>, ProviderError>` — `"deepseek"` → `DeepSeek` with `model` or `"deepseek-chat"`; `"claude"` → `Claude` with `model` or `DEFAULT_MODEL`; unknown → `Parse("unknown LLM provider: <name>")`. `Some("")` counts as unset.
  - Generator: `provider = LLM_PROVIDER or "deepseek"`; `secret = api_key_var(&provider)` (unknown provider → the queue handler errors with the same message as `llm_from_env`); `model = text(&env, "LLM_MODEL")`.

- [ ] **Step 1: Failing tests** in `llm.rs` (FakeHttp records URL and body):
  - `api_key_var_per_provider`: the three cases above.
  - `dispatch_deepseek_default_model`: `llm_from_env("deepseek", h, "dk", None)` → call hits `api.deepseek.com`, body model `deepseek-chat`.
  - `dispatch_claude_default_model`: `llm_from_env("claude", h, "ak", None)` with a Claude fixture → hits `api.anthropic.com/v1/messages`, body model `claude-sonnet-5-5`, header `x-api-key: ak`.
  - `model_override`: `Some("claude-opus-5-5")` → body model `claude-opus-5-5`; `Some("")` → default.
  - `unknown_provider_errors`: `"nope"` → `Parse(_)`.
- [ ] **Step 2:** `cargo test -p prebell-core llm` → FAIL.
- [ ] **Step 3:** Implement; delete `llm_from_env` from `deepseek.rs`; update the generator import and secret lookup; add a comment in `wrangler.toml` `[vars]` listing `LLM_PROVIDER = "deepseek" | "claude"` and optional `LLM_MODEL`. `LLM_PROVIDER` stays `"deepseek"`.
- [ ] **Step 4:** `cargo test -p prebell-core` PASS; `cargo build -p prebell-core --target wasm32-unknown-unknown`; `cd crates/generator && worker-build --release` builds.
- [ ] **Step 5: Commit** `feat: select Claude or DeepSeek by LLM_PROVIDER`.

---

### Task 3: Runbook — switching providers

**Files:** Create `docs/runbook.md` (Plan 1 Task 13 will add deploy, cron and rerun sections to the same file).

- [ ] **Step 1:** Write a "LLM provider" section: the two providers and their secrets; switching to Claude (`wrangler secret put ANTHROPIC_API_KEY`, set `LLM_PROVIDER = "claude"` in `wrangler.toml` `[vars]`, redeploy — owner only); optional `LLM_MODEL`; switching back; local dev with `.dev.vars`; what failures look like (HTTP 401/429/529, `truncated`, `refusal`) and that the run state machine retries them until 9:25 ET like any provider error.
- [ ] **Step 2:** Re-read for accuracy against `llm.rs` and the generator.
- [ ] **Step 3: Commit** `docs: runbook for switching the LLM provider`.

---

### Task 4: Whole-branch review and verification

- [ ] **Step 1:** Review the full diff `feat/plan-1...HEAD` against this plan and the spec; fix findings.
- [ ] **Step 2: Verify, all must pass:** `cargo test -p prebell-core`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo build -p prebell-core --target wasm32-unknown-unknown`; `worker-build --release` in `crates/generator`; `pnpm --filter web test`, `typecheck`, `lint`; `cargo fmt --all --check`.
- [ ] **Step 3:** Push `feat/plan-5-claude`; open PR into `feat/plan-1` titled `Plan 5: Claude API provider`.

---

## Parallelization (for subagent execution)

Sequential: Task 1 → Task 2 (needs `Claude`) → Task 3 (documents Task 2's config) → Task 4.

## Out of scope (possible follow-ups, owner's call)

- `output_config.effort` tuning, structured outputs (`output_config.format`) per prompt, prompt caching of the system prompt, streaming.
- Server-side refusal fallbacks (beta). For this content the relevant categories are unlikely; a refusal is surfaced as an error and retried by the run state machine.
- Replacing Tavily with Claude's web-search tool (spec mentions it as possible after the switch).
- Moving to `anthropic-version: 2026-10-21`; the request body above is already valid under it.
