import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import CalendarWeek from "../src/components/CalendarWeek";
import EconomyEvents from "../src/components/EconomyEvents";
import { eventTitle, listEvents, type MacroEvent } from "../src/lib/events";
import type { D1Like } from "../src/lib/posts";
import { memoryD1 } from "./d1";

const CPI = "https://www.bls.gov/schedule/news_release/cpi.htm";
const FOMC = "https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm";
const row = (id: string, series: string, ref: string | null, variant: string | null, startsAt: string | null, date: string, tier: number, status: string, proof: string, release: string | null = null, movedFrom: string | null = null) =>
  `('${id}','${series}',${ref ? `'${ref}'` : "NULL"},${variant ? `'${variant}'` : "NULL"},${startsAt ? `'${startsAt}'` : "NULL"},'${date}',${tier},'${status}','${proof}',${release ? `'${release}'` : "NULL"},${movedFrom ? `'${movedFrom}'` : "NULL"},0,'x','x','x')`;
const seed = `INSERT INTO macro_events (id, series, ref_period, variant, starts_at, et_date, tier, status, proof_url, release_url, moved_from, missing_count, first_seen_at, checked_at, updated_at) VALUES
  ${row("bls:cpi:2026-10", "cpi", "2026-10", null, "2026-11-10T13:30:00Z", "2026-11-10", 1, "released", CPI, "https://www.bls.gov/news.release/archives/cpi_11102026.htm")},
  ${row("fed:fomc_minutes:2026-10-28", "fomc_minutes", "2026-10-28", null, "2026-11-18T19:00:00Z", "2026-11-18", 2, "moved", "https://www.federalreserve.gov/newsevents/2026-november.htm", null, "2026-11-17")},
  ${row("bls:jobs:2026-10", "jobs", "2026-10", null, "2026-11-06T13:30:00Z", "2026-11-06", 1, "confirmed", "https://www.bls.gov/schedule/news_release/empsit.htm")},
  ${row("nyse:early_close:2026-11-27", "early_close", null, null, null, "2026-11-27", 2, "scheduled", "https://www.nyse.com/markets/hours-calendars")},
  ${row("x:ppi:2026-10", "ppi", "2026-10", null, "2026-11-13T13:30:00Z", "2026-11-13", 2, "scheduled", "https://www.bls.gov/x")},
  ${row("x:bad:2026-11", "cpi", "2026-11", null, "2026-11-12T13:30:00Z", "2026-11-12", 1, "scheduled", "https://evil.example/cpi")},
  ${row("x:minor:2026-11", "cpi", "2026-11", null, "2026-11-11T13:30:00Z", "2026-11-11", 3, "scheduled", CPI)};`;

describe("eventTitle", () => {
  it("names each v1 kind from the glossary in both languages", () => {
    expect(eventTitle("cpi", "2026-10", null, "en")).toBe("October CPI");
    expect(eventTitle("cpi", "2026-10", null, "zh")).toBe("10月消费者物价指数（CPI）");
    expect(eventTitle("jobs", "2026-11", null, "zh")).toBe("11月非农就业报告");
    expect(eventTitle("fomc", "2026-12", "sep", "en")).toBe("FOMC rate decision (with projections)");
    expect(eventTitle("fomc_minutes", "2026-10-28", null, "zh")).toBe("FOMC会议纪要（10月会议）");
    expect(eventTitle("election", null, "general", "en")).toBe("US general election");
    expect(eventTitle("ppi", "2026-10", null, "en")).toBeNull();
    expect(eventTitle("cpi", null, null, "en")).toBeNull();
  });
});

describe("listEvents", () => {
  it("groups tier 1-2 events by date and drops unknown kinds and unofficial links", async () => {
    const ev = await listEvents(memoryD1(seed), "2026-11-02", "2026-11-30", "en");
    expect(Object.keys(ev).sort()).toEqual(["2026-11-06", "2026-11-10", "2026-11-18", "2026-11-27"]);
    expect(ev["2026-11-10"][0]).toMatchObject({
      title: "October CPI",
      tier: 1,
      status: "released",
      releaseUrl: "https://www.bls.gov/news.release/archives/cpi_11102026.htm",
    });
    expect(ev["2026-11-18"][0]).toMatchObject({ status: "moved", movedFrom: "2026-11-17", tier: 2 });
    expect(ev["2026-11-27"][0]).toMatchObject({ title: "Early close, 1:00 PM ET", startsAt: null });
  });

  it("is empty before migration 0008 (no table)", async () => {
    const db: D1Like = {
      prepare() {
        throw new Error("no such table: macro_events");
      },
    };
    expect(await listEvents(db, "2026-11-02", "2026-11-06", "en")).toEqual({});
  });
});

describe("EconomyEvents", () => {
  const ev = (more: Partial<MacroEvent>): MacroEvent => ({
    id: "bls:cpi:2026-10",
    series: "cpi",
    title: "October CPI",
    note: null,
    startsAt: "2026-11-10T13:30:00Z",
    date: "2026-11-10",
    tier: 1,
    status: "confirmed",
    proofUrl: CPI,
    releaseUrl: null,
    movedFrom: null,
    ...more,
  });

  it("shows time, title, the major mark and the schedule link", () => {
    const out = renderToStaticMarkup(<EconomyEvents events={[ev({})]} locale="en" />);
    expect(out).toContain('<h3 class="cal-slot">Economy &amp; Fed</h3>');
    expect(out).toContain("8:30 AM ET");
    expect(out).toContain('<span class="cal-event-title">October CPI</span><span class="cal-event-major">Major</span>');
    expect(out).toContain(`<a class="cal-read" href="${CPI}" rel="noopener noreferrer">Schedule ↗</a>`);
  });

  it("once released, a tick, the word for screen readers and the release link", () => {
    const url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
    const out = renderToStaticMarkup(<EconomyEvents events={[ev({ status: "released", releaseUrl: url })]} locale="zh" />);
    expect(out).toContain('<span class="sr-only">已发布</span>');
    expect(out).toContain(`href="${url}" rel="noopener noreferrer">发布原文 ↗</a>`);
    expect(out).toContain("重要");
  });

  it("writes moved and postponed as words, and the FOMC press conference line", () => {
    const moved = renderToStaticMarkup(<EconomyEvents events={[ev({ status: "moved", movedFrom: "2026-11-05", tier: 2 })]} locale="en" />);
    expect(moved).toContain("Moved from Nov 5");
    expect(moved).not.toContain("Major");
    const fomc = renderToStaticMarkup(
      <EconomyEvents
        events={[ev({ series: "fomc", status: "postponed", title: "FOMC rate decision", note: "Press conference 2:30 PM ET", proofUrl: FOMC })]}
        locale="en"
      />,
    );
    expect(fomc).toContain('<span class="cal-event-flag">Postponed</span>');
    expect(fomc).toContain('<span class="cal-event-note">Press conference 2:30 PM ET</span>');
  });

  it("renders nothing for a day without events", () => {
    expect(renderToStaticMarkup(<EconomyEvents events={[]} locale="en" />)).toBe("");
  });

  it("sits above the day's earnings in the calendar week", () => {
    const out = renderToStaticMarkup(
      <CalendarWeek days={[{ date: "2026-11-10", entries: [] }]} events={{ "2026-11-10": [ev({})] }} locale="en" />,
    );
    expect(out.indexOf("cal-econ")).toBeGreaterThan(out.indexOf("cal-date"));
    expect(out.indexOf("cal-econ")).toBeLessThan(out.indexOf("cal-none"));
  });
});
