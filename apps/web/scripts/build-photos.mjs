// Builds the company photos shown under the opening of an earnings check and at the top of a ticker page,
// from the pinned list in data/photos.tsv. Nothing else ships: `--candidates` only proposes files (Wikidata's
// P18 "image" for each watchlist company) for a person to choose from and pin.
//
// data/photos.tsv, tab-separated, `#` starts a comment line:
//   ticker, exact Commons file name, English caption, Chinese caption,
//   [credit]  the name as the credit shows it, when Commons asks for a form other than the author's name
//             ("© Raimond Spekking"); otherwise the author's name is used,
//   [terms]   Commons' Permission text, word for word, when it holds a condition or request beyond the licence:
//             repeating it records that someone read it, and the build stops when it changes on Commons.
//
// For each pinned file the build fetches the Commons metadata fresh (Artist, Attribution, Permission, Credit,
// LicenseShortName, LicenseUrl, Restrictions, DateTimeOriginal, ImageDescription, the file's own EXIF and its
// wikitext) and stops when: the licence is no longer public domain, CC0, CC BY or CC BY-SA, or its URL or the
// Commons page is not https; Restrictions holds anything but a personality-rights or trademark note; the author
// is not a plain name; a requested attribution would not be shown in full; Permission names another licence
// than the licence tag, or holds a condition the terms column does not repeat (only Commons' exact template
// text is skipped); the EXIF Artist or Copyright names someone other than the credited author or their
// Commons account (a company name counts; only placeholders, editing software and the camera's own make in
// Artist do not); the
// image is not a JPEG or PNG at least MIN_WIDTH wide, or is taller than 4:5 or wider than 2.4:1; a caption's
// year is not the year the photo was taken; or a caption says "headquarters" (总部) without naming Wikidata's
// headquarters location (P159). The rules are pure functions in scripts/photo-rules.mjs, tested in
// tests/photorules.test.ts. Images are only resized (the long side to at most MAX_SIDE px, never upscaled) and
// re-encoded as WebP with the credit in their EXIF; never cropped. It writes:
//   public/photos/{TICKER}.webp
//   src/lib/photos.generated.ts         ticker -> file, size, captions, credit and source, read by src/lib/photos.ts
//                                       (the credit line under each photo and the /{locale}/photo-credits page)
//   public/photos/LICENSES.md           source, author, requested notice, licence and change for every file
//   node_modules/.cache/photos/review.html   a review sheet (not committed): every photo as it will appear
// Every manifest URL ends in `?v=` and the file's content hash, because /photos/* is served immutable for a
// year. Everything is built in node_modules/.cache/photos-build and swapped into place only when the whole run
// succeeds. Downloads are cached in node_modules/.cache/photos (metadata is refetched every run), at most two
// requests a second to Wikimedia, with the same kind of User-Agent as the logo build.
// Run `pnpm photos` in apps/web after changing data/photos.tsv. See docs/runbook.md, "Photos".
//   --offline     use only the cache (no requests; the checks then use the cached metadata)
//   --candidates  write node_modules/.cache/photos/candidates.{json,tsv} from Wikidata P18; builds nothing
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import {
  authorName,
  captionYears,
  commonsPage,
  exifMismatch,
  exifName,
  headquartersNamed,
  licenceAllowed,
  licenceUrl,
  originalSource,
  permissionTerms,
  requestNotCovered,
  stripFlickrPlace,
  takenYear,
  text,
  userNames,
} from "./photo-rules.mjs";
import { WIKIDATA_ITEMS, createFetcher, current, hashOf, readWatchlist, wikidataItems } from "./wikimedia.mjs";

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
/** WebP quality: a typical 1440px photo comes out at about 140 KB. Shown at 680 CSS px or less, so 2x on most screens. */
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
/** Restrictions values that note a right in what is shown (a person in the frame, a trademark on a building), not a limit on reuse of the photo. */
const NOTED = new Set(["personality", "trademarked"]);

const { get, getJson } = createFetcher({ cache: CACHE, ua: UA, offline: OFFLINE });
const rows = readWatchlist(WATCHLIST);
const byTicker = Object.fromEntries(rows.map((r) => [r.ticker, r]));
/** Author names whose Flickr " from <place>" suffix was dropped, printed so they can be checked. */
const stripped = [];

// ---------------------------------------------------------------------------------------------
// Commons and Wikidata.

/** An EXIF tag's value from Commons' commonmetadata list (values may be nested lists). */
const exifTag = (list, name) => {
  const hit = (list ?? []).find((x) => x.name === name)?.value;
  // A multilingual value is a list of { name: language, value } plus a { name: "_type", value: "lang" } marker.
  return Array.isArray(hit) ? hit.filter((x) => x?.name !== "_type").map((x) => x?.value ?? x).join("; ") : (hit ?? "");
};

/** The other Commons file this one was cropped from: {{Extracted from|...}} in its wikitext. */
const extractedFrom = (wikitext) => wikitext.match(/\{\{\s*Extracted from\s*\|\s*(?:1\s*=\s*)?([^|}]+)/i)?.[1]?.trim().replace(/^File:/i, "") ?? null;

/** Commons file name -> its imageinfo, metadata, EXIF and wikitext, 50 files a request. `fresh` skips the cache (licences can change). */
async function commonsInfo(names, fresh) {
  const out = {};
  for (let i = 0; i < names.length; i += 50) {
    const batch = names.slice(i, i + 50);
    const titles = encodeURIComponent(batch.map((n) => `File:${n}`).join("|"));
    const res = await getJson(
      "https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo&iiprop=url|size|mime|extmetadata|commonmetadata" +
        `&iiurlwidth=${THUMB}&iiextmetadatafilter=Artist|Attribution|Permission|Credit|LicenseShortName|LicenseUrl|Restrictions|DateTimeOriginal|ImageDescription|Categories&titles=${titles}`,
      { fresh },
    );
    const wiki = await getJson(
      `https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2&prop=revisions&rvprop=content&rvslots=main&titles=${titles}`,
      { fresh },
    );
    const wikitext = Object.fromEntries(wiki.query.pages.map((pg) => [pg.title, pg.revisions?.[0]?.slots?.main?.content ?? ""]));
    const title = Object.fromEntries(batch.map((n) => [`File:${n}`, n]));
    for (const { from, to } of res.query.normalized ?? []) title[to] = title[from];
    for (const page of Object.values(res.query.pages)) {
      const info = page.imageinfo?.[0];
      const name = title[page.title];
      if (!info || !name) continue;
      const m = info.extmetadata ?? {};
      const source = originalSource(m.Credit?.value);
      const permission = permissionTerms(m.Permission?.value, text(m.LicenseShortName?.value));
      const attribution = text(m.Attribution?.value);
      // The credited name: the attribution the author asks for when it is a plain name (not a credit line with
      // a ©, a licence or a URL), else the Artist field; the Flickr importer's " from <place>" is dropped.
      const plainAttribution = /©|\/|\bcc[ -]|licen[cs]e/i.test(attribution) ? null : authorName(m.Attribution?.value);
      const raw = plainAttribution ?? authorName(m.Artist?.value);
      const author = raw && stripFlickrPlace(raw, source?.flickr);
      if (author && author !== raw) stripped.push(`${name}: "${raw}" -> "${author}"`);
      out[name] = {
        // A thumbnail at THUMB px when the original is wider; the original otherwise.
        url: info.thumburl && info.thumbwidth < info.width ? info.thumburl : info.url,
        page: commonsPage(info.descriptionurl),
        pageRaw: info.descriptionurl,
        width: info.width,
        height: info.height,
        mime: info.mime,
        licence: text(m.LicenseShortName?.value),
        licenceUrl: licenceUrl(m.LicenseUrl?.value),
        licenceUrlRaw: m.LicenseUrl?.value ?? null,
        artist: text(m.Artist?.value),
        // The Commons or Wikipedia accounts the Artist field links to: the same person as the credited name.
        users: userNames(m.Artist?.value),
        author,
        // What the author asks the credit to say: the Attribution field, and an "Attribution:" in the licence box.
        requests: [...new Set([attribution, permission.attribution].filter(Boolean))],
        conditions: permission.conditions,
        conflict: permission.conflict,
        permission: text(m.Permission?.value),
        source,
        exifArtist: text(exifTag(info.commonmetadata, "Artist")),
        exifMake: text(exifTag(info.commonmetadata, "Make")),
        exifCopyright: text(exifTag(info.commonmetadata, "Copyright")),
        derivedFrom: extractedFrom(wikitext[page.title] ?? ""),
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
        !licenceAllowed(i.licence) && `licence ${i.licence || "none"}`,
        i.restrictions.some((x) => !NOTED.has(x)) && `restrictions ${i.restrictions.join(",")}`,
        !i.author && `author "${i.artist.slice(0, 40)}"`,
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

/** data/photos.tsv: ticker, Commons file name, English caption, Chinese caption, [credit], [terms]. */
const pinned = readFileSync(PINNED, "utf8")
  .split("\n")
  .map((l, i) => [l, i + 1])
  .filter(([l]) => l.trim() && !l.startsWith("#"))
  .map(([l, line]) => {
    const cols = l.split("\t").map((s) => s.trim());
    if (cols.length < 4 || cols.length > 6 || cols.slice(0, 4).some((c) => !c))
      throw new Error(`data/photos.tsv line ${line}: expected ticker, file, English caption, Chinese caption, then an optional credit and terms`);
    const [ticker, file, en, zh, credit = "", terms = ""] = cols;
    if (!byTicker[ticker]) throw new Error(`data/photos.tsv line ${line}: ${ticker} is not on the watchlist`);
    return { ticker, file: file.replace(/^File:/, "").replace(/_/g, " "), en, zh, credit, terms, line };
  });
const dupes = pinned.map((p) => p.ticker).filter((t, i, a) => a.indexOf(t) !== i);
if (dupes.length) throw new Error(`data/photos.tsv pins these tickers twice: ${dupes.join(" ")}`);

const info = await commonsInfo([...new Set(pinned.map((p) => p.file))], !OFFLINE);
const hq = await headquarters(
  pinned.map((p) => p.ticker),
  !OFFLINE,
);

/** The licence as the credit names it. */
const licenceName = (i) => (/^public domain$/i.test(i.licence) ? "Public domain" : i.licence);

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
  if (!i.page) fail(p, `the Commons page is not an https commons.wikimedia.org/wiki/File: URL (${i.pageRaw})`);
  if (!licenceAllowed(i.licence)) fail(p, `licence "${i.licence}" is not public domain, CC0, CC BY or CC BY-SA`);
  if (/^cc/i.test(i.licence) && !i.licenceUrl) fail(p, `a CC licence whose URL is not on creativecommons.org (${i.licenceUrlRaw ?? "none"})`);
  if (i.licenceUrlRaw && !i.licenceUrl) fail(p, `the licence URL ${i.licenceUrlRaw} is not on creativecommons.org`);
  const bad = i.restrictions.filter((x) => !NOTED.has(x));
  if (bad.length) fail(p, `Restrictions: ${bad.join(", ")}`);
  if (i.restrictions.length) notes.push(`${p.ticker}: Restrictions ${i.restrictions.join(", ")} (noted in LICENSES.md)`);
  if (!i.author) fail(p, `the author "${i.artist}" is not a plain name`);
  if (!/^image\/(jpeg|png)$/.test(i.mime)) fail(p, `${i.mime} is not a JPEG or PNG`);
  if (i.width < MIN_WIDTH) fail(p, `${i.width}px wide, under ${MIN_WIDTH}`);
  // The credit: the author's name, or the credit column when Commons asks for another form. The column may only
  // use words from what Commons gives (a request or the author's name), so it never invents a credit.
  const name = p.credit || i.author || "";
  if (p.credit) {
    const allowed = [...i.requests, i.author ?? "", i.artist].join(" ");
    const extra = requestNotCovered(p.credit, allowed, "");
    if (extra.length) fail(p, `the credit column "${p.credit}" has words Commons does not give: ${extra.join(" ")}`);
  }
  for (const r of i.requests) {
    const missing = requestNotCovered(r, name, licenceName(i));
    if (missing.length)
      fail(p, `Commons asks to be credited as "${r}"; the credit "${name}" would not show: ${missing.join(" ")}. Set the credit column to follow it, or drop the row`);
  }
  // Permission that contradicts the licence tag (another licence, a licence box for another licence) never ships.
  if (i.conflict) fail(p, `${i.conflict}: the licence terms conflict, so drop the row`);
  // A condition or request in Permission must be repeated word for word in the terms column.
  if (i.conditions !== p.terms)
    fail(
      p,
      i.conditions
        ? `Commons' Permission says: "${i.conditions}". Read it; if the photo can be used as the site uses it, put that text in the terms column, otherwise drop the row`
        : `the terms column no longer matches Commons' Permission field (now empty or boilerplate): "${i.permission}"`,
    );
  // The file's own EXIF must not name another author.
  for (const [tag, value] of [["Artist", i.exifArtist], ["Copyright", i.exifCopyright]]) {
    const other = exifMismatch(value, [name, i.author ?? "", i.artist, ...i.users, ...i.requests], { tag, make: i.exifMake });
    if (other) fail(p, `the file's EXIF ${tag} "${value}" names ${other}, not the credited author "${name}"`);
  }
  // A year in a caption is the year the photo was taken, so it must match DateTimeOriginal.
  for (const [lang, caption] of [["en", p.en], ["zh", p.zh]]) {
    const years = captionYears(caption);
    if (years.some((y) => y !== i.year)) fail(p, `the ${lang} caption names ${years.join(", ")} but the photo was taken ${i.year ?? "on an unknown date"}`);
  }
  if (captionYears(p.en).length > 0 !== captionYears(p.zh).length > 0) fail(p, "one caption gives the year and the other does not");
  // "Headquarters" only when Wikidata's current headquarters location is named in the caption.
  if (/headquarters/i.test(p.en) !== /总部/.test(p.zh)) fail(p, "one caption says headquarters (总部) and the other does not");
  if (!headquartersNamed(p.en, hq[p.ticker].names))
    fail(p, `the caption says headquarters, but Wikidata's current headquarters location (${hq[p.ticker].names.join(" / ") || "none"}) is not in it`);
}

/** The notices the author or the file gives, exactly as given: requested attributions, Permission conditions, EXIF copyright. */
const noticesOf = (i) => [...new Set([...i.requests, i.conditions, exifName(i.exifCopyright, { tag: "Copyright" }) ? i.exifCopyright : ""].filter(Boolean))];
/**
 * Whether the Commons file is itself someone's crop of another file: { from } names that file ({{Extracted
 * from}} in its wikitext), or is null when only the file name says "crop". Null when it is not a crop.
 */
const derivedOf = (p, i) => (i.derivedFrom ? { from: i.derivedFrom } : /\bcrop(ped)?\b/i.test(p.file) ? { from: null } : null);
/** The same as an English note, for LICENSES.md. */
const derivedNote = (d, author) =>
  !d ? null : d.from ? `The Commons file is a crop of the Commons file "${d.from}"; Commons credits it to ${author}.` : `The Commons file is a crop (its name says so); Commons credits it to ${author} as their own work.`;

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
  const name = p.credit || i.author;
  const licence = licenceName(i);
  const { data, info: out } = await sharp(file.body)
    .rotate() // apply the EXIF orientation, so the shape below is the shape shown
    .resize(MAX_SIDE, MAX_SIDE, { fit: "inside", withoutEnlargement: true })
    // The credit travels with the file: everything else in the original's metadata is dropped.
    .withExif({
      IFD0: {
        Artist: name,
        Copyright: `${name} / Wikimedia Commons, ${licence}${i.licenceUrl ? ` (${i.licenceUrl})` : ""}. Resized by PreBell. Source: ${i.page}`,
      },
    })
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
    author: name,
    licence,
    licenceUrl: i.licenceUrl,
    page: i.page,
    file: p.file,
    sourceUrl: i.source?.url ?? null,
    sourceTitle: i.source?.title ?? null,
    notices: noticesOf(i),
    derived: derivedOf(p, i),
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
file. The company photographs on earnings checks and ticker pages come from Wikimedia Commons. They are not
covered by the repository's MIT licence: each is credited under the picture with its author and its licence
or public-domain status, and PreBell claims no rights in them. The same details are on the site's photo
credits page (\`/en/photo-credits\`). Pages never load a photo from Wikimedia. To have a photo corrected or
removed, write to support@prebell.site.

Every file was chosen by hand. Only public domain, CC0, CC BY and CC BY-SA files are used. **Changes:** each
photo is resized (the long side to at most ${MAX_SIDE} px, never enlarged) and re-encoded as WebP, with the
credit written into its EXIF; PreBell does not crop or otherwise alter it. Where the licence is CC BY-SA, the
resized file is shared under the same licence.

Licences: ${Object.entries(licences)
    .sort((a, b) => b[1] - a[1])
    .map(([l, n]) => `${l} ${n}`)
    .join(", ")}.

| File | Company | Commons file | Author | Requested attribution / notice | Original source | Licence | Taken | Fetched | Changes | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
${built
  .map(({ p, i, fetched, out }) => {
    const m = manifest[p.ticker];
    return `| \`${p.ticker}.webp\` | ${md(byTicker[p.ticker].name)} | [${md(p.file)}](${m.page}) | ${md(m.author)} | ${md(m.notices.join(" — ") || "none")} | ${m.sourceUrl ? `${m.sourceTitle ? `${md(m.sourceTitle)}: ` : ""}${m.sourceUrl}` : "own work on Commons"} | ${md(m.licence)}${m.licenceUrl ? ` (${m.licenceUrl})` : ""} | ${md(i.taken || "unknown")} | ${fetched} | resized to ${out.width}x${out.height}, WebP | ${md([i.restrictions.length ? `Restrictions: ${i.restrictions.join(", ")}` : "", derivedNote(m.derived, m.author) ?? ""].filter(Boolean).join(" "))} |`;
  })
  .join("\n")}

Total size of the files above: ${total} bytes.
`,
);

writeFileSync(
  join(BUILD, "photos.generated.ts"),
  `// Generated by scripts/build-photos.mjs from data/photos.tsv. Do not edit; run \`pnpm photos\`.\n` +
    `/**\n * Ticker -> its photo: file (with a \`?v=\` content hash), size in px, captions, the credit (author as credited,\n` +
    ` * licence, Commons page) and the source details the photo credits page lists (Commons file name, original\n` +
    ` * source, the author's notices exactly as given, and a note when the Commons file is itself a crop).\n */\n` +
    `export const COMPANY_PHOTOS: Readonly<Record<string, { src: string; width: number; height: number; en: string; zh: string; author: string; licence: string; licenceUrl: string | null; page: string; file: string; sourceUrl: string | null; sourceTitle: string | null; notices: readonly string[]; derived: { from: string | null } | null }>> = ${JSON.stringify(manifest, null, 2)};\n`,
);

// Swap the new folder in: the old one is moved aside first and put back if the swap fails.
const OLD = `${PHOTOS}.old`;
rmSync(OLD, { recursive: true, force: true });
if (existsSync(PHOTOS)) renameSync(PHOTOS, OLD);
try {
  renameSync(BUILD, PHOTOS);
} catch (e) {
  if (existsSync(OLD)) renameSync(OLD, PHOTOS);
  throw e;
}
renameSync(join(PHOTOS, "photos.generated.ts"), MANIFEST);
rmSync(OLD, { recursive: true, force: true });

// The review sheet: every photo as it will appear, with what Commons and the file say beside it.
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const credit = (m, zh) =>
  `${zh ? "图片：" : "Photo: "}${m.author} / Wikimedia Commons · ${m.licence === "Public domain" ? (zh ? "公有领域" : "Public domain") : m.licence} · ${zh ? "已缩放 · 来源" : "resized · Source"}`;
const row = (label, value) => `<dt>${label}</dt><dd>${value}</dd>`;
const cards = built
  .map(({ p, i, data, out, bytes }) => {
    const m = manifest[p.ticker];
    return `<section class="card" id="${esc(p.ticker)}">
<div class="photo"><figure><img src="data:image/webp;base64,${data.toString("base64")}" width="${out.width}" height="${out.height}" alt="${esc(p.en)}">
<figcaption>${esc(p.en)} <span class="credit">${esc(credit(m, false))}</span></figcaption></figure>
<p class="zh">${esc(p.zh)} <span class="credit">${esc(credit(m, true))}</span></p></div>
<div class="source"><label class="reject"><input type="checkbox" value="${esc(p.ticker)}"> Reject ${esc(p.ticker)}</label>
<h2>${esc(p.ticker)} · ${esc(byTicker[p.ticker].name)}</h2>
<dl>${row("Commons file", `<a href="${esc(m.page)}">${esc(p.file)}</a>`)}
${row("Commons description", esc(i.description) || "<em>none</em>")}
${row("Taken", esc(i.taken || "unknown"))}
${row("Wikidata headquarters (P159)", esc(hq[p.ticker].names.join(" / ") || "none"))}
${row("Author (Artist field)", esc(i.artist) || "<em>none</em>")}
${row("Requested attribution", esc(i.requests.join(" — ")) || "<em>none</em>")}
${row("Permission", esc(i.permission) || "<em>none</em>")}
${row("EXIF Artist / Copyright / Make", `${esc(i.exifArtist) || "<em>none</em>"} / ${esc(i.exifCopyright) || "<em>none</em>"} / ${esc(i.exifMake) || "<em>none</em>"}`)}
${row("Original source", m.sourceUrl ? `${esc(m.sourceTitle ?? "")} ${esc(m.sourceUrl)}` : "own work on Commons")}
${m.derived ? row("Derived from", esc(derivedNote(m.derived, m.author))) : ""}
${row("Categories", esc(i.categories.join(" · ")))}
${row("Licence", `${esc(i.licence)}${i.restrictions.length ? ` · Restrictions: ${esc(i.restrictions.join(", "))}` : ""}`)}
${row("File", `${out.width}x${out.height}, ${Math.round(bytes / 1024)} KB`)}</dl></div></section>`;
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
<p>${built.length} photos from <code>apps/web/data/photos.tsv</code>, ${Math.round(total / 1024)} KB in all. Each is shown as on the site: the photo, the caption and the credit (English, then Chinese; "Source" links to the photo credits page), with the Commons description, date, author fields, Permission and the file's own EXIF beside it. Tick <b>Reject</b> on any photo that is wrong, then press the button and paste the list back.</p>
<p><button id="go" type="button">List the rejected tickers</button></p><textarea id="out" readonly placeholder="Rejected tickers appear here"></textarea></header>
${cards}
<script>document.getElementById("go").onclick=function(){var t=[].map.call(document.querySelectorAll(".reject input:checked"),function(x){return x.value});var o=document.getElementById("out");o.value=t.length?"Rejected photos: "+t.join(" "):"No photos rejected.";o.focus();o.select();};</script>
</body></html>
`,
);

console.log(`${built.length} photos, ${total} bytes (${Math.round(total / built.length / 1024)} KB each on average); licences: ${JSON.stringify(licences)}`);
const sizes = built.map((b) => b.bytes).sort((a, b) => a - b);
console.log(`File sizes: smallest ${sizes[0]}, median ${sizes[Math.floor(sizes.length / 2)]}, largest ${sizes.at(-1)} bytes`);
if (stripped.length) console.log(`Flickr " from <place>" dropped from the author's name (check these):\n  ${stripped.join("\n  ")}`);
if (notes.length) console.log(`Notes:\n  ${notes.join("\n  ")}`);
console.log(`Review sheet: ${REVIEW}`);
