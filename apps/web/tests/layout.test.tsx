import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { IndexLineView } from "../src/lib/indexline";

// The language link reads the router; the shell test only needs to know it is in the nav.
vi.mock("../src/components/LangSwitchLink", () => ({
  default: ({ label, className }: { label: string; className: string }) => <a className={className}>{label}</a>,
}));

const { default: Masthead } = await import("../src/components/Masthead");
const { default: IndexLine } = await import("../src/components/IndexLine");

const recap: IndexLineView = {
  kind: "recap",
  asOfUtc: "2026-11-04T21:41:00Z",
  entries: [
    { symbol: "^GSPC", last: 7688.62, changePct: 1.58, dir: "up" },
    { symbol: "^DJI", last: 48000.1, changePct: -0.4, dir: "down" },
    { symbol: "^RUT", last: 2500, changePct: 0, dir: "flat" },
  ],
};
const futures: IndexLineView = {
  kind: "premarket",
  asOfUtc: "2026-11-04T12:30:00Z",
  entries: [{ symbol: "ES=F", last: 7650.25, changePct: -1.14, dir: "down" }],
};

describe("paper shell", () => {
  it("renders masthead, dateline, nav and index line in order", () => {
    const out =
      renderToStaticMarkup(<Masthead locale="en" dateline="Wednesday, November 4, 2026 · After the close" />) +
      renderToStaticMarkup(<IndexLine line={recap} locale="en" />);
    const at = (s: string) => out.indexOf(s);
    expect(at('class="masthead"')).toBeGreaterThanOrEqual(0);
    expect(at("masthead-name")).toBeLessThan(at('class="dateline"'));
    expect(at('class="dateline"')).toBeLessThan(at('class="site-nav"'));
    expect(at('class="site-nav"')).toBeLessThan(at('class="index-line"'));
    expect(out).toContain("Wednesday, November 4, 2026 · After the close");
    for (const label of ["Briefs", "Earnings", "Calendar", "About", "中文"]) expect(out).toContain(label);
    expect(out).toContain('href="/en/archive?type=earnings"');
    expect(out).toContain('href="/en/earnings"');
  });

  it("puts the Chinese name first on the Chinese edition", () => {
    const en = renderToStaticMarkup(<Masthead locale="en" dateline="d" />);
    const zh = renderToStaticMarkup(<Masthead locale="zh" dateline="d" />);
    expect(en.indexOf("PreBell")).toBeLessThan(en.indexOf("盘前铃"));
    expect(zh.indexOf("盘前铃")).toBeLessThan(zh.indexOf("PreBell"));
    expect(zh).toContain('href="/zh"');
    expect(zh).toContain("English");
  });

  it("shows level, arrow and signed colour class per entry, with the as-of time", () => {
    const out = renderToStaticMarkup(<IndexLine line={recap} locale="en" />);
    expect(out).toContain("S&amp;P 500");
    expect(out).toContain("7,688.62");
    expect(out).toContain('<span class="num up"><span aria-hidden="true">▲</span><span class="sr-only">up </span>1.58%</span>');
    expect(out).toContain('<span class="num down"><span aria-hidden="true">▼</span><span class="sr-only">down </span>0.40%</span>');
    expect(out).toContain('<span class="num flat">0.00%</span>');
    expect(out).toContain("as of 4:41 PM ET");
  });

  it("omits the index line without a snapshot", () => {
    expect(renderToStaticMarkup(<IndexLine line={null} locale="en" />)).toBe("");
  });

  it("labels futures entries as futures", () => {
    const out = renderToStaticMarkup(<IndexLine line={futures} locale="en" />);
    expect(out).toContain("S&amp;P 500 futures");
    expect(out).toContain("7,650.25");
    expect(out).toContain("1.14%");
    const zh = renderToStaticMarkup(<IndexLine line={futures} locale="zh" />);
    expect(zh).toContain("标普500期货");
    expect(zh).toContain("美东");
  });
});

describe("globals.css", () => {
  const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");

  it("drops the old brand colours, dark mode and the pill", () => {
    for (const gone of ["#00c805", "#ff5000", "prefers-color-scheme", ".pill", ".tile", ".estrip", ".grid-portal"]) {
      expect(css.toLowerCase()).not.toContain(gone);
    }
  });

  it("defines the paper palette tokens", () => {
    for (const hex of ["#faf8f3", "#1a1a1a", "#666", "#d9d5cc", "#222", "#15803d", "#b91c1c", "#8a1c1c", "#e8a317"]) {
      expect(css.toLowerCase()).toContain(hex);
    }
  });

  it("lets the index line scroll in place at every width and never clips its left end", () => {
    const base = css.match(/^\.index-line \{[^}]*\}/m)?.[0] ?? "";
    expect(base).toContain("overflow-x: auto");
    // Its sr-only labels are absolutely positioned: without this they escape the scroller and widen the page.
    expect(base).toContain("position: relative");
    expect(base).not.toContain("justify-content");
    expect(css).toMatch(/\.ix:first-child \{ margin-left: auto; \}/);
  });

  it("uses no colour outside the palette, no shadows, and one mobile breakpoint at 720px", () => {
    const allowed = new Set(["#faf8f3", "#1a1a1a", "#666", "#d9d5cc", "#222", "#15803d", "#b91c1c", "#8a1c1c", "#e8a317", "#fff", "#000"]);
    const hexes = (css.toLowerCase().match(/#[0-9a-f]{3,8}\b/g) ?? []).filter((h) => !allowed.has(h));
    expect(hexes).toEqual([]);
    expect(css).not.toMatch(/box-shadow|text-shadow/);
    const widths = [...css.matchAll(/@media[^{]*\(\s*(?:max|min)-width:\s*([\d.]+px)/g)].map((m) => m[1]);
    expect(new Set(widths)).toEqual(new Set(["720px"]));
  });
});
