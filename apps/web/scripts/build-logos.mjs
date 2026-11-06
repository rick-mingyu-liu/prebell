// Regenerates the bundled logos for every watchlist ticker (both tiers; a rerun picks up new rows).
// Company cards are wide (about 128x48), so a horizontal wordmark is preferred. Sources, in order:
//   1. Wikidata (the SEC CIK in watchlist.tsv -> item -> P154 logo) -> Wikimedia Commons, freely
//      licensed files only                                        -> companies/{TICKER}.png
//   2. COMMONS_FALLBACK: hand-picked Commons files for a few tickers whose Wikidata logo is unusable
//   3. the pinned `simple-icons` devDependency (CC0 glyphs, symbol only) -> companies/{TICKER}.svg
//   4. Wikidata P8972 small logo (a square icon), Commons again    -> companies/{TICKER}.png
//   5. the company's own site icon (apple-touch-icon / icon link), last resort -> companies/{TICKER}.png
// A file is rejected when blank, wider than MAX_WORDMARK:1, or a raster too small to look sharp.
// plus the partner marks in PARTNERS. It writes:
//   public/logos/companies/*, public/logos/partners/*
//   src/lib/logos.generated.ts            ticker -> file and ticker -> natural size, read by src/lib/logos.ts
//   public/logos/LICENSES.md              source, date and licence for every file
// Company PNGs are 2x: a wordmark fits 256x96 px (128x48 CSS px) with no padding or tile, so it sits on
// the card's own background; a full-bleed app icon is a 96px rounded square; a mark too pale for a white
// card is set on a dark pill. The natural CSS size of each file is the manifest's COMPANY_LOGO_SIZES.
// Everything is built in node_modules/.cache/logos-build and moved into place only when the whole run
// succeeds, so a failed run leaves the committed files alone. Definitive answers (2xx, and 4xx other
// than 408/429) are cached in node_modules/.cache/logos, so a run that stopped (a Wikimedia timeout,
// say) resumes where it was on the next run; delete the cache to refetch. Requests go out at most two a
// second to Wikimedia and one a second elsewhere, with a User-Agent naming the project and
// support@prebell.site, per the Wikimedia policy. Run `pnpm logos` in apps/web after changing a mapping
// below, bumping simple-icons or adding watchlist rows. See docs/runbook.md.
//   --offline       use only the cache (no requests)
//   --allow-shrink  accept a manifest that drops tickers the committed one has
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const require = createRequire(import.meta.url);
const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const WATCHLIST = join(WEB, "../../crates/core/data/watchlist.tsv");
const CACHE = join(WEB, "node_modules/.cache/logos");
const BUILD = join(WEB, "node_modules/.cache/logos-build");
const LOGOS = join(WEB, "public/logos");
const MANIFEST = join(WEB, "src/lib/logos.generated.ts");
const OFFLINE = process.argv.includes("--offline");
const ALLOW_SHRINK = process.argv.includes("--allow-shrink");
const SI_ROOT = join(dirname(require.resolve("simple-icons/icons.json")), "..");
const SI_VERSION = JSON.parse(readFileSync(join(SI_ROOT, "package.json"), "utf8")).version;
const SI_DATA = require("simple-icons/icons.json");
/** The day the pinned simple-icons version was installed; bump together with the package. */
const FETCHED = "2026-10-30";
const UA = "PreBell logo build (https://prebell.site; support@prebell.site)";

/**
 * Watchlist ticker -> simple-icons slug. Only the company's own corporate mark (or, where noted,
 * the consumer brand the company trades as). Everything else goes to Wikidata, then the site icon.
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

/** Tickers kept as a text chip on purpose, with the reason (shown in LICENSES.md). */
const SKIP = {
  AVGO: "Broadcom licenses its logo only on request (logorequest.broadcom.com)",
  VMRK: "renamed from Equity Residential; Wikidata still holds the old company's logo",
};

/** Tickers whose site icon is not the company's mark (it would be wrong, not just small): Commons and simple-icons only. */
const NO_SITE_ICON = {
  IBM: "its site icon is a grey bee-like glyph that does not read as IBM",
  MDT: "its site icon is a figure graphic, not the Medtronic mark",
  MAR: "its site icon is an orange ring that does not read as Marriott",
  IEX: "its site icon is a blurry raster",
};

/**
 * Tickers whose Wikidata/Commons logo is the wrong company, a subsidiary, a product or an old brand (found
 * in review): no Commons file is used; simple-icons, then the site icon, then a text tile.
 */
const NO_COMMONS = {
  BIDU: "Commons only has a generic icon-set paw, not Baidu's own file",
  BILI: "Commons names Bilibili Gaming, an esports team",
  BKNG: "its Commons logo is a cramped stacked lockup; the simple-icons glyph reads better",
  "BRK.B": "its Commons logo is Berkshire Hathaway Energy, a subsidiary",
  BRO: "Commons search finds Brown Bears Athletics",
  DOW: "Commons search finds Martin Dow, a different company",
  EIX: "Commons search finds Edison State Community College",
  GRMN: "its Commons logo is the Garmin-ASUS joint brand",
  HIG: "Commons search finds a 1968 railroad logo",
  HONA: "its Commons logo is the 1946 Minneapolis-Honeywell one",
  JCI: "its Commons logo is an old, blurred one",
  JNJ: "Commons search finds a presidential campaign logo",
  KR: "its Commons logo is the 1961-2019 one",
  MGM: "Commons search finds a casino-tower logo, not MGM Resorts",
  PGR: "Commons names a political party with the same initials",
  PRU: "Commons names Prudential do Brasil",
  TRI: "Commons names the Thomson Reuters Foundation, a charity",
  UNH: "its Commons logo is UnitedHealthcare, a subsidiary",
};

/** Wikidata items for tickers whose watchlist CIK has no Wikidata match (new holding companies, foreign issuers). */
const WIKIDATA = { BLK: "Q219635", XOM: "Q156238", NVO: "Q818846", SAN: "Q6496310" };

/** Commons files used instead of the Wikidata logo (which is wrong or the wrong shape); nothing else from Commons is tried. */
const COMMONS = {
  BLK: "BlackRock wordmark.svg", // Wikidata names Aladdin, a product
  GD: "General Dynamics logo 2020.svg", // Wikidata names General Dynamics Mission Systems
  MS: "Morgan Stanley Logo 2024.svg", // Wikidata names MSCI's old name
  ORCL: "Oracle logo.svg", // Commons search finds Oracle Solaris
  ADSK: "Autodesk logo.svg", // Wikidata names Autodesk Maya, a product
  AXON: "AXON Company logo.svg",
  CTSH: "Cognizant logo 2022.svg", // Commons search found a photo of an office
  EMR: "EmersonElectricLogo.png", // its Wikidata logo is the 1945 one
  FERG: "Ferguson plc logo.svg", // Commons search found Massey Ferguson
  GLW: "Corning Incorporated Logo.svg", // Commons search found SUNY Corning Community College
  IR: "Ingersoll Rand logo.svg", // its Wikidata logo is from 1925
  KDP: "Keurig Dr Pepper.svg",
  LUV: "Southwest Airlines logo 2014.svg", // its Wikidata logo is the 1971 one
  TPR: "Tapestry logo.svg", // its Wikidata logo is Coach, one of its brands
  TT: "TraneTechnologieslogo.svg", // its Wikidata logo is Ingersoll Rand, the old name
};

/** Commons files tried when the company's Wikidata logo is missing, not free or unusable (before simple-icons). */
const COMMONS_FALLBACK = {
  CVS: "CVS Health heart.png", // the CVS Health heart, as on its app icons
};

/**
 * Logos wider than MAX_WORDMARK (after trimming) are unreadable strips on a 128px card; below that the
 * wordmark is used as it is. A raster wordmark must be at least MIN_WIDE_RASTER px wide, any other raster
 * at least MIN_RASTER px on its short side; vectors have no minimum.
 */
const MAX_WORDMARK = 6.5;
/** A wordmark wider than MAX_WORDMARK but not wider than this is used only when nothing else works (it is small on a card). */
const MAX_WIDE_LAST_RESORT = 9;
const MIN_WIDE_RASTER = 120;
const MIN_RASTER = 64;
/** CSS size of the slot a logo fits (files are 2x): width, height. */
const SLOT = [128, 48];

/**
 * Partner marks, each a square tile shown beside the partner's name. `slug` is a simple-icons glyph;
 * `url` is the partner's own file, linked from `page` (none of them publishes brand terms).
 */
const PARTNERS = {
  okx: { slug: "okx" },
  moomoo: {
    url: "https://cdn.futustatic.com/moomoo_common/dist/img/mm_logo-3fce2..png",
    page: "https://www.moomoo.com/us/newsroom",
    what: "moomoo app icon, from moomoo's newsroom page",
  },
  wealthsimple: {
    url: "https://cdn.uc.assets.prezly.com/a7b77202-a167-4518-a92d-0a5fd2792f44/Wealthsimple_Brandmark_SoftBlack_Small.svg",
    page: "https://newsroom.wealthsimple.com/media",
    what: "Wealthsimple brandmark, from Wealthsimple's newsroom media page",
  },
  cryptocom: {
    url: "https://mkt-site-asset.crypto.com/assets/logo/main-app-default.svg",
    page: "https://crypto.com/eea/company-news",
    what: "Crypto.com app icon, from crypto.com",
  },
};

// ---------------------------------------------------------------------------------------------
// Network: cached, one request per second.

let lastRequest = 0;
const today = () => new Date().toISOString().slice(0, 10);

/** A 2xx, or a 4xx a retry won't change (not 408 timeout or 429 rate limit): safe to cache. */
const definitive = (status) => (status >= 200 && status < 300) || (status >= 400 && status < 500 && status !== 408 && status !== 429);
/** Company-site URLs that failed in a way a retry could fix; reported at the end, never cached. */
const retryable = [];

/**
 * GET with an on-disk cache. Returns { body, type, url, fetched }, or null. Only definitive answers
 * are cached. A network error, timeout, 408, 429 or 5xx is not: for a `required` source (Wikidata,
 * Commons, a partner file) it stops the run, since a missing answer would silently drop logos; for a
 * company site it is reported, and the shrink check at the end catches any logo it cost.
 */
async function get(url, required = false) {
  mkdirSync(CACHE, { recursive: true });
  const key = join(CACHE, createHash("sha1").update(url).digest("hex"));
  const cached = existsSync(`${key}.json`) && JSON.parse(readFileSync(`${key}.json`, "utf8"));
  // Older caches also hold transient failures (no status, or a retryable one): those don't count.
  if (cached && definitive(cached.status ?? (cached.ok ? 200 : 0))) return cached.ok ? { ...cached, body: readFileSync(key) } : null;
  if (retryable.includes(url)) return null; // already failed this run
  let why = "not cached (--offline)";
  if (!OFFLINE) {
    await new Promise((r) => setTimeout(r, Math.max(0, lastRequest + (/(^|\.)(wikimedia|wikidata|wikipedia)\.org$/.test(new URL(url).hostname) ? 500 : 1000) - Date.now())));
    lastRequest = Date.now();
    try {
      const res = await fetch(url, { headers: { "User-Agent": UA }, signal: AbortSignal.timeout(20_000) });
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

const getJson = async (url) => {
  const res = await get(url, true);
  if (!res) throw new Error(`${url}: no answer`);
  return JSON.parse(res.body.toString("utf8"));
};

// ---------------------------------------------------------------------------------------------
// Wikidata and Commons.

/** Ticker -> Wikidata item, matched on the SEC CIK (P5531, stored zero-padded to 10 digits). */
async function wikidataItems(rows) {
  const byCik = {};
  for (let i = 0; i < rows.length; i += 80) {
    const ciks = rows.slice(i, i + 80).map((r) => `"${r.cik.padStart(10, "0")}"`).join(" ");
    const query = `SELECT ?cik ?item WHERE { VALUES ?cik { ${ciks} } ?item wdt:P5531 ?cik. }`;
    const res = await getJson(`https://query.wikidata.org/sparql?format=json&query=${encodeURIComponent(query)}`);
    for (const b of res.results.bindings) byCik[b.cik.value] ??= b.item.value.split("/").pop();
  }
  return Object.fromEntries(rows.map((r) => [r.ticker, WIKIDATA[r.ticker] ?? byCik[r.cik.padStart(10, "0")]]));
}

/** Current values of a property: not deprecated, no end date, preferred rank and newest start first. */
function current(claims = []) {
  const start = (c) => c.qualifiers?.P580?.[0]?.datavalue?.value.time ?? "";
  return claims
    .filter((c) => c.rank !== "deprecated" && !c.qualifiers?.P582 && c.mainsnak.datavalue)
    .sort((a, b) => (b.rank === "preferred") - (a.rank === "preferred") || start(b).localeCompare(start(a)))
    .map((c) => c.mainsnak.datavalue.value);
}

/** Item -> { icons, logos, sites } from wbgetentities, 50 items a request. */
async function entities(ids) {
  const out = {};
  for (let i = 0; i < ids.length; i += 50) {
    const batch = ids.slice(i, i + 50).join("|");
    const res = await getJson(`https://www.wikidata.org/w/api.php?action=wbgetentities&format=json&props=claims&ids=${batch}`);
    for (const [id, e] of Object.entries(res.entities))
      out[id] = { icons: current(e.claims?.P8972), logos: current(e.claims?.P154), sites: current(e.claims?.P856) };
  }
  return out;
}

const plain = (html = "") => html.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();
/** Free licences accepted from Commons: public domain (incl. PD-textlogo, PD-shape), CC0, CC BY, CC BY-SA. */
const FREE = /^(public domain|pd\b|cc0|cc by(-sa)? \d)/i;

/** A Commons licence tag that says why a file is free, e.g. PD-textlogo or PD-shape (not the layout helpers). */
const TAG = /^Template:(PD-(?!Layout|generic)[^/]+|CC-zero)$/i;

/**
 * Commons file name -> { url, page, licence, licenceUrl, artist, attribution, tags }, 50 files a
 * request. `tags` are the file's licence templates, following the API's continuation.
 */
async function commonsInfo(names) {
  const out = {};
  for (let i = 0; i < names.length; i += 50) {
    const batch = names.slice(i, i + 50);
    const titles = encodeURIComponent(batch.map((n) => `File:${n}`).join("|"));
    const res = await getJson(
      "https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo&iiprop=url|size|mime|extmetadata&iiurlwidth=800" +
        `&iiextmetadatafilter=LicenseShortName|LicenseUrl|Artist|Attribution&titles=${titles}`,
    );
    const tags = {};
    for (let more = ""; more !== null; ) {
      const t = await getJson(
        `https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=templates&tlnamespace=10&tllimit=max&titles=${titles}${more}`,
      );
      for (const page of Object.values(t.query.pages))
        (tags[page.title] ??= []).push(...(page.templates ?? []).map((x) => x.title).filter((x) => TAG.test(x)).map((x) => x.slice(9)));
      more = t.continue ? `&${new URLSearchParams(t.continue)}` : null;
    }
    const title = Object.fromEntries(batch.map((n) => [`File:${n}`, n]));
    for (const { from, to } of res.query.normalized ?? []) title[to] = title[from];
    for (const page of Object.values(res.query.pages)) {
      const info = page.imageinfo?.[0];
      const name = title[page.title];
      if (!info || !name) continue;
      const m = info.extmetadata ?? {};
      out[name] = {
        // A raster wider than 800px is fetched as an 800px thumbnail (a 2x wordmark never needs more).
        url: info.mime !== "image/svg+xml" && info.width > 800 && info.thumburl ? info.thumburl : info.url,
        page: info.descriptionurl,
        licence: plain(m.LicenseShortName?.value),
        licenceUrl: m.LicenseUrl?.value,
        artist: plain(m.Artist?.value),
        attribution: plain(m.Attribution?.value),
        tags: tags[page.title] ?? [],
      };
    }
  }
  return out;
}

// ---------------------------------------------------------------------------------------------
// Site icons.

/** The page's icon links, largest first, then /apple-touch-icon.png and /favicon.ico. */
function iconLinks(html, base) {
  const out = [];
  for (const tag of html.match(/<link\b[^>]*>/gi) ?? []) {
    const attr = (n) => tag.match(new RegExp(`\\b${n}\\s*=\\s*["']([^"']*)["']`, "i"))?.[1];
    const rel = attr("rel")?.toLowerCase() ?? "";
    const href = attr("href");
    if (!href || !/(^|\s)(icon|apple-touch-icon(-precomposed)?)(\s|$)/.test(rel)) continue;
    let url;
    try {
      url = new URL(href.replace(/&amp;/g, "&"), base).href;
    } catch {
      continue;
    }
    const sizes = (attr("sizes") ?? "").match(/\d+/g)?.map(Number) ?? [];
    const size = /\.svg(\?|$)/i.test(url) ? 1000 : sizes.length ? Math.max(...sizes) : rel.includes("apple") ? 180 : 0;
    out.push({ url, size });
  }
  out.sort((a, b) => b.size - a.size);
  out.push({ url: new URL("/apple-touch-icon.png", base).href }, { url: new URL("/favicon.ico", base).href });
  return out.filter((c, i) => out.findIndex((d) => d.url === c.url) === i);
}

/** The largest image in a .ico file: a PNG entry as is, or a 32-bit BMP entry converted to PNG. Null for anything else. */
async function fromIco(buf) {
  let best = null;
  for (let i = 0; i < buf.readUInt16LE(4); i++) {
    const e = 6 + i * 16;
    const width = buf[e] || 256;
    if (!best || width > best.width) best = { width, data: buf.subarray(buf.readUInt32LE(e + 12), buf.readUInt32LE(e + 12) + buf.readUInt32LE(e + 8)) };
  }
  const d = best?.data;
  if (!d?.length) return null;
  if (d.readUInt32BE(0) === 0x89504e47) return d;
  // A BITMAPINFOHEADER, then bottom-up BGRA rows; the height counts the AND mask too, so it is doubled.
  const [header, w, h, bpp] = [d.readUInt32LE(0), d.readInt32LE(4), d.readInt32LE(8) / 2, d.readUInt16LE(14)];
  if (bpp !== 32) return null;
  const px = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const s = header + ((h - 1 - y) * w + x) * 4;
      px.set([d[s + 2], d[s + 1], d[s], d[s + 3]], (y * w + x) * 4);
    }
  return sharp(px, { raw: { width: w, height: h, channels: 4 } }).png().toBuffer();
}

/** The largest usable icon (an SVG, or at least 48px) on the company's homepage. */
async function siteIcon(homepages) {
  for (const home of homepages) {
    const page = await get(home);
    if (!page || !/html/i.test(page.type)) continue;
    for (const { url } of iconLinks(page.body.toString("utf8"), page.url)) {
      const file = await get(url);
      const art = file && (await artwork(file.body).catch((e) => console.warn(`  ${url}: ${e.message}`)));
      if (art && !art.blank && (art.svg || (Math.min(art.srcWidth, art.srcHeight) >= 48 && Math.min(art.width, art.height) >= MIN_RASTER)))
        return { art, url: file.url, home: page.url, fetched: file.fetched };
    }
  }
  return null;
}

// ---------------------------------------------------------------------------------------------
// Rendering.

const luminance = (hex) => {
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const toHex = (r, g, b) => [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");
/** Too pale to read on white (Caterpillar and McDonald's yellow are below 1.6:1). */
const pale = (hex) => 1.05 / (luminance(hex) + 0.05) < 1.6;
const TILE = { light: ["#fff", "#e5e5e5"], dark: ["#111", "#333"] };

/**
 * A 32px rounded tile with the 24px glyph inset 4px, so one file reads on the light and the dark
 * page. Brand colours too pale for white go on a near-black tile.
 */
function tile({ path, hex }) {
  const [bg, edge] = TILE[pale(hex) ? "dark" : "light"];
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">` +
    `<rect x=".5" y=".5" width="31" height="31" rx="7" fill="${bg}" stroke="${edge}"/>` +
    `<path transform="translate(4 4)" fill="#${hex}" d="${path}"/></svg>\n`
  );
}

/**
 * Any logo file (SVG or raster) -> a trimmed RGBA PNG plus what the tile needs: its aspect, whether
 * it is a full-bleed app icon (square, and the same non-white colour all round its edge), whether it is
 * too pale for a white tile, and whether it is blank (empty, or one flat square: a placeholder).
 */
async function artwork(file) {
  const ico = file.length > 6 && file.readUInt32BE(0) === 0x00000100;
  const buf = ico ? await fromIco(file) : file;
  if (!buf) throw new Error("no PNG or 32-bit image in the .ico");
  const meta = await sharp(buf).metadata();
  const svg = meta.format === "svg";
  const density = svg ? Math.min(2400, Math.max(1, (72 * 512) / Math.max(meta.width, meta.height))) : undefined;
  let img = sharp(buf, svg ? { density } : {}).ensureAlpha();
  if (!svg && Math.max(meta.width, meta.height) > 512) img = img.resize(512, 512, { fit: "inside" });
  const flat = await img.png().toBuffer();
  const trimmed = await sharp(flat)
    .trim({ threshold: 12 })
    .png()
    .toBuffer()
    .catch(() => flat);
  const { data, info } = await sharp(trimmed).raw().toBuffer({ resolveWithObject: true });
  const px = (fx, fy) => {
    const i = (Math.floor(fy * info.height) * info.width + Math.floor(fx * info.width)) * 4;
    return [...data.subarray(i, i + 4)];
  };
  // Near each corner and at the middle of each edge, so a glyph like Honeywell's "H" doesn't count.
  const edge = [
    ...[0.08, 0.92].flatMap((fx) => [0.08, 0.92].map((fy) => px(fx, fy))),
    ...[px(0.5, 0.03), px(0.5, 0.97), px(0.03, 0.5), px(0.97, 0.5)],
  ];
  const fullBleed =
    Math.abs(info.width / info.height - 1) < 0.08 &&
    edge.every((p) => p[3] > 240 && p.slice(0, 3).every((v, i) => Math.abs(v - edge[0][i]) < 24)) &&
    luminance(toHex(...edge[0].slice(0, 3))) < 0.85;
  let opaque = 0;
  let light = 0;
  const colours = new Set();
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 128) continue;
    opaque++;
    colours.add(((data[i] >> 4) << 8) | ((data[i + 1] >> 4) << 4) | (data[i + 2] >> 4));
    if (pale(toHex(data[i], data[i + 1], data[i + 2]))) light++;
  }
  return {
    png: trimmed,
    svg,
    srcWidth: meta.width,
    srcHeight: meta.height,
    width: info.width,
    height: info.height,
    aspect: info.width / info.height,
    fullBleed,
    pale: !fullBleed && light > opaque * 0.75,
    blank: opaque === 0 || (colours.size < 2 && opaque > 0.9 * info.width * info.height),
  };
}

/**
 * The raster twin of tile(): a 64px PNG. A full-bleed app icon fills the tile, clipped to its
 * rounded corners; anything else sits 8px in on a white (or, when pale, near-black) tile.
 */
async function pngTile(art) {
  const S = 64;
  const svg = (body) => Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${S}" height="${S}">${body}</svg>`);
  let out;
  if (art.fullBleed) {
    out = sharp(art.png)
      .resize(S, S, { fit: "cover" })
      .composite([{ input: svg(`<rect width="${S}" height="${S}" rx="14"/>`), blend: "dest-in" }]);
  } else {
    const [bg, edge] = TILE[art.pale ? "dark" : "light"];
    const back = svg(`<rect x="1" y="1" width="${S - 2}" height="${S - 2}" rx="13" fill="${bg}" stroke="${edge}" stroke-width="2"/>`);
    // A wide logo may use more of the width (4px in, not 8) so it isn't smaller than it has to be.
    const glyph = await sharp(art.png)
      .resize(art.aspect > 1.5 ? 56 : 48, 48, { fit: "inside" })
      .png()
      .toBuffer();
    out = sharp(back).composite([{ input: glyph, gravity: "centre" }]);
  }
  return out.png({ palette: true, quality: 95, effort: 10, compressionLevel: 9 }).toBuffer();
}

/**
 * A company mark -> { png, w, h }: the 2x PNG and its natural CSS size. A wordmark (or any mark that is
 * not full-bleed) is trimmed and fitted inside SLOT with no tile, so the card's background shows around
 * it; a full-bleed app icon is a rounded square; a mark too pale for white is set on a dark pill.
 */
async function renderMark(art) {
  const [sw, sh] = SLOT.map((v) => v * 2);
  const out = (img) => img.png({ palette: true, quality: 90, effort: 10, compressionLevel: 9 }).toBuffer({ resolveWithObject: true });
  const done = ({ data, info }) => ({ png: data, w: Math.max(1, Math.round(info.width / 2)), h: Math.max(1, Math.round(info.height / 2)) });
  const svg = (w, h, body) => Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}">${body}</svg>`);
  if (art.fullBleed) {
    const S = sh;
    return done(await out(sharp(art.png).resize(S, S, { fit: "cover" }).composite([{ input: svg(S, S, `<rect width="${S}" height="${S}" rx="${S / 4.5}"/>`), blend: "dest-in" }])));
  }
  const pad = art.pale ? 16 : 0;
  const glyph = await sharp(art.png)
    .resize(sw - 2 * pad, sh - 2 * pad, { fit: "inside", withoutEnlargement: true })
    .png()
    .toBuffer({ resolveWithObject: true });
  if (!art.pale) return done(await out(sharp(glyph.data)));
  const [w, h] = [glyph.info.width + 2 * pad, glyph.info.height + 2 * pad];
  const back = svg(w, h, `<rect width="${w}" height="${h}" rx="${pad}" fill="#111"/>`);
  return done(await out(sharp(back).composite([{ input: glyph.data, gravity: "centre" }])));
}

const icon = (slug) => {
  const data = SI_DATA.find((i) => i.slug === slug);
  if (!data) throw new Error(`simple-icons ${SI_VERSION} has no "${slug}"`);
  const svg = readFileSync(require.resolve(`simple-icons/icons/${slug}.svg`), "utf8");
  const path = svg.match(/<path d="([^"]+)"/)?.[1];
  if (!path) throw new Error(`no path in ${slug}.svg`);
  return { ...data, path: path.replace(/\s+/g, " ") };
};

// ---------------------------------------------------------------------------------------------
// Build.

const watchRows = readFileSync(WATCHLIST, "utf8")
  .split("\n")
  .filter((l) => l.trim() && !l.startsWith("#"))
  .map((l) => {
    const [ticker, cik, , name, domains = "", , tier] = l.split("\t").map((s) => s.trim());
    return { ticker, cik, name, domains: domains.split(",").filter(Boolean), tier };
  });
const tickers = watchRows.map((w) => w.ticker);
for (const t of [...Object.keys(COMPANIES), ...Object.keys(SKIP), ...Object.keys(NO_SITE_ICON), ...Object.keys(NO_COMMONS), ...Object.keys(COMMONS), ...Object.keys(WIKIDATA), ...Object.keys(COMMONS_FALLBACK)])
  if (!tickers.includes(t)) console.warn(`  ${t} is in a mapping but not on the watchlist (ignored)`);

// Built here, moved into public/logos and src/lib at the very end.
rmSync(BUILD, { recursive: true, force: true });
const companiesDir = join(BUILD, "companies");
const partnersDir = join(BUILD, "partners");
for (const dir of [companiesDir, partnersDir]) mkdirSync(dir, { recursive: true });

const rows = [];
const row = (file, mark, source, fetched, licence) => rows.push(`| \`${file}\` | ${mark} | ${source} | ${fetched} | ${licence} |`);
const siLicence = (i) =>
  i.license ? `${i.license.type}${i.license.url ? ` (${i.license.url})` : ""}` : "CC0-1.0 (Simple Icons)";
const siSource = (i) =>
  `simple-icons@${SI_VERSION} \`${i.slug}\` (upstream source: ${i.source}; brand guidelines: ${i.guidelines ?? "none listed"})`;
const tier = { commons: [], hand: [], simpleIcons: [], icon: [], search: [], wide: [], site: [] };
/** Why each ticker's rejected candidates lost, printed for the tickers that end up without a logo. */
const why = {};

const manifest = {};
const sizes = {};
const todo = watchRows.filter((w) => !(w.ticker in SKIP));
const items = await wikidataItems(todo);
const claims = await entities([...new Set(Object.values(items).filter(Boolean))]);
const commons = await commonsInfo([
  ...new Set([...Object.values(COMMONS), ...Object.values(COMMONS_FALLBACK), ...Object.values(claims).flatMap((c) => [...c.logos, ...c.icons])]),
]);

/** Null when the artwork is fit to show, else the reason it is not. */
function reject(art) {
  if (art.blank) return "blank";
  if (art.aspect > MAX_WORDMARK) return `too wide (${art.aspect.toFixed(1)}:1)`;
  if (art.aspect < 0.5) return `too tall (1:${(1 / art.aspect).toFixed(1)})`;
  if (!art.svg && art.aspect > 1.5 && art.width < MIN_WIDE_RASTER) return `low-res (${art.width}px wide)`;
  if (!art.svg && art.aspect <= 1.5 && Math.min(art.width, art.height) < MIN_RASTER) return `low-res (${art.width}x${art.height})`;
  return null;
}

/** Commons file names that look like the company's logo: "<first word of its name> ... logo", best match first. */
async function searchLogos(w) {
  // The title must hold the company's whole core name ("brownbrown"), or a distinctive first word (7+ letters).
  const words = w.name.replace(/^the\s+/i, "").split(/[\s,.&-]+/).map((x) => x.toLowerCase().replace(/[^a-z0-9]/g, "")).filter(Boolean);
  const core = words.filter((x) => !/^(inc|corp|corporation|company|co|group|holdings|plc|ltd|limited|and)$/.test(x));
  const full = core.join("");
  const first = core[0] ?? "";
  if (full.length < 3) return [];
  const q = encodeURIComponent(`${w.name} logo`);
  const res = await getJson(`https://commons.wikimedia.org/w/api.php?action=query&format=json&list=search&srnamespace=6&srlimit=8&srsearch=${q}`);
  return res.query.search
    .map((r) => r.title.replace(/^File:/, ""))
    .filter((n) => /\.(svg|png)$/i.test(n) && /logo/i.test(n))
    .filter((n) => {
      const t = n.toLowerCase().replace(/[^a-z0-9]/g, "");
      return t.includes(full) || (first.length >= 7 && t.includes(first));
    })
    .filter((n) => !/icon|favicon|history|old|building|store|brand guide|president|campaign|college|university|school|party|athletics|foundation|church|product/i.test(n));
}

for (const w of todo) {
  const qid = items[w.ticker];
  const c = claims[qid] ?? { icons: [], logos: [], sites: [] };
  const notes = (why[w.ticker] = []);
  /** A free Commons file that passes reject(), as a pick; null otherwise (the reason is noted). */
  const fromCommons = async (prop, name) => {
    const info = commons[name];
    if (!info) return notes.push(`${name}: not on Commons`), null;
    if (!FREE.test(info.licence)) return notes.push(`${name}: ${info.licence || "no licence"}`), null;
    const file = await get(info.url, true);
    const art = file && (await artwork(file.body).catch((e) => notes.push(`${name}: ${e.message}`)));
    const bad = art && reject(art);
    const found = art && { kind: "commons", art, info, name, prop, fetched: file.fetched };
    if (bad && art.aspect > MAX_WORDMARK && art.aspect <= MAX_WIDE_LAST_RESORT && !reject({ ...art, aspect: MAX_WORDMARK })) wide ??= found;
    if (!art || bad) return bad && notes.push(`${name}: ${bad}`), null;
    return found;
  };
  let wide = null;
  const fromSearch = async (w) => {
    const names = await searchLogos(w);
    Object.assign(commons, await commonsInfo(names.filter((n) => !commons[n])));
    for (const name of names) {
      const found = await fromCommons("search", name);
      if (found) return found;
    }
    return null;
  };
  let pick = null;
  const noCommons = w.ticker in NO_COMMONS;
  if (COMMONS[w.ticker]) pick = await fromCommons("hand", COMMONS[w.ticker]);
  else if (!noCommons) for (const name of c.logos) if ((pick = await fromCommons("P154", name))) break;
  if (!pick && !noCommons && COMMONS_FALLBACK[w.ticker]) pick = await fromCommons("hand", COMMONS_FALLBACK[w.ticker]);
  if (!pick && w.ticker in COMPANIES) pick = { kind: "simple-icons", icon: icon(COMPANIES[w.ticker]) };
  if (!pick && !noCommons && !COMMONS[w.ticker]) for (const name of c.icons) if ((pick = await fromCommons("P8972", name))) break;
  if (!pick && !noCommons && !COMMONS[w.ticker]) pick = await fromSearch(w);
  if (!pick && !(w.ticker in NO_SITE_ICON)) {
    const site = await siteIcon([...new Set([...w.domains.map((d) => `https://${d}/`), ...c.sites])]);
    if (site) pick = { kind: "site", ...site };
    else notes.push("no site icon (none linked, blocked, or under 48px)");
  }
  if (!pick && wide) {
    pick = { ...wide, wide: true };
    tier.wide.push(w.ticker);
  }
  if (!pick) continue;
  let file;
  if (pick.kind === "simple-icons") {
    const i = pick.icon;
    file = `${w.ticker}.svg`;
    // A glyph is drawn bare on the card; one too pale for white goes on the dark tile.
    const svg = pale(i.hex)
      ? tile(i).replace("<svg ", '<svg width="48" height="48" ')
      : `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="48" height="48"><path fill="#${i.hex}" d="${i.path}"/></svg>\n`;
    writeFileSync(join(companiesDir, file), svg);
    sizes[w.ticker] = [48, 48];
    row(`companies/${file}`, i.title, siSource(i), FETCHED, siLicence(i));
    tier.simpleIcons.push(w.ticker);
  } else {
    file = `${w.ticker}.png`;
    const m = await renderMark(pick.art);
    writeFileSync(join(companiesDir, file), m.png);
    sizes[w.ticker] = [m.w, m.h];
    if (pick.kind === "commons") {
      const { info } = pick;
      const by = /^cc by/i.test(info.licence) ? `; by ${info.attribution || info.artist || "see the file page"}` : "";
      row(
        `companies/${file}`,
        w.name,
        `Wikimedia Commons [${pick.name}](${info.page}) (${pick.prop === "hand" ? "chosen by hand in the script" : pick.prop === "search" ? "found by searching Commons for the company name" : `Wikidata [${qid}](https://www.wikidata.org/wiki/${qid}) ${pick.prop}`})`,
        pick.fetched,
        `${info.licence}${/^public domain/i.test(info.licence) && info.tags.length ? ` (${info.tags.join(", ")})` : ""}` +
          `${info.licenceUrl ? ` (${info.licenceUrl})` : ""}${by}`,
      );
      (pick.prop === "P154" ? tier.commons : pick.prop === "hand" ? tier.hand : pick.prop === "search" ? tier.search : tier.icon).push(w.ticker);
    } else {
      row(`companies/${file}`, w.name, `site icon ${pick.url} (linked from ${pick.home})`, pick.fetched, "site icon, trademark of owner, nominative use");
      tier.site.push(w.ticker);
    }
  }
  manifest[w.ticker] = `/logos/companies/${file}`;
}

for (const [id, p] of Object.entries(PARTNERS)) {
  if (p.slug) {
    const i = icon(p.slug);
    writeFileSync(join(partnersDir, `${id}.svg`), tile(i));
    row(`partners/${id}.svg`, i.title, siSource(i), FETCHED, siLicence(i));
    continue;
  }
  const file = await get(p.url, true);
  if (!file) throw new Error(`partner ${id}: could not fetch ${p.url}`);
  writeFileSync(join(partnersDir, `${id}.png`), await pngTile(await artwork(file.body)));
  row(
    `partners/${id}.png`,
    p.what,
    `${p.url} (linked from ${p.page})`,
    file.fetched,
    "partner's own asset, trademark of owner; no published brand terms found",
  );
}

writeFileSync(
  join(BUILD, "logos.generated.ts"),
  `// Generated by scripts/build-logos.mjs. Do not edit; run \`pnpm logos\`.\n` +
    `export const COMPANY_LOGOS: Readonly<Record<string, string>> = ${JSON.stringify(manifest, null, 2)};\n` +
    `/** Natural size of each file in CSS px (the file itself is 2x): [width, height]. */\n` +
    `export const COMPANY_LOGO_SIZES: Readonly<Record<string, readonly [number, number]>> = ${JSON.stringify(sizes)};\n`,
);

const missing = tickers.filter((t) => !(t in manifest));
const bytes = [companiesDir, partnersDir]
  .flatMap((d) => readdirSync(d).map((f) => readFileSync(join(d, f)).length))
  .reduce((a, b) => a + b, 0);
writeFileSync(
  join(BUILD, "LICENSES.md"),
  `# Logo sources and licences

Generated by \`apps/web/scripts/build-logos.mjs\`; edit the script, not this file. Every file below is
bundled with the site; pages never load a logo from a third party.

Sources, in order of preference per ticker (company cards are wide, so a horizontal wordmark comes first):

1. **Wikimedia Commons via Wikidata** (\`.png\`, ${tier.commons.length} tickers by the logo property P154, ${tier.hand.length} chosen
   by hand in the script, ${tier.icon.length} by the small-logo property P8972): the logo file named on the
   company's Wikidata item (matched on its SEC CIK). Only files Commons lists as public domain (mostly
   PD-textlogo), CC0, CC BY or CC BY-SA are used, and CC BY rows name the author. The file is trimmed and
   fitted inside 128x48 CSS px (stored at 2x) with no other change to its shape. Logos wider than
   ${MAX_WORDMARK}:1 and rasters under ${MIN_WIDE_RASTER}px wide are not used.
2. **Simple Icons** (\`.svg\` glyphs, ${tier.simpleIcons.length} tickers): the npm package, pinned at
   simple-icons@${SI_VERSION}, for companies with no usable Commons logo. Its icon data is
   [CC0-1.0](https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md); its
   [disclaimer](https://github.com/simple-icons/simple-icons/blob/develop/DISCLAIMER.md) notes that a
   per-icon licence, where one is listed, takes precedence. The script only sets the glyph's colour (on a
   dark tile when the brand colour is too pale for white); it never changes the shape.
3. **The company's own site icon** (\`.png\`, ${tier.site.length} tickers), last resort: the largest
   \`apple-touch-icon\` / \`icon\` linked from the company's homepage, fetched once at build time.

**Copyright vs trademark.** A free licence covers copyright in the drawing. Every logo remains a
trademark of the company named. PreBell shows company logos only to identify the company a news item,
earnings date or price move is about (nominative use), never to suggest endorsement or a partnership.
Partner marks appear next to affiliate links, which carry the affiliate disclosure. They come from each
partner's own site or newsroom; none of the partners publishes brand terms for them.

| File | Mark | Source | Fetched | Licence |
| --- | --- | --- | --- | --- |
${rows.join("\n")}

Watchlist tickers without a logo (text chip): ${missing.map((t) => (SKIP[t] ? `${t} (${SKIP[t]})` : t)).join(", ") || "none"}.

Total size of the files above: ${bytes} bytes.
`,
);

// A regeneration must not quietly lose logos (a site that was down, a file renamed upstream).
const committed = existsSync(MANIFEST) ? JSON.parse(readFileSync(MANIFEST, "utf8").match(/COMPANY_LOGOS[^=]*= (\{[\s\S]*?\n\});/)[1]) : {};
const lost = Object.keys(committed).filter((t) => !(t in manifest));
if (lost.length && !ALLOW_SHRINK)
  throw new Error(`these tickers would lose their logo: ${lost.join(" ")}. Nothing was written; pass --allow-shrink if that is intended.`);

for (const dir of ["companies", "partners"]) {
  rmSync(join(LOGOS, dir), { recursive: true, force: true });
  renameSync(join(BUILD, dir), join(LOGOS, dir));
}
renameSync(join(BUILD, "LICENSES.md"), join(LOGOS, "LICENSES.md"));
renameSync(join(BUILD, "logos.generated.ts"), MANIFEST);

console.log(
  `${Object.keys(manifest).length} of ${tickers.length} tickers have a logo (Commons P154 ${tier.commons.length}, Commons by hand ${tier.hand.length}, ` +
    `simple-icons ${tier.simpleIcons.length}, Commons P8972 ${tier.icon.length}, Commons search ${tier.search.length}, site icon ${tier.site.length}); ${bytes} bytes`,
);
console.log(`Very wide, last resort (${tier.wide.length}): ${tier.wide.join(" ")}\nCommons search (${tier.search.length}): ${tier.search.join(" ")}\nSite icon (${tier.site.length}): ${tier.site.join(" ")}`);
console.log(`Without (${missing.length}):\n${missing.map((t) => `  ${t}: ${SKIP[t] ?? (why[t] ?? []).join("; ") ?? ""}`).join("\n")}`);
if (lost.length) console.warn(`Dropped (--allow-shrink): ${lost.join(" ")}`);
if (retryable.length)
  console.warn(`${retryable.length} company-site requests failed in a way a retry could fix (not cached); run again to retry them.`);
