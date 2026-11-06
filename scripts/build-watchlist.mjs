// Regenerates crates/core/data/watchlist.tsv (the earnings watchlist) and
// apps/web/src/lib/companies.generated.ts (ticker -> name and tier for the site) from files in git:
//   crates/core/data/universe.txt            S&P 500 ∪ Nasdaq-100 (membership)
//   crates/core/data/watchlist-overrides.tsv hand-checked tiers, names, domains, extra companies
//   crates/core/data/marketcap-snapshot.tsv  dated market caps (the $100B write-up line)
// plus SEC data: company_tickers.json (ticker -> CIK) and each CIK's submissions JSON (conformed
// name, filing history -> domestic or foreign filer). SEC fair access: a User-Agent with a contact
// address and at most 5 requests/second (half the 10/s limit). SEC answers (200) are cached in
// node_modules/.cache/watchlist for CACHE_HOURS, so reruns the same day write the same bytes; an
// older entry is fetched again, so a rename or a new filing type shows up on the next day's run.
// Usage (repo root):
//   node scripts/build-watchlist.mjs            generate (fetches what the cache lacks or has old)
//   node scripts/build-watchlist.mjs --refresh  fetch everything from SEC again, whatever the cache
//   node scripts/build-watchlist.mjs --offline  use only the cache, whatever its age
//   node scripts/build-watchlist.mjs --caps     refresh marketcap-snapshot.tsv (Nasdaq screener)
//                                               first
// A full-tier row without domains stops the run and prints Wikidata's official websites (P856)
// for those CIKs as suggestions; check them and add them to the overrides. See docs/runbook.md.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DATA = join(ROOT, "crates/core/data");
const UNIVERSE = join(DATA, "universe.txt");
const OVERRIDES = join(DATA, "watchlist-overrides.tsv");
const CAPS = join(DATA, "marketcap-snapshot.tsv");
const OUT = join(DATA, "watchlist.tsv");
const WEB_OUT = join(ROOT, "apps/web/src/lib/companies.generated.ts");
const CACHE = join(ROOT, "node_modules/.cache/watchlist");
const OFFLINE = process.argv.includes("--offline");
const REFRESH_CAPS = process.argv.includes("--caps");
const REFRESH_SEC = process.argv.includes("--refresh");
/** A cached SEC answer older than this is fetched again (unless --offline). */
const CACHE_HOURS = 20;
const UA = "PreBell watchlist build (https://prebell.site; support@prebell.site)";
/** 5 requests/second to SEC. */
const SEC_GAP_MS = 200;
/** Write-ups for every company at or above this market cap (Rick, 2026-11-06). */
const FULL_CAP = 100e9;
/** Filing history read for the filer type: this many days before the snapshot date. */
const DOMESTIC_DAYS = 456;
/** The Nasdaq screener refuses non-browser clients. */
const BROWSER_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
/** watchlist.rs reads every row; 11 Yahoo batches of 50 in the calendar refresh. */
const MAX_ROWS = 550;

/**
 * Rows whose display name does not start with a word of their SEC name, checked by hand. Any
 * other such row stops the run: SEC may map a reused ticker to a different company (SATS went
 * from EchoStar to Strive), and the CIK would follow it.
 */
const NAME_OK = {
  BNY: "Bank of New York Mellon",
  GE: "General Electric, trading as GE Aerospace",
  IBM: "International Business Machines",
  SPCX: "Space Exploration Technologies",
  WAB: "Westinghouse Air Brake Technologies, trading as Wabtec",
};
/** The same noise words as `NAME_NOISE` in crates/core/src/earnings/detect.rs. */
const NAME_NOISE = new Set(
  ("inc incorporated corp corporation co company companies ltd limited plc holdings holding the " +
    "group sa nv ag lp llc").split(" "),
);
/** Lowercase, accent-free significant words ("Itaú Unibanco" -> ["itau", "unibanco"]). */
const nameWords = (name) =>
  name
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[&.'\u2019]/g, "")
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w && !NAME_NOISE.has(w));
/**
 * The display name's first word of 3 or more letters (initials such as "C.H." are spelled apart on
 * EDGAR) is a word of the SEC name, or the SEC name's first two words joined ("ExxonMobil").
 */
const sameCompany = (display, sec) => {
  const all = nameWords(display);
  const key = all.find((w) => w.length >= 3) ?? all[0];
  const words = nameWords(sec);
  return Boolean(key) && (words.includes(key) || words.slice(0, 2).join("") === key);
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fail = (msg) => {
  console.error(`build-watchlist: ${msg}`);
  process.exit(1);
};

if (OFFLINE && REFRESH_SEC) fail("--offline and --refresh exclude each other");

/** Tab-separated data lines without comments. */
const tsv = (path) =>
  readFileSync(path, "utf8")
    .split("\n")
    .filter((l) => l.trim() && !l.startsWith("#"))
    .map((l) => l.split("\t").map((s) => s.trim()));
const opt = (s) => (s && s !== "-" ? s : null);

// ---------------------------------------------------------------------------------------------
// SEC, cached.

let lastSec = 0;
async function secJson(url) {
  mkdirSync(CACHE, { recursive: true });
  const file = join(CACHE, `${createHash("sha1").update(url).digest("hex")}.json`);
  const fresh = existsSync(file) && Date.now() - statSync(file).mtimeMs < CACHE_HOURS * 3_600_000;
  if (existsSync(file) && (OFFLINE || (fresh && !REFRESH_SEC))) return JSON.parse(readFileSync(file, "utf8"));
  if (OFFLINE) fail(`not cached (--offline): ${url}`);
  const wait = lastSec + SEC_GAP_MS - Date.now();
  if (wait > 0) await sleep(wait);
  lastSec = Date.now();
  const res = await fetch(url, { headers: { "User-Agent": UA, Accept: "application/json" } });
  if (res.status === 429 || res.status === 403) fail(`SEC answered ${res.status} for ${url}; wait and rerun`);
  if (res.status !== 200) fail(`SEC answered ${res.status} for ${url}`);
  const text = await res.text();
  writeFileSync(file, text);
  return JSON.parse(text);
}

// ---------------------------------------------------------------------------------------------
// Market caps (--caps): one request for Nasdaq's stock screener, which lists every US-listed stock
// with its market cap (Yahoo's v7 quote answered 429 to the build machine on 2026-11-06).

async function fetchCaps(tickers) {
  const url = "https://api.nasdaq.com/api/screener/stocks?tableonly=true&limit=10000&download=true";
  const res = await fetch(url, { headers: { "User-Agent": BROWSER_UA, Accept: "application/json" } });
  if (res.status !== 200) fail(`Nasdaq screener answered ${res.status}`);
  const rows = (await res.json())?.data?.rows ?? [];
  // The screener writes class shares with a slash (BRK/B).
  const bySymbol = new Map(rows.map((r) => [r.symbol.replace("/", "."), Number(r.marketCap) || 0]));
  const unknown = tickers.filter((t) => !bySymbol.has(t));
  if (unknown.length) fail(`Nasdaq screener does not list ${unknown.join(" ")}`);
  const noCap = tickers.filter((t) => bySymbol.get(t) === 0);
  const day = new Intl.DateTimeFormat("en-CA", { timeZone: "America/New_York" }).format(new Date());
  const lines = [
    `# Market caps (USD, Nasdaq stock screener) of the universe.txt tickers, fetched ${day} ET by`,
    "# scripts/build-watchlist.mjs --caps. 0 = the screener gave none. The $100B write-up line",
    "# reads this snapshot; refresh it with the quarterly index refresh, not daily.",
    `# as-of ${day}`,
    ...tickers.map((t) => `${t}\t${Math.round(bySymbol.get(t))}`),
  ];
  writeFileSync(CAPS, `${lines.join("\n")}\n`);
  console.log(`wrote ${CAPS} (${tickers.length} caps, ${day}; none for ${noCap.join(" ") || "-"})`);
}

// ---------------------------------------------------------------------------------------------
// Wikidata suggestions for missing domains (one SPARQL request; printed, never written).

async function suggestDomains(rows) {
  if (OFFLINE || rows.length === 0) return;
  const values = rows.map((r) => `"${String(r.cik).padStart(10, "0")}"`).join(" ");
  const q = `SELECT ?cik ?site WHERE { VALUES ?cik { ${values} } ?item wdt:P5531 ?cik; wdt:P856 ?site. }`;
  const res = await fetch(`https://query.wikidata.org/sparql?format=json&query=${encodeURIComponent(q)}`, {
    headers: { "User-Agent": UA, Accept: "application/sparql-results+json" },
  });
  if (res.status !== 200) return console.error(`Wikidata answered ${res.status}; no suggestions`);
  const sites = new Map();
  for (const b of (await res.json()).results.bindings) {
    const host = new URL(b.site.value).hostname.replace(/^www\./, "");
    const cik = Number(b.cik.value);
    sites.set(cik, [...new Set([...(sites.get(cik) ?? []), host])]);
  }
  console.error("Suggestions (Wikidata P856; check each, then add to watchlist-overrides.tsv):");
  for (const r of rows) console.error(`${r.ticker}\t-\t-\t${(sites.get(r.cik) ?? ["?"]).join(",")}`);
}

// ---------------------------------------------------------------------------------------------
// Build.

const universe = tsv(UNIVERSE).map(([ticker, name]) => ({ ticker, name: name.replace(/ \(Class [A-Z]\)$/, "") }));
const inUniverse = new Set(universe.map((u) => u.ticker));
const overrides = new Map();
for (const [ticker, tier, name, domains] of tsv(OVERRIDES)) {
  if (overrides.has(ticker)) fail(`overrides: ${ticker} twice`);
  if (tier && !["full", "calendar", "drop", "-"].includes(tier)) fail(`overrides: ${ticker}: bad tier ${tier}`);
  overrides.set(ticker, { tier: opt(tier), name: opt(name), domains: opt(domains)?.split(",") ?? [] });
}

if (REFRESH_CAPS) await fetchCaps(universe.map((u) => u.ticker));
if (!existsSync(CAPS)) fail(`${CAPS} is missing; run with --caps`);
const asOf = readFileSync(CAPS, "utf8").match(/^# as-of (\d{4}-\d{2}-\d{2})$/m)?.[1];
if (!asOf) fail("marketcap-snapshot.tsv has no `# as-of YYYY-MM-DD` line");
const caps = new Map(tsv(CAPS).map(([t, c]) => [t, Number(c)]));

const tickerIndex = await secJson("https://www.sec.gov/files/company_tickers.json");
const secByTicker = new Map(Object.values(tickerIndex).map((e) => [e.ticker.replace("-", "."), e]));

const candidates = [
  ...universe.map((u) => u.ticker),
  ...[...overrides.keys()].filter((t) => !inUniverse.has(t)),
];
const rows = [];
const notes = { no202: [], titleDiffers: [] };
for (const ticker of candidates) {
  const o = overrides.get(ticker) ?? { tier: null, name: null, domains: [] };
  if (o.tier === "drop") continue;
  const sec = secByTicker.get(ticker);
  if (!sec) fail(`${ticker} is not in SEC company_tickers.json (renamed or delisted?)`);
  const cik = sec.cik_str;
  const sub = await secJson(`https://data.sec.gov/submissions/CIK${String(cik).padStart(10, "0")}.json`);
  const recent = sub.filings.recent;
  const since = new Date(Date.parse(asOf) - DOMESTIC_DAYS * 86_400_000).toISOString().slice(0, 10);
  const inWindow = (i) => recent.filingDate[i] >= since && recent.filingDate[i] <= asOf;
  const filed = (pred) => recent.form.some((f, i) => inWindow(i) && pred(f, i));
  // Foreign: files 20-F/40-F/6-K and no 10-Q. A new company (spin-off, IPO) with no 10-Q yet is
  // domestic.
  const fpi = filed((f) => ["20-F", "40-F", "6-K"].includes(f)) && !filed((f) => f === "10-Q");
  const filer = fpi ? "foreign" : "domestic";
  if (filer === "domestic" && !filed((f, i) => f === "8-K" && recent.items[i].split(",").includes("2.02")))
    notes.no202.push(ticker);
  const secName = sub.name.trim();
  if (secName.toLowerCase() !== sec.title.trim().toLowerCase())
    notes.titleDiffers.push(`${ticker}: ${sec.title} / ${secName}`);
  const name = o.name ?? universe.find((u) => u.ticker === ticker)?.name;
  if (!name) fail(`${ticker}: an extra company needs a name in the overrides`);
  if (!sameCompany(name, secName) && !(ticker in NAME_OK))
    fail(
      `${ticker}: SEC files CIK ${cik} as "${secName}", not "${name}" (a reused ticker?); ` +
        "check it, then add it to NAME_OK",
    );
  if (!inUniverse.has(ticker) && o.tier !== "calendar") fail(`${ticker}: extra companies are calendar only`);
  // A universe ticker the snapshot lacks fails (rerun with --caps); 0 is a cap the screener left
  // out.
  if (inUniverse.has(ticker) && !caps.has(ticker))
    fail(`${ticker} is not in marketcap-snapshot.tsv; run with --caps`);
  let tier = o.tier ?? (caps.get(ticker) >= FULL_CAP ? "full" : "calendar");
  // A foreign (6-K) filer has no Item 2.02 to write from.
  if (filer === "foreign") tier = "calendar";
  rows.push({ ticker, cik, filer, name, domains: o.domains, sec: secName, tier });
}
for (const t of overrides.keys()) if (!candidates.includes(t)) fail(`overrides: ${t} is unknown`);

const byCik = new Map();
for (const r of rows) {
  if (byCik.has(r.cik)) fail(`${byCik.get(r.cik)} and ${r.ticker} share CIK ${r.cik}; drop one in the overrides`);
  byCik.set(r.cik, r.ticker);
}
if (rows.length > MAX_ROWS) fail(`${rows.length} rows (at most ${MAX_ROWS})`);
const noDomains = rows.filter((r) => r.tier === "full" && r.domains.length === 0);
if (noDomains.length) {
  console.error(`full-tier rows without domains: ${noDomains.map((r) => r.ticker).join(" ")}`);
  await suggestDomains(noDomains);
  process.exit(1);
}

rows.sort((a, b) => (a.ticker < b.ticker ? -1 : 1));
const header = `# GENERATED by scripts/build-watchlist.mjs; do not edit. Change watchlist-overrides.tsv,
# universe.txt or marketcap-snapshot.tsv and rerun (see docs/runbook.md).
#
# The earnings watchlist: S&P 500 ∪ Nasdaq-100 (universe.txt, one row per CIK) plus hand-picked
# large foreign companies. Write-ups ("full" tier) for the S&P 100 and every company at or above
# $100B in the market-cap snapshot of ${asOf}; the rest is calendar only. CIKs and SEC names from
# SEC company_tickers.json and submissions; a filer is foreign when, in the ${DOMESTIC_DAYS} days
# before ${asOf}, it filed a 20-F, 40-F or 6-K and no 10-Q. The poller skips (and alerts once a
# day about) an EDGAR filing under a row's CIK whose filer name is not the row's SEC name.
#
# Columns (tab-separated): ticker, CIK, filer (domestic | foreign), display name, company web
# domains (comma-separated; subdomains match; calendar proof), SEC conformed name, tier
# (full | calendar).
`;
const body = rows.map((r) => [r.ticker, r.cik, r.filer, r.name, r.domains.join(","), r.sec, r.tier].join("\t"));
const web = `// GENERATED by scripts/build-watchlist.mjs from crates/core/data/watchlist.tsv; do not edit.
/** Watchlist ticker -> [display name, has earnings write-ups]. */
export const COMPANIES: Readonly<Record<string, readonly [name: string, writeUps: boolean]>> = {
${rows.map((r) => `  ${JSON.stringify(r.ticker)}: [${JSON.stringify(r.name)}, ${r.tier === "full"}],`).join("\n")}
};
`;
writeFileSync(`${OUT}.tmp`, `${header}${body.join("\n")}\n`);
writeFileSync(`${WEB_OUT}.tmp`, web);
renameSync(`${OUT}.tmp`, OUT);
renameSync(`${WEB_OUT}.tmp`, WEB_OUT);

const count = (key, value) => rows.filter((r) => r[key] === value).length;
console.log(
  `wrote ${rows.length} rows: ${count("tier", "full")} full, ${count("tier", "calendar")} calendar; ` +
    `${count("filer", "domestic")} domestic, ${count("filer", "foreign")} foreign (caps as of ${asOf})`,
);
console.log(`foreign filers: ${rows.filter((r) => r.filer === "foreign").map((r) => r.ticker).join(" ")}`);
console.log(
  `domestic, no 8-K Item 2.02 in ${DOMESTIC_DAYS} days (never detected, calendar seeds only): ` +
    (notes.no202.join(" ") || "none"),
);
if (notes.titleDiffers.length)
  console.log(`company_tickers title differs from the submissions name:\n  ${notes.titleDiffers.join("\n  ")}`);
