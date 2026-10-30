#!/usr/bin/env bash
# Live check of the Yahoo flow in crates/core/src/yahoo.rs (8 requests, 2 s apart).
# Usage: scripts/yahoo-live-check.sh OUT_DIR   (OUT_DIR outside the repo)
# Prints statuses, Set-Cookie names (never values) and the crumb's length and
# character set (never the crumb). Writes the JSON bodies to OUT_DIR for
# trimming into crates/core/tests/fixtures/. Stops at the first failure or 429.
set -euo pipefail
OUT=${1:?usage: $0 OUT_DIR}
mkdir -p "$OUT"
JAR=$(mktemp)
trap 'rm -f "$JAR" "$OUT"/*.hdr' EXIT
# The same User-Agent as `UA` in yahoo.rs.
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36"
HERE=$(cd "$(dirname "$0")/.." && pwd)
SYMS=$(grep -v '^#' "$HERE/crates/core/data/universe.txt" | cut -f1 | head -42 | tr '.' '-' | paste -sd, -),MSFT,NVDA,TSLA,INTC,BRK-B,XOM,META,GOOGL

echo "start $(TZ=America/New_York date '+%F %T %Z')"
# get NAME URL [curl args...]: saves NAME.hdr and NAME.body, prints the status.
get() {
  local name=$1 url=$2; shift 2
  local code
  code=$(curl -sS --max-time 20 -A "$UA" -D "$OUT/$name.hdr" -o "$OUT/$name.body" \
    -w '%{http_code} redirects=%{num_redirects}' "$@" "$url") || { echo "$name: curl failed"; exit 1; }
  echo "$name: $code"
  if [[ $code == 429* ]]; then echo "429: stopping"; exit 1; fi
  sleep 2
}

get fc https://fc.yahoo.com -c "$JAR"
echo "fc Set-Cookie names: $(grep -i '^set-cookie:' "$OUT/fc.hdr" | sed -E 's/^[^:]+: *([^=]+)=.*/\1/' | paste -sd' ' -)"
echo "fc Location: $(grep -i '^location:' "$OUT/fc.hdr" | cut -d' ' -f2- || true)"

get crumb https://query1.finance.yahoo.com/v1/test/getcrumb -b "$JAR"
CRUMB=$(cat "$OUT/crumb.body"); rm "$OUT/crumb.body"
echo "crumb length=${#CRUMB} chars=$(printf %s "$CRUMB" | fold -w1 | sort -u | tr -d '\n' | sed -E 's/[A-Za-z0-9]//g')+alnum"
ENC=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$CRUMB")

get quote "https://query1.finance.yahoo.com/v7/finance/quote?symbols=$SYMS&crumb=$ENC" -b "$JAR" -H 'Accept: application/json'
get badcrumb "https://query1.finance.yahoo.com/v7/finance/quote?symbols=AAPL&crumb=notARealCrumb1" -b "$JAR" -H 'Accept: application/json'
for s in ES%3DF %5EGSPC AAPL NVDA; do
  get "chart_$s" "https://query1.finance.yahoo.com/v8/finance/chart/$s?interval=1m&range=1d&includePrePost=true" -H 'Accept: application/json'
done
unset CRUMB ENC
echo "done $(TZ=America/New_York date '+%F %T %Z'); bodies in $OUT"
