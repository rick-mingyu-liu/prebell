import type { Lang } from "./posts";

const TZ = "America/New_York";

const en = new Intl.DateTimeFormat("en-US", { timeZone: TZ, dateStyle: "medium", timeStyle: "short" });
const zh = new Intl.DateTimeFormat("zh-CN", {
  timeZone: TZ,
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  hour12: true,
});

/** A UTC ISO timestamp shown in US Eastern time, e.g. "Oct 29, 2026, 8:34 AM ET". */
export function formatEt(iso: string | null | undefined, lang: Lang): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  // ICU puts a narrow no-break space before AM/PM; keep plain spaces.
  if (lang === "en") return `${en.format(d).replace(/[  ]/g, " ")} ET`;
  return `${zh.format(d).replace(/[  ]/g, " ")}（美东）`;
}

/** A trading date (YYYY-MM-DD) as a calendar date, independent of time zone. */
export function formatDate(date: string, lang: Lang): string {
  const d = new Date(`${date}T12:00:00Z`);
  if (Number.isNaN(d.getTime())) return date;
  return new Intl.DateTimeFormat(lang === "en" ? "en-US" : "zh-CN", {
    timeZone: "UTC",
    dateStyle: lang === "en" ? "medium" : "long",
  }).format(d);
}

const enTime = new Intl.DateTimeFormat("en-US", { timeZone: TZ, timeStyle: "short" });
const zhTime = new Intl.DateTimeFormat("zh-CN", { timeZone: TZ, hour: "numeric", minute: "2-digit", hour12: true });

/** A UTC ISO timestamp as a time of day in US Eastern, e.g. "8:34 AM ET". */
export function formatEtTime(iso: string | null | undefined, lang: Lang): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  if (lang === "en") return `${enTime.format(d).replace(/[  ]/g, " ")} ET`;
  return `${zhTime.format(d).replace(/[  ]/g, " ")}（美东）`;
}

const enDay = new Intl.DateTimeFormat("en-US", { timeZone: "UTC", weekday: "short", month: "short", day: "numeric" });
const zhMonthDay = new Intl.DateTimeFormat("zh-CN", { timeZone: "UTC", month: "long", day: "numeric" });
const zhWeekday = new Intl.DateTimeFormat("zh-CN", { timeZone: "UTC", weekday: "short" });

/** A trading date as a short label with the weekday: "Mon, Nov 2" / "11月2日 周一". */
export function formatDayShort(date: string, lang: Lang): string {
  const d = new Date(`${date}T12:00:00Z`);
  if (Number.isNaN(d.getTime())) return date;
  return lang === "en" ? enDay.format(d) : `${zhMonthDay.format(d)} ${zhWeekday.format(d)}`;
}
