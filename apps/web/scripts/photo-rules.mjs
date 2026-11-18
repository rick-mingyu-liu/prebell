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

/**
 * Lower-case word tokens, initials included ("Larry D. Moore" is larry, d, moore, so "Larry E. Moore" is another
 * name), accents folded ("Hiàn" is "hian"), plus "©" as its own token. Punctuation drops out.
 */
const tokens = (s) => {
  const t = String(s ?? "")
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/_/g, " ");
  return [...(t.includes("©") ? ["©"] : []), ...(t.match(/[\p{L}\p{N}]+/gu) ?? [])];
};

/** EXIF values that name no one, in either field: placeholders a camera or an app leaves. */
const PLACEHOLDER = /^(unknown|none|owner|copyright|author|artist|photographer|camera|digital camera)$/i;
/** EXIF Artist values that are the editing software, not a person (never ignored in Copyright). */
const SOFTWARE = /^(picasa|photoshop|adobe photoshop|lightroom|adobe lightroom|gimp|windows photo viewer)\b/i;
/** Camera makers: ignored only in Artist, and only when the value is the camera's own Make (a camera default). */
const MAKERS = /^(canon|nikon|sony|olympus|ricoh|pentax|fujifilm|panasonic|samsung|apple|google|huawei|xiaomi|dji|leica|motorola|htc|lg|oneplus)$/i;
const fold = (s) => tokens(s).join(" ");

/**
 * An EXIF Artist or Copyright value reduced to the name in it ("©2019 Kenneth C. Zirkel" -> "Kenneth C. Zirkel"),
 * or "" when it names no one. In Artist, editing software and the camera maker (when it equals the EXIF Make) are
 * not names; in Copyright every name counts, a company included. "All rights reserved" with no name is returned
 * as it stands, so it is never taken as naming no one.
 */
export function exifName(value, { tag = "Copyright", make = "" } = {}) {
  const raw = text(value);
  const v = raw
    .replace(/\b(cc[ -]?by(-sa)?|cc0|public domain)\b[\s\d.]*/gi, " ")
    .replace(/©|\(c\)|\bcopyright\b:?|\bphotographer\b:?|\bphoto(graph)?( by)?\b:?|\ball rights reserved\b|\b(19|20)\d\d\b/gi, " ")
    .replace(/^[\s,.;:/-]+|[\s,.;:/-]+$/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!v || !/\p{L}{2,}/u.test(v)) return /all rights reserved/i.test(raw) ? raw : "";
  if (PLACEHOLDER.test(v)) return "";
  if (tag === "Artist" && SOFTWARE.test(v)) return "";
  if (tag === "Artist" && MAKERS.test(v) && make && fold(make).split(" ")[0] === fold(v)) return "";
  return v;
}

/**
 * Null when an EXIF Artist or Copyright value names no one or names the credited author: every word of the EXIF
 * name must be among the words of the credited names (the credit, the author, the Commons user name and the
 * requested attributions), so "Jeroen Stroes" passes for "Jeroen Stroes Aviation Photography" but "Coolcaesar
 * for Reuters" does not pass for "Coolcaesar". Otherwise returns the name the EXIF gives, which is someone else.
 */
export function exifMismatch(value, credited, opts) {
  const name = exifName(value, opts);
  if (!name) return null;
  return namedIn(name, credited) ? null : name;
}

/** Whether every word of `name` (initials included) is among the words of the credited names. */
function namedIn(name, credited) {
  const a = tokens(name).filter((x) => x !== "©");
  const b = new Set(credited.flatMap((c) => tokens(c)).filter((x) => x !== "©"));
  return a.length > 0 && a.every((w) => b.has(w));
}

/** The Commons or Wikipedia user names an Artist field links to ("User:Hiàn" -> "Hiàn"). */
export function userNames(artistHtml) {
  return [...String(artistHtml ?? "").matchAll(/href="[^"]*\/wiki\/User:([^"#?/]+)"/gi)].map((m) => decodeURIComponent(m[1]).replace(/_/g, " "));
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

const sameLicence = (x, y) => x.toLowerCase().replace(/[-\s]+/g, " ").trim() === y.toLowerCase().replace(/[-\s]+/g, " ").trim();

/** Commons' CC licence box as text, in full: its long and short licence names, an optional "Attribution:", and what follows it. */
const BOX = new RegExp(
  "^This file is licensed under the Creative Commons (Attribution(?:-Share Alike)? \\d\\.\\d(?: [A-Za-z]+)*) license\\." +
    "(?: Attribution: (.+?))?" +
    " You are free: to share – to copy, distribute and transmit the work to remix – to adapt the work" +
    " Under the following conditions: attribution – You must give appropriate credit, provide a link to the license, and indicate if changes were made\\." +
    " You may do so in any reasonable manner, but not in any way that suggests the licensor endorses you or your use\\." +
    "( share alike – If you remix, transform, or build upon the material, you must distribute your contributions under the same or compatible license as the original\\.)?" +
    " https://creativecommons\\.org/\\S+ (CC BY(?:-SA)? \\d\\.\\d(?: [a-z]{2}(?:-[a-z]{2})?)?) Creative Commons (Attribution(?:-Share Alike)? \\d\\.\\d) true true(.*)$",
);
/** Commons' VRT permission notice, in full, with its ticket number. */
const VRT = new RegExp(
  "^This work is free and may be used by anyone for any purpose\\. If you wish to use this content, you do not need to request permission as long as you follow any licensing requirements mentioned on this page\\." +
    " The Wikimedia Foundation has received an e-mail confirming that the copyright holder has approved publication under the terms mentioned on this page\\." +
    " This correspondence has been reviewed by a Volunteer Response Team \\(VRT\\) member and stored in our permission archive\\." +
    " The correspondence is available to trusted volunteers as ticket #(\\d+)\\. If you have questions about the archived correspondence, please use the VRT noticeboard\\." +
    " Ticket link: https://ticket\\.wikimedia\\.org/otrs/index\\.pl\\?Action=AgentTicketZoom&TicketNumber=\\1 Find other files from the same ticket:(.*)$",
);
/** The only sentence allowed after a licence box: the Flickr or Panoramio bot's review. */
const REVIEWED = /^This image, which was originally posted to (Flickr|Panoramio), was automatically reviewed on \d{1,2} [A-Z][a-z]+ \d{4} by [^.,]+, who confirmed that it was available on \1 under the above license on that date\.$/;
/** The {{self}} line: a name, then the template's words. The name is checked against the credited names. */
const SELF = /^(.{1,120}), the copyright holder of this work, hereby publishes it under the following licen[cs]e:$/;
/** How {{self}} renders the author of a file transferred from a Wikipedia: "X at the English-language Wikipedia". */
const AT_WIKIPEDIA = / at the [A-Z][a-z]+-language Wikipedia$/;
/**
 * Wording in Permission that names a licence the build never accepts, in any form: non-commercial, no
 * derivatives (also as "-nc", "nc-sa", "-nd"), the GFDL, or "all rights reserved". Always a conflict.
 */
const FORBIDDEN = /non-?commercial|no-?deriv|\bGFDL\b|GNU Free Documentation|all rights reserved|(^|[\s(\-/_,])(nc|nd)(?=$|[\s)\-/_.,;])/i;

/**
 * Commons' Permission text, checked against the file's licence and the credited names. Known template text is
 * removed exactly: the licence box (whose licence must be the file's), the {{self}} line when its name is one of
 * the credited names (so a condition written into the name is not skipped), the VRT notice, and the file's own
 * licence name restated. After a box only the Flickr or Panoramio review sentence may follow; after a VRT notice, nothing.
 * Returns { conditions, attribution, conflict }:
 * - conditions: the whole text when anything is left over ("" when none): someone must read it, and the build
 *   stops until data/photos.tsv repeats it word for word;
 * - attribution: the "Attribution: X" a box names, or null;
 * - conflict: why the text contradicts the file's licence (a box or a restated name for another licence, or
 *   non-commercial, no-derivatives, GFDL or "all rights reserved" wording anywhere), or null. The build always
 *   stops on a conflict, whatever the terms column says.
 */
export function permissionTerms(html, licence = "", credited = []) {
  const t = text(html);
  const none = { conditions: "", attribution: null, conflict: null };
  if (!t) return none;
  const forbidden = t.match(FORBIDDEN);
  if (forbidden) return { conditions: t, attribution: null, conflict: `Permission says "${forbidden[0].trim()}", a licence the site does not use` };
  if (/^(public domain|cc0( 1\.0)?|cc[ -]by(-sa)?[ -]\d\.\d( [a-z]{2})?)$/i.test(t))
    return sameLicence(t, licence) ? none : { conditions: t, attribution: null, conflict: `Permission names ${t}, the licence tag ${licence || "none"}` };
  const self = t.match(SELF);
  if (self) return namedIn(self[1].replace(AT_WIKIPEDIA, ""), credited) ? none : { ...none, conditions: t };
  const vrt = t.match(VRT);
  if (vrt) return vrt[2].trim() ? { ...none, conditions: t } : none;
  const box = t.match(BOX);
  if (box) {
    const [, long, attribution = null, sa, short, again, rest] = box;
    const conflict =
      !sameLicence(short, licence) || !long.startsWith(again) || Boolean(sa) !== /-SA/.test(short) || /Share Alike/.test(long) !== /-SA/.test(short)
        ? `Commons' licence box names ${short} (${long}), the licence tag ${licence || "none"}`
        : null;
    const after = rest.trim();
    return { conditions: after && !REVIEWED.test(after) ? t : "", attribution: attribution?.trim() ?? null, conflict };
  }
  if (/^This file is licensed under/i.test(t)) return { ...none, conditions: t, conflict: `Commons' licence box could not be read: "${t.slice(0, 120)}"` };
  return { ...none, conditions: t };
}

/** Hosts in a Credit field that are Wikimedia itself or its tools, never the original source. */
const WIKIMEDIA = /(^|\.)(wikimedia\.org|wikipedia\.org|wikidata\.org|wikimedia\.de|toolforge\.org|wmflabs\.org|wmcloud\.org)$/;
/** Link texts that are not a title. */
const GENERIC = /^(file url|catalog(ue)? record|source|link|here|original|website|download|flickr|panoramio|view|image)$/i;

/**
 * The first https link in a Credit field that leaves Wikimedia and its tools (Flickr, Panoramio via the web
 * archive, a museum's file), with its title when the link text is one (not the URL, not "File URL"). A plain-http
 * link is not used: the credits page writes only https URLs. Null when there is none.
 */
export function originalSource(creditHtml) {
  for (const m of String(creditHtml ?? "").matchAll(/<a\b[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/gi)) {
    const url = decode(m[1]).replace(/^\/\//, "https://");
    let host;
    try {
      host = new URL(url).hostname;
    } catch {
      continue;
    }
    if (!/^https:\/\/\S+$/.test(url) || WIKIMEDIA.test(host)) continue;
    const label = text(m[2]);
    return { url, title: label && label !== url && !/^https?:/.test(label) && !GENERIC.test(label) ? label : null, flickr: /(^|\.)flickr\.com$/.test(host) };
  }
  return null;
}
