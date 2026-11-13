#!/usr/bin/env bash
# Saves the stored facts the insight trial reads, from the live D1 and R2.
# Read-only: D1 SELECTs and R2 object gets, nothing is written remotely.
#
#   tools/insight-trial/fetch-facts.sh tools/insight-trial/facts
#
# Writes into OUT_DIR:
#   premarket_<date>.json, recap_<date>.json   briefs and recaps, last 14 days
#   earnings_<TICKER>_<accession>.json          earnings checks, last 21 days
#   calendar.json   [[ticker, "YYYY-MM-DD", status], ...] report dates, not quarantined
#
# Each R2 key is saved with "/" replaced by "_"; a file already saved is kept.
# Needs wrangler (run through npx from crates/generator, so it finds the
# config and the account login) and jq.
set -euo pipefail

out="${1:?usage: fetch-facts.sh OUT_DIR}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
tool="$(cd "$(dirname "$0")" && pwd)"
cd "$tool/../../crates/generator"

wrangler() { npx wrangler "$@"; }

# Rows of one read-only query.
query() {
  wrangler d1 execute prebell --remote --json --command "$1" | jq '.[0].results'
}

keys="$(query "SELECT facts_r2_key AS k FROM posts
  WHERE facts_r2_key IS NOT NULL AND published_at IS NOT NULL
    AND ((content_type IN ('premarket', 'recap') AND trading_date >= date('now', '-14 day'))
      OR (content_type = 'earnings' AND trading_date >= date('now', '-21 day')))
  ORDER BY trading_date, facts_r2_key" | jq -r '.[].k')"

n=0
for k in $keys; do
  f="$out/${k//\//_}"
  n=$((n + 1))
  if [ -s "$f" ]; then
    continue
  fi
  wrangler r2 object get "prebell-facts/$k" --remote --file "$f"
done
echo "facts bundles listed: $n" >&2

# Report dates near the posts' trading dates (the pack keeps those within
# 100 days), with their status: reported, confirmed or unconfirmed. An
# unconfirmed date reaches the pack marked as such, and a line citing it
# must hedge it. Quarantined dates are left out.
query "SELECT ticker, report_date, status FROM earnings_calendar
  WHERE status != 'quarantined'
    AND report_date BETWEEN date('now', '-120 day') AND date('now', '+100 day')
  ORDER BY ticker, report_date" | jq '[.[] | [.ticker, .report_date, .status]]' >"$out/calendar.json"

echo "calendar rows: $(jq length "$out/calendar.json")" >&2
