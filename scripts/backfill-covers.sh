#!/usr/bin/env bash
# One-off: compute cover lines for posts published before covers existed and
# write them as UPDATE statements to backfill-covers.sql. Reads D1 and R2 only;
# it never runs the SQL. Run from the repo root.
set -euo pipefail
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

OUT=backfill-covers.sql
BUCKET=prebell-facts
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
WRANGLER=(pnpm exec wrangler)

cargo build -q -p prebell-core --bin cover-lines
BIN=$(cargo metadata -q --format-version 1 --no-deps | jq -r .target_directory)/debug/cover-lines
: >"$OUT"

# Wrangler config lives in crates/generator; paths below are relative to it.
cd crates/generator
"${WRANGLER[@]}" d1 execute prebell --remote --json --command \
  "SELECT p.id, p.content_type, p.facts_r2_key, t.lang FROM posts p JOIN post_translations t ON t.post_id = p.id WHERE t.cover_line1 IS NULL AND p.facts_r2_key IS NOT NULL" \
  >"$TMP/rows.json"
cd ../..

jq -e -r '.[0].results[] | [.id, .content_type, .facts_r2_key, .lang] | @tsv' "$TMP/rows.json" >"$TMP/rows.tsv"
rows=0 written=0 skipped=0
mkdir "$TMP/lines"   # one file per facts key: its cover lines JSON, or "ERR: reason"

while IFS=$'\t' read -r id kind key lang; do
  rows=$((rows + 1))
  f="$TMP/lines/$(echo "$key" | tr '/' '_')"
  if [[ ! -e $f ]]; then
    if ! (cd crates/generator && "${WRANGLER[@]}" r2 object get "$BUCKET/$key" --file "$f.facts" --remote >/dev/null 2>"$TMP/err"); then
      echo "ERR: download failed: $(tail -n1 "$TMP/err")" >"$f"
    elif ! "$BIN" "$kind" "$f.facts" >"$f" 2>&1; then
      sed -i.bak 's/^/ERR: /' "$f"
    fi
  fi
  lines=$(cat "$f")
  if [[ $lines == ERR:* ]]; then
    echo "skip post $id ($lang): ${lines#ERR: }" >&2
    skipped=$((skipped + 1))
    continue
  fi
  # A brief with no usable quote has an empty line 1: leave the plain cover.
  if [[ $(jq -r --arg l "$lang" '.[$l][0]' <<<"$lines") == "" ]]; then
    echo "skip post $id ($lang): no cover lines in the facts" >&2
    skipped=$((skipped + 1))
    continue
  fi
  jq -r --arg l "$lang" --argjson id "$id" '
    def q: gsub("'"'"'"; "'"''"'");
    "UPDATE post_translations SET cover_line1 = '"'"'\(.[$l][0] | q)'"'"', cover_line2 = '"'"'\(.[$l][1] | q)'"'"' WHERE post_id = \($id) AND lang = '"'"'\($l)'"'"';"
  ' <<<"$lines" >>"$OUT"
  written=$((written + 1))
done <"$TMP/rows.tsv"

echo "rows without cover lines: $rows; UPDATE statements written: $written; skipped: $skipped"
echo "SQL written to $OUT (not executed). To apply it:"
echo "  cd crates/generator && pnpm exec wrangler d1 execute prebell --remote --file ../../$OUT"
