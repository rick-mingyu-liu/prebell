// Regenerates the bundled logos from the pinned `simple-icons` devDependency (CC0 icon data):
//   public/logos/companies/{TICKER}.svg   one tile per mapped watchlist ticker
//   public/logos/partners/{id}.svg (+ -dark.svg)
//   src/lib/logos.generated.ts            the ticker -> file manifest read by src/lib/logos.ts
//   public/logos/LICENSES.md              source, date and licence for every file
// Run `pnpm logos` in apps/web after changing a mapping below or bumping simple-icons. See docs/runbook.md.
import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const WATCHLIST = join(WEB, "../../crates/core/data/watchlist.tsv");
const SI_ROOT = join(dirname(require.resolve("simple-icons/icons.json")), "..");
const SI_VERSION = JSON.parse(readFileSync(join(SI_ROOT, "package.json"), "utf8")).version;
const SI_DATA = require("simple-icons/icons.json");
/** The day the pinned simple-icons version was installed; bump together with the package. */
const FETCHED = "2026-10-30";

/**
 * Watchlist ticker -> simple-icons slug. Only the company's own corporate mark (or, where noted,
 * the consumer brand the company trades as). Tickers left out keep the text chip.
 */
const COMPANIES = {
  AAPL: "apple",
  ABBV: "abbvie",
  ABT: "abbott",
  ACN: "accenture",
  AMD: "amd",
  AXP: "americanexpress",
  BA: "boeing",
  BAC: "bankofamerica",
  BCS: "barclays",
  BKNG: "bookingdotcom", // Booking Holdings' main brand
  CAT: "caterpillar",
  CSCO: "cisco",
  DE: "johndeere",
  F: "ford",
  FDX: "fedex",
  GE: "generalelectric", // the GE monogram, used by GE Aerospace
  GM: "generalmotors",
  GOOGL: "google", // Alphabet trades as Google
  GS: "goldmansachs",
  INTC: "intel",
  INTU: "intuit",
  JPM: "chase", // JPMorgan Chase's consumer brand
  KO: "cocacola",
  MA: "mastercard",
  MCD: "mcdonalds",
  META: "meta",
  MMM: "3m",
  MRK: "merck", // Merck & Co. (merck.com), not Merck KGaA
  NFLX: "netflix",
  NKE: "nike",
  NVDA: "nvidia",
  PLTR: "palantir",
  PYPL: "paypal",
  QCOM: "qualcomm",
  SAP: "sap",
  SBUX: "starbucks",
  T: "atandt",
  TGT: "target",
  TM: "toyota",
  TSLA: "tesla",
  UBER: "uber",
  UPS: "ups",
  V: "visa",
  VZ: "verizon",
  WFC: "wellsfargo",
};

/** Partner id -> simple-icons slug. Wordmarks, drawn black for the light sidebar and white for the dark one. */
const PARTNERS = { okx: "okx" };

const icon = (slug) => {
  const data = SI_DATA.find((i) => i.slug === slug);
  if (!data) throw new Error(`simple-icons ${SI_VERSION} has no "${slug}"`);
  const svg = readFileSync(require.resolve(`simple-icons/icons/${slug}.svg`), "utf8");
  const path = svg.match(/<path d="([^"]+)"/)?.[1];
  if (!path) throw new Error(`no path in ${slug}.svg`);
  return { ...data, path: path.replace(/\s+/g, " ") };
};

const luminance = (hex) => {
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

/**
 * A 32px rounded tile with the 24px glyph inset 4px, so one file reads on the light and the dark
 * page. Brand colours too pale for white (Caterpillar and McDonald's yellow) go on a near-black tile.
 */
function tile({ path, hex }) {
  const pale = 1.05 / (luminance(hex) + 0.05) < 1.6;
  const bg = pale ? "#111" : "#fff";
  const edge = pale ? "#333" : "#e5e5e5";
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">` +
    `<rect x=".5" y=".5" width="31" height="31" rx="7" fill="${bg}" stroke="${edge}"/>` +
    `<path transform="translate(4 4)" fill="#${hex}" d="${path}"/></svg>\n`
  );
}

/** The glyph's own vertical extent (absolute x,y pairs, as in the okx path), so a wide wordmark isn't padded into a square. */
function wordmark({ path }, fill) {
  const ys = [...path.matchAll(/-?\d*\.?\d+/g)].map(Number).filter((_, i) => i % 2 === 1);
  const top = Math.min(...ys);
  const h = Math.max(...ys) - top;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 ${top.toFixed(3)} 24 ${h.toFixed(3)}"><path fill="${fill}" d="${path}"/></svg>\n`;
}

const watchlist = readFileSync(WATCHLIST, "utf8")
  .split("\n")
  .filter((l) => l.trim() && !l.startsWith("#"))
  .map((l) => l.split("\t")[0].trim());
for (const t of Object.keys(COMPANIES)) if (!watchlist.includes(t)) throw new Error(`${t} is not on the watchlist`);

const companiesDir = join(WEB, "public/logos/companies");
const partnersDir = join(WEB, "public/logos/partners");
for (const dir of [companiesDir, partnersDir]) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
}

const rows = [];
const licence = (i) =>
  i.license ? `${i.license.type}${i.license.url ? ` (${i.license.url})` : ""}` : "CC0-1.0 (Simple Icons)";
const row = (file, i) =>
  rows.push(
    `| \`${file}\` | ${i.title} | simple-icons@${SI_VERSION} \`${i.slug}\` (upstream source: ${i.source}) | ${FETCHED} | ${licence(i)} | ${i.guidelines ?? "none listed"} |`,
  );

const manifest = {};
for (const [ticker, slug] of Object.entries(COMPANIES)) {
  const i = icon(slug);
  const file = `${ticker}.svg`;
  writeFileSync(join(companiesDir, file), tile(i));
  manifest[ticker] = `/logos/companies/${file}`;
  row(`companies/${file}`, i);
}
for (const [id, slug] of Object.entries(PARTNERS)) {
  const i = icon(slug);
  writeFileSync(join(partnersDir, `${id}.svg`), wordmark(i, "#000"));
  writeFileSync(join(partnersDir, `${id}-dark.svg`), wordmark(i, "#fff"));
  row(`partners/${id}.svg`, i);
  row(`partners/${id}-dark.svg`, i);
}

writeFileSync(
  join(WEB, "src/lib/logos.generated.ts"),
  `// Generated by scripts/build-logos.mjs from simple-icons@${SI_VERSION}. Do not edit; run \`pnpm logos\`.\n` +
    `export const COMPANY_LOGOS: Readonly<Record<string, string>> = ${JSON.stringify(manifest, null, 2)};\n`,
);

const missing = watchlist.filter((t) => !(t in COMPANIES));
const bytes = [companiesDir, partnersDir]
  .flatMap((d) => readdirSync(d).map((f) => readFileSync(join(d, f)).length))
  .reduce((a, b) => a + b, 0);
writeFileSync(
  join(WEB, "public/logos/LICENSES.md"),
  `# Logo sources and licences

Generated by \`apps/web/scripts/build-logos.mjs\`; edit the script, not this file.

Every glyph comes from the [Simple Icons](https://simpleicons.org) npm package, pinned at
simple-icons@${SI_VERSION} (published 2026-10-25). Simple Icons releases its icon data under
[CC0-1.0](https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md); its
[disclaimer](https://github.com/simple-icons/simple-icons/blob/develop/DISCLAIMER.md) notes that a
per-icon licence, where one is listed, takes precedence and that the marks stay trademarks of their
owners. The script only recolours and frames the glyph (a 32px rounded tile, or a cropped wordmark);
it never changes the shape.

**Copyright vs trademark.** CC0 covers copyright in the drawing. The logos remain trademarks of the
companies named below. PreBell shows them only to identify the company a news item, earnings date or
price move is about (nominative use), never to suggest endorsement or a partnership. Partner logos
appear next to affiliate links, which carry the affiliate disclosure.

Partners without a file here (moomoo, Wealthsimple, Crypto.com) have no Simple Icons entry, and their
official brand pages couldn't be reached or found when this was built, so the sidebar shows their name
as text.

| File | Mark | Source | Fetched | Licence | Brand guidelines |
| --- | --- | --- | --- | --- | --- |
${rows.join("\n")}

Watchlist tickers without a logo (text chip): ${missing.join(", ")}.

Total size of the files above: ${bytes} bytes.
`,
);

console.log(`${Object.keys(manifest).length} company logos, ${missing.length} without, ${bytes} bytes`);
