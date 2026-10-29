# PreBell runbook

Operational notes for the owner. Plan 1 Task 13 adds the deploy, cron and rerun sections to this file.

## LLM provider

Every LLM call goes through one provider: reason extraction while building facts, and the EN/ZH drafts. The generator Worker picks the provider from config when each queue batch starts.

| `LLM_PROVIDER` | API | Secret | Default model |
|---|---|---|---|
| `deepseek` (default) | `https://api.deepseek.com/chat/completions` | `DEEPSEEK_API_KEY` | `deepseek-chat` |
| `claude` | `https://api.anthropic.com/v1/messages` | `ANTHROPIC_API_KEY` | `claude-sonnet-5-5` |

- `LLM_PROVIDER` and the optional `LLM_MODEL` are plain vars in `crates/generator/wrangler.toml` under `[vars]`. If `LLM_MODEL` is unset or empty, the provider's default model is used.
- The secrets are set only with `wrangler secret put` (production) or in `crates/generator/.dev.vars` (local, git-ignored). They are never committed.
- Any other `LLM_PROVIDER` value makes the queue handler fail with `parse error: unknown LLM provider: <value>` before it does any work. No run row and no alert are written, so this shows only in `wrangler tail`.
- The number check (`numbers::check`) and quote anchoring run the same way whichever provider is used. Switching provider does not loosen them.

### Switch to Claude (owner only; this deploys)

1. `cd crates/generator && pnpm exec wrangler secret put ANTHROPIC_API_KEY` and paste the key.
2. In `crates/generator/wrangler.toml`, set `LLM_PROVIDER = "claude"`. To use a different model, also add `LLM_MODEL = "<model id>"`, for example `claude-opus-5-5`.
3. Redeploy the generator (`pnpm exec wrangler deploy` in `crates/generator`). Vars take effect on deploy.
4. Check the next run (and time it: compare `updated_at` with 08:30 ET): `pnpm exec wrangler d1 execute prebell --remote --command "select run_key,status,attempts,last_error from runs order by updated_at desc limit 3"`.

Claude thinks before answering, so a build (up to 11 extraction calls plus the EN and ZH drafts) may take longer than on DeepSeek. A build older than 15 minutes counts as stale and a second one starts. If the first runs take more than about 10 minutes, tell the developer: a faster `LLM_MODEL` or a lower effort setting are the options.

### Switch back to DeepSeek

Set `LLM_PROVIDER = "deepseek"`, remove `LLM_MODEL` if you set it to a Claude model, and redeploy. `DEEPSEEK_API_KEY` can stay set the whole time, and so can `ANTHROPIC_API_KEY`.

### Local dev

Put the key in `crates/generator/.dev.vars`:

```
LLM_PROVIDER=claude
ANTHROPIC_API_KEY=sk-ant-...
```

Values in `.dev.vars` override `[vars]` under `wrangler dev`. Then follow the local steps in `docs/plan-1-notes.md`. A missing key logs `secret ANTHROPIC_API_KEY is not set; provider calls will fail`.

### What Claude failures look like

Where a failure shows up depends on which call failed.

- **Reason extraction** (the first calls in a build): errors are not recorded. If every extraction fails — wrong key, rate limit, outage, or refusals — no mover keeps a reason, the required movers section is missing, and `runs.last_error` reads just `movers`. That is the same thing you see when the news has no usable reasons, so use `pnpm exec wrangler tail` in `crates/generator` to see the HTTP status. The run stays `pending` and retries every 5 minutes until 09:25 ET, then it is skipped and an error alert is raised.
- **EN draft:** each draft gets two attempts per tick. If both fail, `last_error` reads `en: llm: <error>` and the run retries on later ticks until 09:25 ET as above.
- **ZH draft:** EN still publishes. ZH is stored as `pending` ("中文版稍后推出") and `last_error` reads `zh: llm: <error>`. Later firings retry ZH, hourly after the morning window. A failure at or after 18:00 ET marks ZH `failed` and raises one warn alert.

The `<error>` part:

| Error | Meaning |
|---|---|
| `http status 401` | Wrong or missing `ANTHROPIC_API_KEY`. |
| `http status 429` | Rate limit. |
| `http status 529` / `5xx` | Anthropic overloaded or unavailable; usually clears on a later tick. |
| `parse error: truncated` | The reply hit `max_tokens` (16000). It is never used as partial JSON. |
| `parse error: refusal: <category>` | The model declined the request. Facts are only saved after a successful EN draft, so find the request in `wrangler tail`. |
| `parse error: stop_reason: <x>` | The turn ended unfinished in some other way. It is never used. |
| `parse error: ...` (other) | The reply was not a JSON object. |
