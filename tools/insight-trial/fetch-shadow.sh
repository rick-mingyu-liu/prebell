#!/usr/bin/env bash
# Saves the shadow stage's insight records from the live R2 bucket.
# Read-only: R2 object gets, nothing is written remotely.
#
#   tools/insight-trial/fetch-shadow.sh 2026-11-18 2026-12-31 tools/insight-trial/shadow
#
# For each weekday from FROM to TO (both YYYY-MM-DD, at most 400 days apart)
# it tries the three fixed keys a trading day can have:
#   insights/premarket/<date>.json         the 8:00 brief
#   insights/premarket/<date>-update.json  the 8:35 update
#   insights/recap/<date>.json             the recap
# and saves each one found as DIR/insights_<kind>_<date>[-update].json; a
# missing key (no post, a holiday, no record yet) is skipped. It never goes
# through posts.facts_r2_key: after the 8:35 update the post row points at
# the update's bundle, and the 8:00 record would be missed. A file already
# saved is fetched again unless its record is final (stage "done"), since
# an open record may still change. Any wrangler error other than a missing
# key is printed and stops the script. Needs wrangler (through pnpm, from
# crates/generator) and jq.
set -euo pipefail

usage="usage: fetch-shadow.sh FROM TO DIR (dates as YYYY-MM-DD)"
from="${1:?$usage}"
to="${2:?$usage}"
out="${3:?$usage}"
for d in "$from" "$to"; do
  if ! [[ "$d" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    echo "not a YYYY-MM-DD date: $d" >&2
    exit 2
  fi
done
if [[ "$to" < "$from" ]]; then
  echo "TO ($to) is before FROM ($from)" >&2
  exit 2
fi
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
days=0
d="$from"
while [[ "$d" < "$to" || "$d" == "$to" ]]; do
  days=$((days + 1))
  if [ "$days" -gt 400 ]; then
    echo "more than 400 days from $from to $to; narrow the range" >&2
    exit 2
  fi
  if [ "$(weekday "$d")" -le 5 ]; then
    for k in "premarket/$d" "premarket/$d-update" "recap/$d"; do
      key="insights/$k.json"
      f="$out/${key//\//_}"
      if [ -s "$f" ] && [ "$(jq -r .stage "$f")" = "done" ]; then
        continue
      fi
      tmp="$f.part"
      err="$f.err"
      if pnpm exec wrangler r2 object get "prebell-facts/$key" --remote --file "$tmp" >/dev/null 2>"$err" && [ -s "$tmp" ]; then
        mv "$tmp" "$f"
        saved=$((saved + 1))
      elif [ ! -s "$err" ] || grep -qi "does not exist\|not found\|NoSuchKey" "$err"; then
        # A missing key: wrangler says so, or writes an empty file.
        missing=$((missing + 1))
      else
        echo "wrangler failed on $key:" >&2
        cat "$err" >&2
        rm -f "$tmp" "$err"
        exit 1
      fi
      rm -f "$tmp" "$err"
    done
  fi
  d="$(next_day "$d")"
done
echo "records saved: $saved; keys not found: $missing; in $out" >&2
