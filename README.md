# PreBell · 盘前铃

Auto-generated US-market briefs in English and Simplified Chinese: a premarket brief before the 9:30 ET open, an after-close recap, and earnings health checks built from SEC filings. Every number in a brief is checked against its source data before publishing.

**Status:** pre-launch. Nothing is deployed yet; see `docs/runbook.md` for the deploy steps.

## Layout

| Path | What |
|---|---|
| `crates/core` | All generator logic (Rust): data fetch, facts, number checker, quote anchoring, EN/ZH drafting, run state machine. Tested natively. |
| `crates/generator` | Cloudflare Worker (workers-rs): cron → queue → `tick`. Config in `wrangler.toml`. |
| `apps/web` | Next.js 16 site on Cloudflare via OpenNext. Routes `/en` and `/zh`. |
| `migrations/` | D1 schema. `db/` holds local preview data only. |
| `docs/` | `spec.md` (design), `plan-*.md` (implementation plans), `runbook.md` (operations), `competitor/` (AlphaLab notes). |

## Develop

```bash
# backend
cargo test -p prebell-core

# site, with sample data (from crates/generator)
pnpm exec wrangler d1 migrations apply prebell --local
pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-preview.sql
bash ../../db/load-preview-facts.sh
cd ../../apps/web && pnpm test && pnpm preview   # http://localhost:8787
```

Secrets live in `crates/generator/.dev.vars` locally (git-ignored) and in Wrangler secrets in production. Never commit them.

## Principles

- Facts first: EN and ZH are written separately from the same verified facts.
- Free tier first: Cloudflare Workers Free limits (10 ms CPU, 50 subrequests, 5 crons) shape the design.
- Only a post's own sources become links; no images or raw HTML from generated text.
