// Regenerates crates/core/data/events.tsv (the macro events calendar: FOMC decisions and minutes,
// the BLS jobs report and CPI, US federal elections) from the official schedules:
//   federalreserve.gov/monetarypolicy/fomccalendars.htm   FOMC meetings (decision = second day)
//   federalreserve.gov/newsevents/<month>.htm             FOMC minutes release dates
//   bls.gov/schedule/news_release/{empsit,cpi}.htm         jobs report and CPI release dates
//   crates/core/data/events-manual.tsv                     elections (each row with its proof URL)
// Market closures are not here: the generator writes them from crates/core/src/clock.rs.
// BLS answers 403 to browser and default User-Agents; it serves a User-Agent naming a contact.
// Requests go out at most 4 per second. 200 answers are cached in node_modules/.cache/events for
// CACHE_HOURS, so reruns the same day write the same bytes (the kept window starts from today's
// date, so a rerun on another day may drop or add rows at the edges).
// Usage (repo root):
//   node scripts/build-events.mjs            generate (fetches what the cache lacks or has old)
//   node scripts/build-events.mjs --refresh  fetch everything again, whatever the cache
//   node scripts/build-events.mjs --offline  use only the cache, whatever its age
// It prints the rows added, removed and moved against the committed file; check them before
// committing. See docs/runbook.md.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DATA = join(ROOT, "crates/core/data");
const OUT = join(DATA, "events.tsv");
const MANUAL = join(DATA, "events-manual.tsv");
const CACHE = join(ROOT, "node_modules/.cache/events");
const OFFLINE = process.argv.includes("--offline");
const REFRESH = process.argv.includes("--refresh");
const CACHE_HOURS = 20;
const UA = "PreBell events build (https://prebell.site; support@prebell.site)";
/** At most 4 requests per second to any one agency. */
const GAP_MS = 250;
/** Rows kept: from this many days before today to this many after. */
const PAST_DAYS = 31;
const AHEAD_DAYS = 400;

const FED = "https://www.federalreserve.gov";
const FOMC_URL = `${FED}/monetarypolicy/fomccalendars.htm`;
const FED_CALENDAR_URL = `${FED}/newsevents/calendar.htm`;
const BLS = {
  jobs: "https://www.bls.gov/schedule/news_release/empsit.htm",
  cpi: "https://www.bls.gov/schedule/news_release/cpi.htm",
};
/** Tiers (see crates/core/src/events/mod.rs): 1 major, 2 notable. */
const TIER = { fomc: 1, fomc_minutes: 2, jobs: 1, cpi: 1 };

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const monthIndex = (name) => {
  const i = MONTHS.indexOf(name.slice(0, 3).toLowerCase());
  if (i < 0) fail(`unknown month: ${name}`);
  return i + 1;
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fail = (msg) => {
  console.error(`build-events: ${msg}`);
  process.exit(1);
};
if (OFFLINE && REFRESH) fail("--offline and --refresh exclude each other");

const pad = (n) => String(n).padStart(2, "0");
const ymd = (y, m, d) => `${y}-${pad(m)}-${pad(d)}`;
const todayEt = () => new Intl.DateTimeFormat("en-CA", { timeZone: "America/New_York" }).format(new Date());
const addDays = (date, n) => {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
};

/** The UTC instant (RFC 3339, seconds, Z) of an ET wall-clock time on `date`. */
function etToUtc(date, hour, minute) {
  // ET is UTC-4 or UTC-5: try both and keep the one that reads back as the wanted ET time.
  for (const offset of [4, 5]) {
    const t = new Date(`${date}T${pad(hour)}:${pad(minute)}:00Z`);
    t.setUTCHours(t.getUTCHours() + offset);
    const back = new Intl.DateTimeFormat("en-CA", {
      timeZone: "America/New_York",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    }).format(t);
    if (back === `${date}, ${pad(hour)}:${pad(minute)}`) return t.toISOString().replace(".000Z", "Z");
  }
  fail(`no UTC instant for ${date} ${hour}:${minute} ET`);
}

let last = 0;
async function getText(url) {
  mkdirSync(CACHE, { recursive: true });
  const file = join(CACHE, `${createHash("sha1").update(url).digest("hex")}.html`);
  const fresh = existsSync(file) && Date.now() - statSync(file).mtimeMs < CACHE_HOURS * 3_600_000;
  if (existsSync(file) && (OFFLINE || (fresh && !REFRESH))) return readFileSync(file, "utf8");
  if (OFFLINE) fail(`not cached (--offline): ${url}`);
  const wait = last + GAP_MS - Date.now();
  if (wait > 0) await sleep(wait);
  last = Date.now();
  const res = await fetch(url, { headers: { "User-Agent": UA, Accept: "text/html" } });
  if (res.status !== 200) fail(`${new URL(url).host} answered ${res.status} for ${url}`);
  const text = await res.text();
  writeFileSync(file, text);
  return text;
}

const strip = (s) =>
  s
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/\s+/g, " ")
    .trim();

// ---------------------------------------------------------------------------------------------
// FOMC meetings: one panel per year, one `fomc-meeting` row per meeting ("27-28", "8-9*" with
// projections, "Apr/May 30-1" across months). The decision and statement are on the second day at
// 2:00 PM ET. Rows such as "22 (notation vote)" are not meetings and are skipped.

function parseFomc(html) {
  const out = [];
  const panels = html.split(/<h4><a id="\d+">/).slice(1);
  for (const panel of panels) {
    const year = Number(panel.match(/^(\d{4}) FOMC Meetings/)?.[1]);
    if (!year) continue;
    const rows = panel.matchAll(
      /fomc-meeting__month[^>]*><strong>([^<]+)<\/strong><\/div>\s*<div class="fomc-meeting__date[^>]*>([^<]+)<\/div>/g,
    );
    for (const [, monthText, dateText] of rows) {
      const days = dateText.trim().match(/^(\d{1,2})-(\d{1,2})(\*?)$/);
      if (!days) continue;
      const months = monthText.trim().split("/");
      const month = monthIndex(months[months.length - 1]);
      const first = monthIndex(months[0]);
      // "Dec/Jan" would end in the next year; the Fed has not used it, but keep it right.
      const y = month < first ? year + 1 : year;
      const date = ymd(y, month, Number(days[2]));
      out.push({
        id: `fed:fomc:${date.slice(0, 7)}`,
        series: "fomc",
        ref_period: date.slice(0, 7),
        variant: days[3] ? "sep" : null,
        starts_at: etToUtc(date, 14, 0),
        et_date: date,
        tier: TIER.fomc,
        proof_url: FOMC_URL,
      });
    }
  }
  if (!out.length) fail("no FOMC meetings parsed from the FOMC calendar");
  return out;
}

// ---------------------------------------------------------------------------------------------
// FOMC minutes: the Fed's month pages list "FOMC Minutes / Meeting of December 8-9 / 30" under
// "FOMC Meetings", with the release time (2:00 p.m.). The release is at
// /monetarypolicy/fomcminutesYYYYMMDD.htm, dated by the meeting's last day.

function monthPages(html, today) {
  const pages = [];
  for (const [, path, name, year] of html.matchAll(/<a href="(\/newsevents\/[\w-]+\.htm)">(\w+) (\d{4})<\/a>/g)) {
    const m = monthIndex(name);
    if (ymd(Number(year), m, 31) >= today.slice(0, 8) + "01") pages.push({ url: `${FED}${path}`, year: Number(year), month: m });
  }
  if (!pages.length) fail("no current month pages on the Fed calendar");
  return pages;
}

function parseMinutes(html, year, month, url) {
  const out = [];
  const rows = html.split('<div class="panel-body">').slice(1);
  for (const row of rows) {
    const text = strip(row);
    const m = text.match(
      /^(\d{1,2}):(\d{2}) ([ap])\.m\. FOMC Minutes Meeting of ([A-Za-z]+) (\d{1,2})(?:\s*-\s*(?:([A-Za-z]+) )?(\d{1,2}))? (\d{1,2})\b/,
    );
    if (!m) continue;
    const [, hh, mm, ap, startMonth, , endMonth, endDay, releaseDay] = m;
    const meetingMonth = monthIndex(endMonth ?? startMonth);
    const meetingDay = Number(endDay ?? m[5]);
    const meetingYear = meetingMonth > month ? year - 1 : year;
    const meeting = ymd(meetingYear, meetingMonth, meetingDay);
    const date = ymd(year, month, Number(releaseDay));
    const hour = (Number(hh) % 12) + (ap === "p" ? 12 : 0);
    out.push({
      id: `fed:fomc_minutes:${meeting}`,
      series: "fomc_minutes",
      ref_period: meeting,
      variant: null,
      starts_at: etToUtc(date, hour, Number(mm)),
      et_date: date,
      tier: TIER.fomc_minutes,
      proof_url: url,
    });
  }
  return out;
}

// ---------------------------------------------------------------------------------------------
// BLS: a table of reference month, release date ("Nov. 06, 2026", "May 07, 2027", "Sept. 04,
// 2026") and time ("08:30 AM"). Past releases drop off the page.

function parseBls(html, series, url) {
  const out = [];
  const rows = html.matchAll(
    // Cells may carry attributes and a footnote mark after the value (as in crates/core/src/events/parse.rs).
    /<tr[^>]*>\s*<td[^>]*>([A-Za-z]+) (\d{4})[^<]*(?:<[^/][^>]*>[^<]*<\/[^>]+>)?[^<]*<\/td>\s*<td[^>]*>([A-Za-z]+)\.? (\d{1,2}), (\d{4})[^<]*(?:<[^/][^>]*>[^<]*<\/[^>]+>)?[^<]*<\/td>\s*<td[^>]*>(\d{1,2}):(\d{2}) ([AP])M[^<]*<\/td>/g,
  );
  for (const [, refMonth, refYear, month, day, year, hh, mm, ap] of rows) {
    const date = ymd(Number(year), monthIndex(month), Number(day));
    const hour = (Number(hh) % 12) + (ap === "P" ? 12 : 0);
    const ref = `${refYear}-${pad(monthIndex(refMonth))}`;
    out.push({
      id: `bls:${series}:${ref}`,
      series,
      ref_period: ref,
      variant: null,
      starts_at: etToUtc(date, hour, Number(mm)),
      et_date: date,
      tier: TIER[series],
      proof_url: url,
    });
  }
  if (!out.length) fail(`no ${series} releases parsed from ${url}`);
  return out;
}

// ---------------------------------------------------------------------------------------------

const COLUMNS = ["id", "series", "ref_period", "variant", "starts_at", "et_date", "tier", "proof_url"];
const opt = (s) => (s && s !== "-" ? s : null);

function readRows(path) {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((l) => l.trim() && !l.startsWith("#"))
    .map((l) => {
      const c = l.split("\t").map((s) => s.trim());
      if (c.length !== COLUMNS.length) fail(`${path}: ${c.length} columns, want ${COLUMNS.length}: ${l}`);
      return Object.fromEntries(COLUMNS.map((k, i) => [k, k === "tier" ? Number(c[i]) : opt(c[i])]));
    });
}

function manualRows() {
  const rows = readRows(MANUAL);
  for (const r of rows) {
    if (!r.proof_url?.startsWith("https://")) fail(`events-manual.tsv: ${r.id} has no official https proof_url`);
    if (!/^\d{4}-\d{2}-\d{2}$/.test(r.et_date ?? "")) fail(`events-manual.tsv: ${r.id} has a bad et_date`);
    if (![1, 2, 3].includes(r.tier)) fail(`events-manual.tsv: ${r.id} has a bad tier`);
  }
  return rows;
}

const today = todayEt();
const from = addDays(today, -PAST_DAYS);
const to = addDays(today, AHEAD_DAYS);

const fomc = parseFomc(await getText(FOMC_URL));
const minutes = [];
for (const p of monthPages(await getText(FED_CALENDAR_URL), today)) {
  minutes.push(...parseMinutes(await getText(p.url), p.year, p.month, p.url));
}
const jobs = parseBls(await getText(BLS.jobs), "jobs", BLS.jobs);
const cpi = parseBls(await getText(BLS.cpi), "cpi", BLS.cpi);

const byId = new Map();
// Official schedules are kept for the window; elections (years ahead) only drop off once past.
const manual = manualRows();
for (const r of [...fomc, ...minutes, ...jobs, ...cpi, ...manual]) {
  if (r.et_date < from || (r.et_date > to && !manual.includes(r))) continue;
  if (byId.has(r.id)) fail(`duplicate id ${r.id}`);
  byId.set(r.id, r);
}
const rows = [...byId.values()].sort(
  (a, b) =>
    a.et_date.localeCompare(b.et_date) || (a.starts_at ?? "").localeCompare(b.starts_at ?? "") || a.id.localeCompare(b.id),
);

// The diff against the committed file, for review.
const old = new Map(readRows(OUT).map((r) => [r.id, r]));
for (const r of rows) {
  const o = old.get(r.id);
  if (!o) console.log(`new     ${r.id} ${r.et_date}`);
  else if (o.et_date !== r.et_date || o.starts_at !== r.starts_at) console.log(`moved   ${r.id} ${o.et_date} -> ${r.et_date}`);
}
for (const [id, o] of old) {
  if (!byId.has(id) && o.et_date >= from) console.log(`removed ${id} ${o.et_date}`);
}

const header = [
  "# GENERATED by scripts/build-events.mjs; do not edit. Elections go in events-manual.tsv; rerun",
  "# the script to refresh the official dates (see docs/runbook.md).",
  "#",
  "# Macro events (v1: FOMC decisions and minutes, BLS jobs report and CPI, US federal elections).",
  "# starts_at is UTC (empty for all-day events); et_date is the ET calendar day. Tiers: 1 major,",
  "# 2 notable. Every row's proof_url is the official page that lists its date.",
  `# covers: ${from} ${to}`,
  `# ${COLUMNS.join("\t")}`,
].join("\n");
const body = rows.map((r) => COLUMNS.map((k) => (r[k] === null || r[k] === undefined ? "-" : String(r[k]))).join("\t"));
writeFileSync(`${OUT}.tmp`, `${header}\n${body.join("\n")}\n`);
renameSync(`${OUT}.tmp`, OUT);
const count = (s) => rows.filter((r) => r.series === s).length;
console.log(
  `build-events: ${rows.length} rows (fomc ${count("fomc")}, minutes ${count("fomc_minutes")}, jobs ${count("jobs")}, cpi ${count("cpi")}, elections ${count("election")}), ${from} .. ${to}`,
);
