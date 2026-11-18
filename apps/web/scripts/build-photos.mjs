// Builds the company photos shown under the opening of an earnings check and at the top of a ticker page,
// from the pinned list in data/photos.tsv (ticker, exact Wikimedia Commons file name, English caption,
// Chinese caption). Nothing else ships: `--candidates` only proposes files (Wikidata's P18 "image" for each
// watchlist company) for a person to choose from and pin.
//
// For each pinned file the build fetches the Commons metadata fresh (Artist, LicenseShortName, LicenseUrl,
// Credit, Restrictions, DateTimeOriginal, ImageDescription) and stops when a licence is no longer public
// domain, CC0, CC BY or CC BY-SA, when Restrictions holds anything but a personality-rights or trademark
// note, when the author cannot be reduced to a plain name, when the image is taller than 4:5 or wider than
// 2.4:1, when a caption's year is not the year the photo was taken, or when a caption says "headquarters"
// (总部) and Wikidata's current headquarters location (P159) is not named in it. Images are only resized
// (the long side to at most MAX_SIDE px, never upscaled) and re-encoded as WebP; never cropped. It writes:
//   public/photos/{TICKER}.webp
//   src/lib/photos.generated.ts         ticker -> file, size, captions and credit, read by src/lib/photos.ts
//   public/photos/LICENSES.md           source, author, licence and change for every file
//   node_modules/.cache/photos/review.html   a review sheet (not committed): every photo as it will appear
// Every manifest URL ends in `?v=` and the file's content hash, because /photos/* is served immutable for a
// year. Everything is built in node_modules/.cache/photos-build and moved into place only when the whole run
// succeeds. Downloads are cached in node_modules/.cache/photos (licence metadata is refetched every run), at
// most two requests a second to Wikimedia, with the same kind of User-Agent as the logo build.
// Run `pnpm photos` in apps/web after changing data/photos.tsv. See docs/runbook.md, "Photos".
//   --offline     use only the cache (no requests; the licence check then uses the cached metadata)
//   --candidates  write node_modules/.cache/photos/candidates.{json,tsv} from Wikidata P18; builds nothing
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import { WIKIDATA_ITEMS, createFetcher, current, hashOf, plain, readWatchlist, wikidataItems } from "./wikimedia.mjs";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const WATCHLIST = join(WEB, "../../crates/core/data/watchlist.tsv");
const PINNED = join(WEB, "data/photos.tsv");
const CACHE = join(WEB, "node_modules/.cache/photos");
const BUILD = join(WEB, "node_modules/.cache/photos-build");
const PHOTOS = join(WEB, "public/photos");
const MANIFEST = join(WEB, "src/lib/photos.generated.ts");
const REVIEW = join(CACHE, "review.html");
const OFFLINE = process.argv.includes("--offline");
const CANDIDATES = process.argv.includes("--candidates");
const UA = "PreBell photo build (https://prebell.site; support@prebell.site)";

/** Long side of a stored photo, in px (an 680px column at 2x, roughly); smaller originals are kept as they are. */
const MAX_SIDE = 1440;
/** WebP quality: a typical 1440px photo comes out at about 140 KB (median of the first 100: 137 KB). Shown at 680 CSS px or less, so 2x on most screens. */
const QUALITY = 50;
/** Narrowest original worth showing across the column. */
const MIN_WIDTH = 1200;
/** Tallest and widest shapes shown uncropped: 4:5 and 2.4:1. */
const MIN_ASPECT = 4 / 5;
const MAX_ASPECT = 2.4;
/** The photos folder may not grow past this (Workers static assets: 25 MiB a file, 20,000 files a version on Free). */
const MAX_TOTAL = 40_000_000;
/** Commons asks for one of its standard thumbnail widths; the build resizes from this one. */
const THUMB = 1920;

/**
 * Licences accepted: public domain, CC0, CC BY and CC BY-SA, any version or port (e.g. "CC BY-SA 3.0 de").
 * Anchored, so BY-NC, BY-ND, GFDL and "Attribution" alone are refused.
 */
const LICENCE = /^(public domain|cc0( 1\.0)?|cc by(-sa)? \d\.\d( [a-z]{2}(-[a-z]{2})?)?)$/i;
/** Restrictions values that note a right in what is shown (a person in the frame, a trademark on a building), not a limit on reuse of the photo. */
const NOTED = new Set(["personality", "trademarked"]);

const { get, getJson } = createFetcher({ cache: CACHE, ua: UA, offline: OFFLINE });
const rows = readWatchlist(WATCHLIST);
const byTicker = Object.fromEntries(rows.map((r) => [r.ticker, r]));

// ---------------------------------------------------------------------------------------------
// Commons and Wikidata.

/** HTML entities in Commons metadata -> characters (after plain() has removed the tags). */
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
const decode = (s) =>
  s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) =>
    e[0] === "#" ? String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : (ENTITIES[e.toLowerCase()] ?? m),
  );
const text = (html) => decode(plain(html ?? "")).replace(/\s+/g, " ").trim();

/**
 * The Artist field as a plain name (a "User:" prefix dropped), or null when it is not one: empty, unknown or
 * anonymous, a URL, an @ (an email address, or a name with an affiliation such as "X @ Project / Team"), markup,
 * the uploader rather than the author ("The original uploader was ..."), or a sentence.
 */
function authorName(html) {
  const name = text(html)
    .replace(/^User:/i, "")
    .replace(/\s*\((talk|contribs?)\)$/i, "");
  if (!name || name.length > 60) return null;
  if (/[<>{}\[\]|@]|https?:|www\.|\bunknown\b|\banonymous\b|\bown work\b|\buploader\b|\bemployee\b|\bnone\b|^(user|author)$/i.test(name)) return null;
  return name;
}

const MONTHS = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
/**
 * The year a photo was taken, from DateTimeOriginal: "2022:05:01 10:00:00", "2022-05-01", "2009-08",
 * "Taken on 8 June 2016, 11:08:51", "28 October 2013" or "14 July 2021 (according to Exif data)". Null when
 * the field is empty or not a clean date (an upload date, "circa", a range, free text).
 */
function takenYear(raw) {
  const t = text(raw)
    .replace(/^taken on /i, "")
    .replace(/ \(according to Exif data\)$/i, "");
  const iso = t.match(/^(\d{4})([-:]\d{2}([-:]\d{2})?)?([ T]\d{1,2}:\d{2}(:\d{2})?)?$/);
  const long = t.match(/^(\d{1,2}) ([A-Za-z]+) (\d{4})(,? \d{1,2}:\d{2}(:\d{2})?)?$/);
  const year = iso ? Number(iso[1]) : long && MONTHS.includes(long[2].toLowerCase()) ? Number(long[3]) : null;
  return year && year > 1820 && year <= new Date().getFullYear() ? year : null;
}

/** Commons file name -> its imageinfo and metadata, 50 files a request. `fresh` skips the cache (licences can change). */
async function commonsInfo(names, fresh) {
  const out = {};
  for (let i = 0; i < names.length; i += 50) {
    const batch = names.slice(i, i + 50);
    const titles = encodeURIComponent(batch.map((n) => `File:${n}`).join("|"));
    const res = await getJson(
      "https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo&iiprop=url|size|mime|extmetadata" +
        `&iiurlwidth=${THUMB}&iiextmetadatafilter=Artist|LicenseShortName|LicenseUrl|Credit|Restrictions|DateTimeOriginal|ImageDescription|Categories|Attribution&titles=${titles}`,
      { fresh },
    );
    const title = Object.fromEntries(batch.map((n) => [`File:${n}`, n]));
    for (const { from, to } of res.query.normalized ?? []) title[to] = title[from];
    for (const page of Object.values(res.query.pages)) {
      const info = page.imageinfo?.[0];
      const name = title[page.title];
      if (!info || !name) continue;
      const m = info.extmetadata ?? {};
      out[name] = {
        // A thumbnail at THUMB px when the original is wider; the original otherwise.
        url: info.thumburl && info.thumbwidth < info.width ? info.thumburl : info.url,
        page: info.descriptionurl,
        width: info.width,
        height: info.height,
        mime: info.mime,
        licence: text(m.LicenseShortName?.value),
        licenceUrl: m.LicenseUrl?.value ?? null,
        artist: m.Artist?.value ?? "",
        // The attribution the author asks for, when it is a plain name (not a credit line with a © or a licence); else the Artist field.
        author: (/©|\/|\bcc[ -]|licen[cs]e/i.test(text(m.Attribution?.value)) ? null : authorName(m.Attribution?.value)) ?? authorName(m.Artist?.value),
        credit: text(m.Credit?.value),
        attribution: text(m.Attribution?.value),
        restrictions: text(m.Restrictions?.value).split("|").map((s) => s.trim()).filter(Boolean),
        taken: text(m.DateTimeOriginal?.value),
        year: takenYear(m.DateTimeOriginal?.value),
        description: text(m.ImageDescription?.value),
        categories: text(m.Categories?.value).split("|").filter(Boolean),
      };
    }
  }
  return out;
}

/** Item -> { images, hq, category, label } from wbgetentities, 50 items a request. */
async function entities(ids, fresh) {
  const out = {};
  for (let i = 0; i < ids.length; i += 50) {
    const res = await getJson(
      `https://www.wikidata.org/w/api.php?action=wbgetentities&format=json&props=claims|labels&languages=en|zh&ids=${ids.slice(i, i + 50).join("|")}`,
      { fresh },
    );
    for (const [id, e] of Object.entries(res.entities))
      out[id] = {
        images: current(e.claims?.P18),
        hq: current(e.claims?.P159).map((v) => v.id).filter(Boolean),
        within: current(e.claims?.P131).map((v) => v.id).filter(Boolean),
        category: current(e.claims?.P373)[0] ?? null,
        label: e.labels?.en?.value ?? null,
        zh: e.labels?.zh?.value ?? null,
      };
  }
  return out;
}

/**
 * Ticker -> the names of its current headquarters location (P159): the place's own English label and the
 * label of the place it lies in (P131), so "Apple Park" also matches "Cupertino".
 */
async function headquarters(tickers, fresh) {
  const items = await wikidataItems(rows, getJson, WIKIDATA_ITEMS);
  const qids = [...new Set(tickers.map((t) => items[t]).filter(Boolean))];
  const companies = await entities(qids, fresh);
  const places = await entities([...new Set(Object.values(companies).flatMap((c) => c.hq))], fresh);
  const parents = await entities([...new Set(Object.values(places).flatMap((p) => p.within))], fresh);
  const out = {};
  for (const t of tickers) {
    const c = companies[items[t]];
    out[t] = {
      qid: items[t] ?? null,
      company: c ?? null,
      names: (c?.hq ?? []).flatMap((h) => [places[h]?.label, ...(places[h]?.within ?? []).map((p) => parents[p]?.label)]).filter(Boolean),
    };
  }
  return out;
}

// ---------------------------------------------------------------------------------------------
// Candidates: Wikidata P18 for every watchlist company, with what a reviewer needs to decide.

if (CANDIDATES) {
  const hq = await headquarters(
    rows.map((r) => r.ticker),
    false,
  );
  const files = [...new Set(Object.values(hq).flatMap((h) => h.company?.images ?? []))];
  const info = await commonsInfo(files, false);
  const out = [];
  for (const r of rows) {
    for (const file of hq[r.ticker].company?.images ?? []) {
      const i = info[file];
      if (!i) continue;
      const aspect = i.width / i.height;
      const problems = [
        !LICENCE.test(i.licence) && `licence ${i.licence || "none"}`,
        i.restrictions.some((x) => !NOTED.has(x)) && `restrictions ${i.restrictions.join(",")}`,
        !i.author && `author "${text(i.artist).slice(0, 40)}"`,
        !/^image\/(jpeg|png)$/.test(i.mime) && i.mime,
        i.width < MIN_WIDTH && `${i.width}px wide`,
        (aspect < MIN_ASPECT || aspect > MAX_ASPECT) && `aspect ${aspect.toFixed(2)}`,
      ].filter(Boolean);
      out.push({ ticker: r.ticker, name: r.name, tier: r.tier, qid: hq[r.ticker].qid, hq: hq[r.ticker].names, file, ...i, aspect: +aspect.toFixed(2), problems });
    }
  }
  writeFileSync(join(CACHE, "candidates.json"), JSON.stringify(out, null, 1));
  const cell = (s) => String(s ?? "").replace(/\s+/g, " ").slice(0, 300);
  writeFileSync(
    join(CACHE, "candidates.tsv"),
    ["ticker\ttier\tname\tfile\tok\tproblems\tlicence\tauthor\ttaken\tsize\thq\tdescription\tcategories"]
      .concat(
        out.map((c) =>
          [c.ticker, c.tier, c.name, c.file, c.problems.length ? "no" : "yes", c.problems.join("; "), c.licence, c.author, c.taken, `${c.width}x${c.height}`, c.hq.join(" / "), cell(c.description), cell(c.categories.join(" | "))].join("\t"),
        ),
      )
      .join("\n") + "\n",
  );
  const ok = out.filter((c) => !c.problems.length);
  console.log(`${out.length} P18 files for ${new Set(out.map((c) => c.ticker)).size} of ${rows.length} watchlist companies; ${ok.length} pass the mechanical checks.`);
  console.log(`Wrote ${join(CACHE, "candidates.tsv")} and candidates.json. Nothing ships unless it is pinned in data/photos.tsv.`);
  process.exit(0);
}

// ---------------------------------------------------------------------------------------------
// Build.

/** data/photos.tsv: ticker, Commons file name, English caption, Chinese caption; # starts a comment line. */
const pinned = readFileSync(PINNED, "utf8")
  .split("\n")
  .map((l, i) => [l, i + 1])
  .filter(([l]) => l.trim() && !l.startsWith("#"))
  .map(([l, line]) => {
    const cols = l.split("\t").map((s) => s.trim());
    if (cols.length !== 4 || cols.some((c) => !c)) throw new Error(`data/photos.tsv line ${line}: expected 4 non-empty tab-separated columns`);
    const [ticker, file, en, zh] = cols;
    if (!byTicker[ticker]) throw new Error(`data/photos.tsv line ${line}: ${ticker} is not on the watchlist`);
    return { ticker, file: file.replace(/^File:/, "").replace(/_/g, " "), en, zh, line };
  });
const dupes = pinned.map((p) => p.ticker).filter((t, i, a) => a.indexOf(t) !== i);
if (dupes.length) throw new Error(`data/photos.tsv pins these tickers twice: ${dupes.join(" ")}`);

const info = await commonsInfo([...new Set(pinned.map((p) => p.file))], !OFFLINE);
const hq = await headquarters(
  pinned.map((p) => p.ticker),
  !OFFLINE,
);

/** Every reason a pinned photo may not ship; the build stops if any is found. */
const errors = [];
const notes = [];
const fail = (p, why) => errors.push(`${p.ticker} (${p.file}): ${why}`);
for (const p of pinned) {
  const i = info[p.file];
  if (!i) {
    fail(p, "not on Commons (renamed or deleted?)");
    continue;
  }
  if (!LICENCE.test(i.licence)) fail(p, `licence "${i.licence}" is not public domain, CC0, CC BY or CC BY-SA`);
  if (/^cc by/i.test(i.licence) && !/^https?:\/\/creativecommons\.org\//.test(i.licenceUrl ?? "")) fail(p, `a CC licence whose licence URL is not on creativecommons.org (${i.licenceUrl ?? "none"})`);
  const bad = i.restrictions.filter((x) => !NOTED.has(x));
  if (bad.length) fail(p, `Restrictions: ${bad.join(", ")}`);
  if (i.restrictions.length) notes.push(`${p.ticker}: Restrictions ${i.restrictions.join(", ")} (noted in LICENSES.md)`);
  if (!i.author) fail(p, `the author "${text(i.artist)}" is not a plain name`);
  if (i.attribution && i.attribution !== i.author) notes.push(`${p.ticker}: Commons asks for the attribution "${i.attribution}"; the credit gives "${i.author}"`);
  if (!/^image\/(jpeg|png)$/.test(i.mime)) fail(p, `${i.mime} is not a JPEG or PNG`);
  if (i.width < MIN_WIDTH) fail(p, `${i.width}px wide, under ${MIN_WIDTH}`);
  // A year in a caption is the year the photo was taken, so it must match DateTimeOriginal.
  for (const [lang, caption] of [["en", p.en], ["zh", p.zh]]) {
    const years = [...caption.matchAll(/(?<!\d)(1[89]\d\d|20\d\d)(?!\d)/g)].map((m) => Number(m[1]));
    if (years.some((y) => y !== i.year)) fail(p, `the ${lang} caption names ${years.join(", ")} but the photo was taken ${i.year ?? "on an unknown date"}`);
  }
  if (/\b(19|20)\d\d\b/.test(p.en) !== /(19|20)\d\d/.test(p.zh)) fail(p, "one caption gives the year and the other does not");
  // "Headquarters" only when Wikidata's current headquarters location is named in the caption.
  const saysHq = /headquarters/i.test(p.en);
  if (saysHq !== /总部/.test(p.zh)) fail(p, "one caption says headquarters (总部) and the other does not");
  if (saysHq && !hq[p.ticker].names.some((n) => p.en.includes(n.replace(/,.*$/, ""))))
    fail(p, `the caption says headquarters, but Wikidata's current headquarters location (${hq[p.ticker].names.join(" / ") || "none"}) is not in it`);
}

rmSync(BUILD, { recursive: true, force: true });
mkdirSync(BUILD, { recursive: true });
const manifest = {};
const built = [];
for (const p of pinned) {
  const i = info[p.file];
  if (!i || errors.some((e) => e.startsWith(`${p.ticker} `))) continue;
  const file = await get(i.url, true);
  if (!file) throw new Error(`${p.ticker}: could not fetch ${i.url}`);
  // The file is untrusted: only sharp reads it, and only a JPEG or PNG is accepted.
  const meta = await sharp(file.body).metadata();
  if (meta.format !== "jpeg" && meta.format !== "png") {
    fail(p, `the downloaded file is ${meta.format}, not a JPEG or PNG`);
    continue;
  }
  const { data, info: out } = await sharp(file.body)
    .rotate() // apply the EXIF orientation, so the shape below is the shape shown
    .resize(MAX_SIDE, MAX_SIDE, { fit: "inside", withoutEnlargement: true })
    .webp({ quality: QUALITY, effort: 6 })
    .toBuffer({ resolveWithObject: true });
  const aspect = out.width / out.height;
  if (aspect < MIN_ASPECT || aspect > MAX_ASPECT) {
    fail(p, `aspect ${aspect.toFixed(2)}:1 is outside 4:5 to 2.4:1`);
    continue;
  }
  writeFileSync(join(BUILD, `${p.ticker}.webp`), data);
  manifest[p.ticker] = {
    src: `/photos/${p.ticker}.webp?v=${hashOf(data)}`,
    width: out.width,
    height: out.height,
    en: p.en,
    zh: p.zh,
    author: i.author,
    licence: /^public domain$/i.test(i.licence) ? "Public domain" : i.licence,
    licenceUrl: i.licenceUrl,
    page: i.page,
  };
  built.push({ p, i, bytes: data.length, data, fetched: file.fetched, out });
}
if (errors.length) throw new Error(`Nothing was written. Fix or remove these rows in data/photos.tsv:\n  ${errors.join("\n  ")}`);

const total = built.reduce((a, b) => a + b.bytes, 0);
if (total > MAX_TOTAL) throw new Error(`the photos come to ${total} bytes, over ${MAX_TOTAL}. Nothing was written.`);

const md = (s) => String(s).replace(/\|/g, "\\|").replace(/\s+/g, " ");
const licences = {};
for (const b of built) licences[manifest[b.p.ticker].licence] = (licences[manifest[b.p.ticker].licence] ?? 0) + 1;
writeFileSync(
  join(BUILD, "LICENSES.md"),
  `# Photo sources and licences

Generated by \`apps/web/scripts/build-photos.mjs\` from \`apps/web/data/photos.tsv\`; edit those, not this
file. These photographs are not covered by the repository's MIT licence: each belongs to its author and is
used under the licence named below, which is also shown with the author's name under the photo on every
page that shows it. Pages never load a photo from Wikimedia.

Every file comes from Wikimedia Commons, chosen by hand. Only public domain, CC0, CC BY and CC BY-SA files are
used. **Changes:** each photo is resized (the long side to at most ${MAX_SIDE} px, never enlarged) and
re-encoded as WebP; it is not cropped or otherwise altered. Where the licence is CC BY-SA, the resized
file is shared under the same licence.

Licences: ${Object.entries(licences)
    .sort((a, b) => b[1] - a[1])
    .map(([l, n]) => `${l} ${n}`)
    .join(", ")}.

| File | Company | Commons file | Author | Licence | Taken | Fetched | Changes | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
${built
  .map(
    ({ p, i, fetched, out }) =>
      `| \`${p.ticker}.webp\` | ${md(byTicker[p.ticker].name)} | [${md(p.file)}](${i.page}) | ${md(i.author)} | ${md(manifest[p.ticker].licence)}${i.licenceUrl ? ` (${i.licenceUrl})` : ""} | ${md(i.taken || "unknown")} | ${fetched} | resized to ${out.width}x${out.height}, WebP | ${md(i.restrictions.join(", "))} |`,
  )
  .join("\n")}

Total size of the files above: ${total} bytes.
`,
);

writeFileSync(
  join(BUILD, "photos.generated.ts"),
  `// Generated by scripts/build-photos.mjs from data/photos.tsv. Do not edit; run \`pnpm photos\`.\n` +
    `/** Ticker -> its photo: file (with a \`?v=\` content hash), size in px, captions, and the credit (author, licence, Commons page). */\n` +
    `export const COMPANY_PHOTOS: Readonly<Record<string, { src: string; width: number; height: number; en: string; zh: string; author: string; licence: string; licenceUrl: string | null; page: string }>> = ${JSON.stringify(manifest, null, 2)};\n`,
);

rmSync(PHOTOS, { recursive: true, force: true });
renameSync(BUILD, PHOTOS);
renameSync(join(PHOTOS, "photos.generated.ts"), MANIFEST);

// The review sheet: every photo as it will appear, with the Commons description beside it.
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const credit = (m, zh) => `${zh ? "图片" : "Photo"}: ${m.author} / Wikimedia Commons · ${m.licence === "Public domain" ? (zh ? "公有领域" : "Public domain") : m.licence}`;
const cards = built
  .map(({ p, i, data, out, bytes }) => {
    const m = manifest[p.ticker];
    return `<section class="card" id="${esc(p.ticker)}">
<div class="photo"><figure><img src="data:image/webp;base64,${data.toString("base64")}" width="${out.width}" height="${out.height}" alt="${esc(p.en)}">
<figcaption>${esc(p.en)} <span class="credit">${esc(credit(m, false))}</span></figcaption></figure>
<p class="zh">${esc(p.zh)} <span class="credit">${esc(credit(m, true))}</span></p></div>
<div class="source"><label class="reject"><input type="checkbox" value="${esc(p.ticker)}"> Reject ${esc(p.ticker)}</label>
<h2>${esc(p.ticker)} · ${esc(byTicker[p.ticker].name)}</h2>
<dl><dt>Commons file</dt><dd><a href="${esc(i.page)}">${esc(p.file)}</a></dd>
<dt>Commons description</dt><dd>${esc(i.description) || "<em>none</em>"}</dd>
<dt>Taken</dt><dd>${esc(i.taken || "unknown")}</dd>
<dt>Wikidata headquarters (P159)</dt><dd>${esc(hq[p.ticker].names.join(" / ") || "none")}</dd>
<dt>Categories</dt><dd>${esc(i.categories.join(" · "))}</dd>
<dt>Licence</dt><dd>${esc(i.licence)}${i.restrictions.length ? ` · Restrictions: ${esc(i.restrictions.join(", "))}` : ""}</dd>
<dt>File</dt><dd>${out.width}x${out.height}, ${Math.round(bytes / 1024)} KB</dd></dl></div></section>`;
  })
  .join("\n");
writeFileSync(
  REVIEW,
  `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>PreBell photos: review</title><style>
body{margin:0;padding:16px;background:#faf8f3;color:#1a1a1a;font:15px/1.6 system-ui,sans-serif}
header{max-width:1200px;margin:0 auto 1rem}h1{font:600 26px Georgia,serif;margin:0 0 .3rem}
.card{display:grid;grid-template-columns:minmax(0,680px) minmax(0,1fr);gap:24px;max-width:1200px;margin:0 auto;padding:20px 0;border-top:1px solid #d9d5cc}
figure{margin:0}img{display:block;width:100%;height:auto;max-height:560px;object-fit:contain;object-position:left}
figcaption,.zh{margin:.4rem 0 0;color:#666;font-size:13px;line-height:1.5}.credit{display:block}
h2{font:600 18px Georgia,serif;margin:.3rem 0}dl{margin:0;font-size:13px}dt{font-weight:600;color:#666;margin-top:.5rem}dd{margin:0;overflow-wrap:anywhere}
.reject{display:inline-block;padding:4px 10px;border:1px solid #b00;color:#b00;font-weight:600;cursor:pointer}
.card:has(input:checked){background:#fbe9e9}
#out{width:100%;max-width:1200px;height:4em;font:14px ui-monospace,monospace}
@media (max-width:760px){.card{grid-template-columns:1fr}}
</style></head><body><header><h1>PreBell company photos: review</h1>
<p>${built.length} photos from <code>apps/web/data/photos.tsv</code>, ${Math.round(total / 1024)} KB in all. Each is shown as on the site: the photo, the caption and the credit (English, then Chinese), with the Commons description and date beside it. Tick <b>Reject</b> on any photo that is wrong, then press the button and paste the list back.</p>
<p><button id="go" type="button">List the rejected tickers</button></p><textarea id="out" readonly placeholder="Rejected tickers appear here"></textarea></header>
${cards}
<script>document.getElementById("go").onclick=function(){var t=[].map.call(document.querySelectorAll(".reject input:checked"),function(x){return x.value});var o=document.getElementById("out");o.value=t.length?"Rejected photos: "+t.join(" "):"No photos rejected.";o.focus();o.select();};</script>
</body></html>
`,
);

console.log(`${built.length} photos, ${total} bytes (${Math.round(total / built.length / 1024)} KB each on average); licences: ${JSON.stringify(licences)}`);
const sizes = built.map((b) => b.bytes).sort((a, b) => a - b);
console.log(`File sizes: smallest ${sizes[0]}, median ${sizes[Math.floor(sizes.length / 2)]}, largest ${sizes.at(-1)} bytes`);
if (notes.length) console.log(`Notes:\n  ${notes.join("\n  ")}`);
console.log(`Review sheet: ${REVIEW}`);
