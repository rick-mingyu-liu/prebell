import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import EarningsStrip from "../src/components/EarningsStrip";
import type { StripDay } from "../src/lib/earnings";

describe("EarningsStrip", () => {
  const entry = (ticker: string, slot: "bmo" | "amc" | "unknown", postSlug: string | null = null) => ({
    ticker,
    date: "2026-11-02",
    slot,
    status: "confirmed" as const,
    postSlug,
  });
  const day: StripDay = {
    date: "2026-11-02",
    bmo: [entry("KO", "bmo", "ko-2026-11-02-earnings"), entry("BRK.B", "bmo")],
    amc: [entry("MSFT", "amc")],
    unknown: [entry("V", "unknown")],
  };

  it("renders nothing for no day", () => {
    expect(renderToStaticMarkup(<EarningsStrip day={null} locale="en" today="2026-11-02" />)).toBe("");
  });
  it("labels a future date, groups the chips and links to the calendar", () => {
    const out = renderToStaticMarkup(<EarningsStrip day={day} locale="en" today="2026-10-31" />);
    expect(out).toContain("Earnings · Mon, Nov 2");
    expect(out).toContain("Before open");
    expect(out).toContain("After close");
    expect(out).toContain("Time TBD");
    expect(out.indexOf("Before open")).toBeLessThan(out.indexOf("After close"));
    expect(out.indexOf("After close")).toBeLessThan(out.indexOf("Time TBD"));
    expect(out).toContain('href="/en/p/ko-2026-11-02-earnings"');
    expect(out).toContain('href="/en/t/MSFT"');
    expect(out).toContain('href="/en/t/BRK.B"');
    expect(out).toContain('href="/en/earnings"');
    expect(out).toContain("Full calendar");
  });
  it("says today when the date is today in ET", () => {
    const out = renderToStaticMarkup(<EarningsStrip day={day} locale="en" today="2026-11-02" />);
    expect(out).toContain("Earnings today");
    expect(out).not.toContain("Nov 2");
  });
  it("is localized in Chinese", () => {
    const out = renderToStaticMarkup(<EarningsStrip day={day} locale="zh" today="2026-10-31" />);
    expect(out).toContain("财报 · 11月2日 周一");
    expect(out).toContain("盘前");
    expect(out).toContain("盘后");
    expect(out).toContain("时间待定");
    expect(out).toContain('href="/zh/p/ko-2026-11-02-earnings"');
    expect(out).toContain('href="/zh/earnings"');
    expect(renderToStaticMarkup(<EarningsStrip day={day} locale="zh" today="2026-11-02" />)).toContain("今日财报");
  });
  it("omits empty groups", () => {
    const out = renderToStaticMarkup(<EarningsStrip day={{ ...day, bmo: [], unknown: [] }} locale="en" today="2026-10-31" />);
    expect(out).not.toContain("Before open");
    expect(out).not.toContain("Time TBD");
    expect(out).toContain("After close");
  });
  it("shows a logo on chips that have one and keeps the plain chip otherwise", () => {
    const out = renderToStaticMarkup(<EarningsStrip day={day} locale="en" today="2026-10-31" />);
    expect(out).toMatch(/<a class="estrip-chip" href="\/en\/p\/ko-2026-11-02-earnings"><img class="tlogo" src="\/logos\/companies\/KO.svg" alt=""[^>]*\/>KO<\/a>/);
    expect(out).toMatch(/<a class="estrip-chip" href="\/en\/t\/MSFT"><img class="tlogo" src="\/logos\/companies\/MSFT.png" alt=""[^>]*\/>MSFT<\/a>/);
    expect(out).toContain('<a class="estrip-chip" href="/en/t/BRK.B">BRK.B</a>');
    expect(out).not.toContain("tlogo-none");
  });
  it("caps chips per group and links the overflow to the calendar", () => {
    const many: StripDay = {
      ...day,
      bmo: Array.from({ length: 11 }, (_, i) => entry(`T${i}`, "bmo")),
    };
    const out = renderToStaticMarkup(<EarningsStrip day={many} locale="en" today="2026-10-31" />);
    expect(out).toContain(">T7<");
    expect(out).not.toContain(">T8<");
    expect(out).toMatch(/<a class="estrip-chip estrip-more" href="\/en\/earnings">\+3 more<\/a>/);
    expect(out).toContain("Full calendar");
    expect(out).toContain(">MSFT<"); // other groups are not squeezed out
    expect(renderToStaticMarkup(<EarningsStrip day={many} locale="zh" today="2026-10-31" />)).toContain("另有3家");
    expect(renderToStaticMarkup(<EarningsStrip day={day} locale="en" today="2026-10-31" />)).not.toContain("more");
  });
});
