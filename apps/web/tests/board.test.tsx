import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { CalendarEntry } from "../src/lib/earnings";

// The manifest as the wide-logo build writes it: a path per ticker and, for wordmarks, the natural size.
vi.mock("../src/lib/logos.generated", () => ({
  COMPANY_LOGOS: {
    WIDE: "/logos/companies/WIDE.svg",
    SQ: "/logos/companies/SQ.png",
    EDGE: "/logos/companies/EDGE.png",
  },
  COMPANY_LOGO_SIZES: { WIDE: [128, 32], EDGE: [86, 48] },
}));

const { default: CalendarWeek, WIDE_LOGO, wordmarkHeight } = await import("../src/components/CalendarWeek");
const { companyLogo, companyLogoInfo } = await import("../src/lib/logos");

const entry = (ticker: string, name: string | null): CalendarEntry => ({
  ticker,
  name,
  writeUps: true,
  date: "2026-11-02",
  slot: "bmo",
  status: "confirmed",
  postSlug: null,
  filingUrl: null,
});

describe("logos with sizes", () => {
  it("gives each logo its natural size, square when the manifest has none", () => {
    expect(companyLogo("wide")).toBe("/logos/companies/WIDE.svg");
    expect(companyLogoInfo("WIDE")).toEqual({ src: "/logos/companies/WIDE.svg", width: 128, height: 32 });
    expect(companyLogoInfo("SQ")).toEqual({ src: "/logos/companies/SQ.png", width: 48, height: 48 });
    expect(companyLogoInfo("NONE")).toBeNull();
    expect(companyLogoInfo("constructor")).toBeNull();
  });
});

describe("calendar cards with wide logos", () => {
  const out = renderToStaticMarkup(
    <CalendarWeek
      days={[{ date: "2026-11-02", entries: [entry("WIDE", "Wide Corp"), entry("SQ", "Square Inc"), entry("EDGE", null)] }]}
      locale="en"
    />,
  );
  const card = (t: string) => out.split("<li ").find((li) => li.includes(`/t/${t}"`))!;

  it("sets a wordmark across the card, named by its alt, with no ticker beside it", () => {
    expect(card("WIDE")).toContain('class="cal-card cal-card-wide cal-confirmed"');
    expect(card("WIDE")).toContain('<img class="cal-logo-wide" src="/logos/companies/WIDE.svg" alt="Wide Corp" width="128" height="32" style="height:22px" loading="eager"');
    expect(card("WIDE")).not.toContain("cal-card-sym");
  });

  it("sizes wordmarks for an even weight: long marks shorter, within 16-30px", () => {
    expect(wordmarkHeight(1.8)).toBe(30);
    expect(wordmarkHeight(2.6)).toBe(28);
    expect(wordmarkHeight(128 / 32)).toBe(22);
    expect(wordmarkHeight(5)).toBe(20);
    expect(wordmarkHeight(20)).toBe(16);
  });

  it("keeps a square or near-square logo as an icon beside the ticker and name", () => {
    expect(86 / 48).toBeLessThan(WIDE_LOGO);
    expect(card("SQ")).toContain('class="cal-card cal-card-icon cal-confirmed"');
    expect(card("SQ")).toContain('<img class="cal-logo-icon" src="/logos/companies/SQ.png" alt="" width="48" height="48"');
    expect(card("SQ")).not.toContain("style=");
    expect(card("SQ")).toContain('<span class="cal-card-name">Square Inc</span>');
    // Without a name the ticker beside the icon is what names the link.
    expect(card("EDGE")).toContain('class="cal-card cal-card-icon cal-confirmed"');
    expect(card("EDGE")).toContain('<span class="cal-card-sym">EDGE</span>');
  });
});
