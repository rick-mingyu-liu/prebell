// Shared by build-logos.mjs and build-photos.mjs: the watchlist reader, the cached and rate-limited GET,
// the SEC CIK -> Wikidata item lookup and a few small helpers. Each script passes its own cache folder
// and User-Agent; nothing here runs on import.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const today = () => new Date().toISOString().slice(0, 10);

/** The first 8 hex digits of a file's SHA-1: the `?v=` of its URL, so a changed file is a new URL (served immutable for a year). */
export const hashOf = (buf) => createHash("sha1").update(buf).digest("hex").slice(0, 8);

/** HTML -> text: tags removed and whitespace collapsed (entities are left as they are). */
export const plain = (html = "") => html.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();

/** Every row of crates/core/data/watchlist.tsv (both tiers): { ticker, cik, name, domains, tier }. */
export function readWatchlist(path) {
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((l) => l.trim() && !l.startsWith("#"))
    .map((l) => {
      const [ticker, cik, , name, domains = "", , tier] = l.split("\t").map((s) => s.trim());
      return { ticker, cik, name, domains: domains.split(",").filter(Boolean), tier };
    });
}

/** A 2xx, or a 4xx a retry won't change (not 408 timeout or 429 rate limit): safe to cache. */
const definitive = (status) => (status >= 200 && status < 300) || (status >= 400 && status < 500 && status !== 408 && status !== 429);

/**
 * A GET with an on-disk cache in `cache`, at most two requests a second to Wikimedia and one a second
 * elsewhere, with `ua` as the User-Agent. `get(url, required)` returns { body, type, url, fetched }, or
 * null. Only definitive answers are cached. A network error, timeout, 408, 429 or 5xx is not: for a
 * `required` source it stops the run (a missing answer would silently drop files); otherwise it is
 * pushed onto `retryable` and reported by the caller. `fresh: true` skips the cache read (the answer
 * is still cached, for `offline` runs). `getJson` is `get(url, true)` parsed as JSON.
 */
export function createFetcher({ cache, ua, offline = false }) {
  let lastRequest = 0;
  const retryable = [];
  async function get(url, required = false, { fresh = false } = {}) {
    mkdirSync(cache, { recursive: true });
    const key = join(cache, createHash("sha1").update(url).digest("hex"));
    const cached = existsSync(`${key}.json`) && JSON.parse(readFileSync(`${key}.json`, "utf8"));
    // Older caches also hold transient failures (no status, or a retryable one): those don't count.
    if (cached && (offline || !fresh) && definitive(cached.status ?? (cached.ok ? 200 : 0)))
      return cached.ok ? { ...cached, body: readFileSync(key) } : null;
    if (retryable.includes(url)) return null; // already failed this run
    let why = "not cached (--offline)";
    if (!offline) {
      await new Promise((r) => setTimeout(r, Math.max(0, lastRequest + (/(^|\.)(wikimedia|wikidata|wikipedia)\.org$/.test(new URL(url).hostname) ? 500 : 1000) - Date.now())));
      lastRequest = Date.now();
      try {
        const res = await fetch(url, { headers: { "User-Agent": ua }, signal: AbortSignal.timeout(20_000) });
        const meta = { ok: res.ok, status: res.status, url: res.url, type: res.headers.get("content-type") ?? "", fetched: today() };
        const body = Buffer.from(await res.arrayBuffer());
        if (definitive(res.status)) {
          writeFileSync(`${key}.json`, JSON.stringify(meta));
          if (res.ok) writeFileSync(key, body);
          if (!res.ok) console.warn(`  ${url}: HTTP ${res.status}`);
          return res.ok ? { ...meta, body } : null;
        }
        why = `HTTP ${res.status}`;
      } catch (e) {
        why = e.cause?.code ?? e.name;
      }
    }
    if (required) throw new Error(`${url}: ${why}. Nothing was written; run again later.`);
    console.warn(`  ${url}: ${why} (not cached)`);
    retryable.push(url);
    return null;
  }
  const getJson = async (url, opts) => {
    const res = await get(url, true, opts);
    if (!res) throw new Error(`${url}: no answer`);
    return JSON.parse(res.body.toString("utf8"));
  };
  return { get, getJson, retryable };
}

/** Wikidata items for tickers whose watchlist CIK has no Wikidata match (new holding companies, foreign issuers). */
export const WIKIDATA_ITEMS = { BLK: "Q219635", XOM: "Q156238", NVO: "Q818846", SAN: "Q6496310" };

/**
 * Ticker -> Wikidata item, matched on the SEC CIK (P5531, stored zero-padded to 10 digits). `overrides`
 * names the item for tickers whose CIK has no Wikidata match (new holding companies, foreign issuers).
 */
export async function wikidataItems(rows, getJson, overrides = {}) {
  const byCik = {};
  for (let i = 0; i < rows.length; i += 80) {
    const ciks = rows.slice(i, i + 80).map((r) => `"${r.cik.padStart(10, "0")}"`).join(" ");
    const query = `SELECT ?cik ?item WHERE { VALUES ?cik { ${ciks} } ?item wdt:P5531 ?cik. }`;
    const res = await getJson(`https://query.wikidata.org/sparql?format=json&query=${encodeURIComponent(query)}`);
    for (const b of res.results.bindings) byCik[b.cik.value] ??= b.item.value.split("/").pop();
  }
  return Object.fromEntries(rows.map((r) => [r.ticker, overrides[r.ticker] ?? byCik[r.cik.padStart(10, "0")]]));
}

/** Current values of a property: not deprecated, no end date, preferred rank and newest start first. */
export function current(claims = []) {
  const start = (c) => c.qualifiers?.P580?.[0]?.datavalue?.value.time ?? "";
  return claims
    .filter((c) => c.rank !== "deprecated" && !c.qualifiers?.P582 && c.mainsnak.datavalue)
    .sort((a, b) => (b.rank === "preferred") - (a.rank === "preferred") || start(b).localeCompare(start(a)))
    .map((c) => c.mainsnak.datavalue.value);
}
