#!/usr/bin/env bash
# Loads db/preview-facts/premarket-{date}.json into local R2 at premarket/{date}.json,
# the key format the generator writes, so the seed briefs render the facts blocks;
# and db/preview-facts/ipo/** at ipo/** (the IPO write-up seed, db/seed-ipo-articles-preview.sql).
# Run after db/seed-preview.sql: bash db/load-preview-facts.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/../crates/generator"
for f in "$here"/preview-facts/premarket-*.json; do
  date="$(basename "$f" .json)"
  date="${date#premarket-}"
  pnpm exec wrangler r2 object put "prebell-facts/premarket/${date}.json" --local --file "$f" \
    --content-type application/json
done
for f in "$here"/preview-facts/ipo/*/*.json; do
  key="ipo/${f#"$here"/preview-facts/ipo/}"
  pnpm exec wrangler r2 object put "prebell-facts/${key}" --local --file "$f" \
    --content-type application/json
done
