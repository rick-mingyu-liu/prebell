# PreBell · 盘前铃

US stock market briefs, written and checked by code, published in English and Simplified Chinese at [prebell.site](https://prebell.site) with no human in the loop.

- **Premarket brief** at 8:00 ET, updated at 8:35 ET after the economic releases, and an **after-close recap**.
- **Earnings checks** built from companies' SEC filings, minutes after they file.
- **IPO watch**: large upcoming listings tracked through their SEC filings, with a write-up at each milestone.
- **MCP server** so AI agents can read the same data:

```bash
claude mcp add --transport http prebell https://prebell.site/mcp
```

Every number in a post is checked against its source data before it publishes; a post whose numbers don't check out is not published.

**Status:** live since November 2026, on Cloudflare's free plan. Operations are in `docs/runbook.md`.

## Layout

| Path | What |
|---|---|
| `crates/core` | All generator logic (Rust): data fetch, facts, number checker, quote anchoring, EN/ZH drafting, run state machine. Tested natively. |
| `crates/generator` | Cloudflare Worker (workers-rs): cron → queue → `tick`. Config in `wrangler.toml`. |
| `apps/web` | Next.js 16 site on Cloudflare via OpenNext. Routes `/en` and `/zh`. |
| `migrations/` | D1 schema. `db/` holds local preview data only. |
| `docs/` | `spec.md` (design), `plan-*.md` (implementation plans; plan 4 was a feature later removed), `runbook.md` (operations). |

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

## License

MIT. See `LICENSE`.
