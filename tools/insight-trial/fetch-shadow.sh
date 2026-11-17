#!/usr/bin/env bash
# Saves the shadow stage's insight records from the live R2 bucket.
# Read-only: R2 object gets, nothing is written remotely.
#
#   tools/insight-trial/fetch-shadow.sh 2026-11-18 2026-12-31 tools/insight-trial/shadow
#
# For each weekday from FROM to TO it tries the three fixed keys a trading
# day can have:
#   insights/premarket/<date>.json         the 8:00 brief
#   insights/premarket/<date>-update.json  the 8:35 update
#   insights/recap/<date>.json             the recap
# and saves each one found as DIR/insights_<kind>_<date>[-update].json; a
# missing key (no post, a holiday, no record yet) is skipped. It never goes
# through posts.facts_r2_key: after the 8:35 update the post row points at
# the update's bundle, and the 8:00 record would be missed. A file already
# saved is fetched again only when its record may still change (stage
# "written"). Needs wrangler (through npx, from crates/generator) and jq.
set -euo pipefail

from="${1:?usage: fetch-shadow.sh FROM TO DIR (dates as YYYY-MM-DD)}"
to="${2:?usage: fetch-shadow.sh FROM TO DIR}"
out="${3:?usage: fetch-shadow.sh FROM TO DIR}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
tool="$(cd "$(dirname "$0")" && pwd)"
cd "$tool/../../crates/generator"

# The day after $1 (YYYY-MM-DD), with BSD (macOS) or GNU date.
next_day() {
  date -j -v+1d -f %Y-%m-%d "$1" +%Y-%m-%d 2>/dev/null || date -d "$1 + 1 day" +%Y-%m-%d
}
# 1-7, Monday to Sunday.
weekday() {
  date -j -f %Y-%m-%d "$1" +%u 2>/dev/null || date -d "$1" +%u
}

saved=0
missing=0
d="$from"
while [[ "$d" < "$to" || "$d" == "$to" ]]; do
  if [ "$(weekday "$d")" -le 5 ]; then
    for k in "premarket/$d" "premarket/$d-update" "recap/$d"; do
      key="insights/$k.json"
      f="$out/${key//\//_}"
      if [ -s "$f" ] && [ "$(jq -r .stage "$f")" = "done" ]; then
        continue
      fi
      tmp="$f.part"
      if npx wrangler r2 object get "prebell-facts/$key" --remote --file "$tmp" >/dev/null 2>&1 && [ -s "$tmp" ]; then
        mv "$tmp" "$f"
        saved=$((saved + 1))
      else
        rm -f "$tmp"
        missing=$((missing + 1))
      fi
    done
  fi
  d="$(next_day "$d")"
done
echo "records saved: $saved; keys not found: $missing; in $out" >&2
