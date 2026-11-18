// The photo build's rules as pure functions, so tests can check them (tests/photorules.test.ts) and the
// build (scripts/build-photos.mjs) applies exactly what is tested. Nothing here fetches or writes anything.

/** HTML entities in Commons metadata -> characters. */
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
const decode = (s) =>
  s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) =>
    e[0] === "#" ? String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : (ENTITIES[e.toLowerCase()] ?? m),
  );

/** Commons HTML -> plain text: style and script blocks and tags removed, entities decoded, whitespace collapsed. */
export function text(html) {
  return decode(
    String(html ?? "")
      .replace(/<(style|script)\b[\s\S]*?<\/\1>/gi, " ")
      .replace(/<[^>]+>/g, " "),
  )
    .replace(/\s+/g, " ")
    .replace(/\s+([,.;:!?)])/g, "$1")
    .replace(/\(\s+/g, "(")
    .trim();
}

/**
 * Licences accepted: public domain, CC0, CC BY and CC BY-SA, any version or port (e.g. "CC BY-SA 3.0 de").
 * Anchored, so BY-NC, BY-ND, GFDL and "Attribution" alone are refused.
 */
export const LICENCE = /^(public domain|cc0( 1\.0)?|cc by(-sa)? \d\.\d( [a-z]{2}(-[a-z]{2})?)?)$/i;
export const licenceAllowed = (name) => LICENCE.test(String(name ?? "").trim());

/** A licence URL over https: http://creativecommons.org/ is rewritten; null when it is not on creativecommons.org. */
export function licenceUrl(url) {
  if (!url) return null;
  const u = String(url).trim().replace(/^http:\/\/creativecommons\.org\//, "https://creativecommons.org/");
  return /^https:\/\/creativecommons\.org\/\S+$/.test(u) ? u : null;
}

/** A Commons file page URL over https, or null. */
export const commonsPage = (url) => (/^https:\/\/commons\.wikimedia\.org\/wiki\/File:\S+$/.test(String(url ?? "")) ? String(url) : null);

/**
 * A name as a plain name (a "User:" prefix dropped), or null when it is not one: empty, unknown or anonymous,
 * a URL, an @ (an email address, or a name with an affiliation such as "X @ Project / Team"), markup, the
 * uploader rather than the author ("The original uploader was ..."), or a sentence.
 */
export function authorName(html) {
  const name = text(html)
    .replace(/^User:/i, "")
    .replace(/\s*\((talk|contribs?)\)$/i, "");
  if (!name || name.length > 60) return null;
  if (/[<>{}\[\]|@]|https?:|www\.|\bunknown\b|\banonymous\b|\bown work\b|\buploader\b|\bemployee\b|\bnone\b|^(user|author)$/i.test(name)) return null;
  return name;
}

/** The Flickr importer's "Name from Place" -> "Name" (only for a file whose source is Flickr); unchanged otherwise. */
export const stripFlickrPlace = (name, fromFlickr) => (fromFlickr ? name.replace(/ from [^,]+(, [^,]+)*$/, "") : name);

const MONTHS = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
/**
 * The year a photo was taken, from DateTimeOriginal: "2022:05:01 10:00:00", "2022-05-01", "2009-08",
 * "Taken on 8 June 2016, 11:08:51", "28 October 2013" or "14 July 2021 (according to Exif data)". Null when
 * the field is empty or not a clean date (an upload date, "circa", a range, free text).
 */
export function takenYear(raw, now = new Date()) {
  const t = text(raw)
    .replace(/^taken on /i, "")
    .replace(/ \(according to Exif data\)$/i, "");
  const iso = t.match(/^(\d{4})([-:]\d{2}([-:]\d{2})?)?([ T]\d{1,2}:\d{2}(:\d{2})?)?$/);
  const long = t.match(/^(\d{1,2}) ([A-Za-z]+) (\d{4})(,? \d{1,2}:\d{2}(:\d{2})?)?$/);
  const year = iso ? Number(iso[1]) : long && MONTHS.includes(long[2].toLowerCase()) ? Number(long[3]) : null;
  return year && year > 1820 && year <= now.getFullYear() ? year : null;
}

/** The four-digit years a caption names. */
export const captionYears = (caption) => [...String(caption).matchAll(/(?<!\d)(1[89]\d\d|20\d\d)(?!\d)/g)].map((m) => Number(m[1]));

/**
 * Whether an English caption that says "headquarters" names one of Wikidata's headquarters places (P159 or the
 * place it lies in; the part before a comma). A caption without "headquarters" passes.
 */
export function headquartersNamed(caption, places) {
  if (!/headquarters/i.test(caption)) return true;
  return places.some((p) => p && caption.includes(p.replace(/,.*$/, "")));
}

/** Lower-case word tokens of length 2 or more (initials and punctuation drop out), plus "©" as its own token. */
const tokens = (s) => {
  const t = String(s ?? "").toLowerCase().replace(/_/g, " ");
  return [...(t.includes("©") ? ["©"] : []), ...(t.match(/[\p{L}\p{N}]{2,}/gu) ?? [])];
};

/** EXIF Artist or Copyright values that name no one: software, camera defaults, placeholders. */
const NO_NAME = /^(picasa|adobe|photoshop|lightroom|gimp|windows|microsoft|digital camera|camera|owner|unknown|none|copyright|author|artist|photographer|all rights reserved|ricoh|olympus|canon|nikon|sony|samsung|apple|google|huawei|xiaomi|dji)$/i;
/** An EXIF Artist or Copyright value reduced to the name in it: "©2019 Kenneth C. Zirkel" -> "Kenneth C. Zirkel"; "" when it names no one. */
export function exifName(value) {
  const v = text(value)
    .replace(/\b(cc[ -]?by(-sa)?|cc0|public domain)\b[\s\d.]*/gi, " ")
    .replace(/©|\(c\)|\bcopyright\b:?|\bphotographer\b:?|\bphoto(graph)?( by)?\b:?|\ball rights reserved\b|\b(19|20)\d\d\b/gi, " ")
    .replace(/^[\s,.;:/-]+|[\s,.;:/-]+$/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!v || NO_NAME.test(v) || /^picasa\b/i.test(v) || !/\p{L}{2,}/u.test(v)) return "";
  return v;
}

/**
 * Null when an EXIF Artist or Copyright value names no one or names the credited author (one name's words
 * contain the other's: "Jeroen Stroes" and "Jeroen Stroes Aviation Photography"); otherwise the name it gives,
 * which is someone else.
 */
export function exifMismatch(value, credited) {
  const name = exifName(value);
  if (!name) return null;
  const a = new Set(tokens(name).filter((x) => x !== "©"));
  const b = new Set(credited.flatMap((c) => tokens(c)).filter((x) => x !== "©"));
  const within = (x, y) => x.size > 0 && [...x].every((w) => y.has(w));
  return within(a, b) || within(b, a) ? null : name;
}

/** Words a credit line's fixed tail always shows: "/ Wikimedia Commons · {licence}", and the connecting words a request may use. */
const TAIL = new Set(["via", "wikimedia", "commons", "the", "at", "on", "and", "licensed", "under", "licence", "license"]);
/**
 * The words of an author's requested attribution that the rendered credit would not show: the credit name, the
 * licence and the fixed tail are taken out, so an empty result means the request is followed in full. "©" counts
 * as a word, so a requested "©" must be in the credit name.
 */
export function requestNotCovered(request, creditName, licence) {
  const shown = new Set([...tokens(creditName), ...tokens(licence), ...tokens(String(licence).replace(/-/g, " ")), ...TAIL]);
  return tokens(String(request).replace(/\bcc-by(-sa)?-(\d)\.(\d)\b/gi, "cc by$1 $2 $3")).filter((w) => !shown.has(w) && !/^(sa|by|cc)$/.test(w));
}

/**
 * Commons' Permission text with the parts that say nothing beyond the licence taken out: Commons' own licence box
 * ("This file is licensed under ... Attribution: X ..."), the {{self}} line ("X, the copyright holder of this
 * work, hereby publishes it under the following license:"), a VRT ticket notice and the file's own licence name
 * restated (a different licence name is kept: it contradicts the licence tag). Whatever is left is a
 * condition or request someone must read: the build stops until data/photos.tsv repeats it word for word.
 * Returns { conditions, attribution }: the remaining text ("" when none) and an "Attribution: X" the box names.
 */
export function permissionTerms(html, licence = "") {
  const t = text(html);
  // The licence's own name restated ("Public domain" on a public-domain file) adds nothing; a different one does.
  const same = (x) => x.toLowerCase().replace(/[-\s]+/g, " ").trim();
  if (t && licence && same(t) === same(licence)) return { conditions: "", attribution: null };
  const attribution = t.match(/^This file is licensed under the Creative Commons .*? license\. Attribution: (.+?) You are free:/)?.[1]?.trim() ?? null;
  const boiler =
    /^This file is licensed under the Creative Commons .* license\./.test(t) ||
    /^.{1,120}, the copyright holder of this work, hereby publishes it under the following licen[cs]e:$/.test(t) ||
    /^This work is free and may be used by anyone for any purpose\. .*Volunteer Response Team/.test(t);
  return { conditions: boiler ? "" : t, attribution };
}

/** The first link in a Credit field that leaves Wikimedia, with its text: the original source (Flickr, Panoramio, a site). */
export function originalSource(creditHtml) {
  for (const m of String(creditHtml ?? "").matchAll(/<a\b[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/gi)) {
    const url = decode(m[1]).replace(/^\/\//, "https://");
    let host;
    try {
      host = new URL(url).hostname;
    } catch {
      continue;
    }
    if (!/^https?:\/\//.test(url) || /(^|\.)(wikimedia|wikipedia|wikidata)\.org$/.test(host)) continue;
    const label = text(m[2]);
    return { url, title: label && label !== url && !/^https?:/.test(label) ? label : null, flickr: /(^|\.)flickr\.com$/.test(host) };
  }
  return null;
}
