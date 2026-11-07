#!/usr/bin/env bash
# One-off: re-cover every published post that has no SVG cover yet. Each post's
# stored facts are rendered to a data-graphic SVG, uploaded to R2 as
# covers/<slug>.svg, and the matching UPDATE statements are written to
# backfill-graphics.sql. A post whose facts have no graphic gets cover_key = NULL
# so an old scene image does not show under the paper styling. It writes to R2
# but never runs the SQL. Run from the repo root.
set -euo pipefail
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

OUT=backfill-graphics.sql
IMG_DIR=backfill-graphics
BUCKET=prebell-facts
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
WRANGLER=(pnpm exec wrangler)

cargo build -q -p prebell-core --bin cover-graphic
BIN=$(cargo metadata -q --format-version 1 --no-deps | jq -r .target_directory)/debug/cover-graphic
mkdir -p "$IMG_DIR"
: >"$OUT"

# Wrangler config lives in crates/generator; paths below are relative to it.
cd crates/generator
"${WRANGLER[@]}" d1 execute prebell --remote --json --command \
  "SELECT slug, content_type, facts_r2_key FROM posts WHERE published_at IS NOT NULL AND facts_r2_key IS NOT NULL AND (cover_key IS NULL OR cover_key NOT LIKE '%.svg')" \
  >"$TMP/rows.json"
cd ../..

jq -e -r '.[0].results[] | [.slug, .content_type, .facts_r2_key] | @tsv' "$TMP/rows.json" >"$TMP/rows.tsv" || true
if [[ ! -s $TMP/rows.tsv ]]; then
  echo "no posts without an SVG cover (or the D1 reply had no rows)"
  exit 0
fi
rows=0 uploaded=0 flat=0 skipped=0

skip() { echo "skip $slug: $1" >&2; skipped=$((skipped + 1)); }

while IFS=$'\t' read -r slug kind key; do
  rows=$((rows + 1))
  if [[ ! $slug =~ ^[a-z0-9-]{1,80}$ ]]; then
    skip "unexpected slug"; continue
  fi
  facts="$TMP/facts.json" svg="$IMG_DIR/$slug.svg"
  rm -f "$facts" "$svg"

  if ! (cd crates/generator && "${WRANGLER[@]}" r2 object get "$BUCKET/$key" --file "$facts" --remote >/dev/null 2>"$TMP/err"); then
    skip "facts download failed: $(tail -n1 "$TMP/err")"; continue
  fi

  rc=0
  "$BIN" "$kind" "$facts" >"$svg" 2>"$TMP/err" || rc=$?
  if [[ $rc == 3 ]]; then
    rm -f "$svg"
    echo "flat $slug: no graphic (cover_key -> NULL)"
    echo "UPDATE posts SET cover_key = NULL WHERE slug = '$slug';" >>"$OUT"
    flat=$((flat + 1))
    continue
  elif [[ $rc != 0 ]]; then
    rm -f "$svg"
    skip "render failed: $(tail -n1 "$TMP/err")"; continue
  fi

  r2key="covers/$slug.svg"
  if ! up=$(cd crates/generator && "${WRANGLER[@]}" r2 object put "$BUCKET/$r2key" --file "../../$svg" --content-type image/svg+xml --remote 2>&1) \
     || [[ $up != *"Upload complete"* ]]; then
    skip "upload failed: $(tail -n1 <<<"$up")"; continue
  fi

  echo "UPDATE posts SET cover_key = '$r2key' WHERE slug = '$slug';" >>"$OUT"
  echo "$slug: $r2key, $(wc -c <"$svg" | tr -d ' ') bytes"
  uploaded=$((uploaded + 1))
done <"$TMP/rows.tsv"

echo "posts without an SVG cover: $rows; uploaded: $uploaded; flat: $flat; skipped: $skipped"
echo "SQL written to $OUT (not executed). To apply it:"
echo "  cd crates/generator && pnpm exec wrangler d1 execute prebell --remote --file ../../$OUT"
