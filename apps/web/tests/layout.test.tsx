import { existsSync, readFileSync, statSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IndexLineView } from "../src/lib/indexline";

// The language link reads the router; the shell test only needs to know it is in the nav.
vi.mock("../src/components/LangSwitchLink", () => ({
  default: ({ label, className }: { label: string; className: string }) => <a className={className}>{label}</a>,
}));

const { default: Masthead } = await import("../src/components/Masthead");
const { default: IndexLine } = await import("../src/components/IndexLine");
const { default: FooterLinks } = await import("../src/components/FooterLinks");

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
  // The as-of label compares the snapshot's Eastern day with today's: pin today to the snapshots' day.
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-11-04T22:00:00Z")); // Wed Nov 4, 5:00 PM EST
  });
  afterEach(() => {
    vi.useRealTimers();
  });

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

  it("links About and Privacy & Terms from the footer, in both locales", () => {
    const en = renderToStaticMarkup(<FooterLinks locale="en" />);
    expect(en).toContain('<a href="/en/about">About</a><a href="/en/legal">Privacy &amp; Terms</a>');
    const zh = renderToStaticMarkup(<FooterLinks locale="zh" />);
    expect(zh).toContain('<a href="/zh/about">关于</a><a href="/zh/legal">隐私与条款</a>');
    expect(en.indexOf("/en/legal")).toBeLessThan(en.indexOf("中文"));
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

  it("shows only the as-of time for a snapshot from today, in both locales", () => {
    const en = renderToStaticMarkup(<IndexLine line={recap} locale="en" />);
    expect(en).toContain('<time class="ix ix-asof" dateTime="2026-11-04T21:41:00Z">as of 4:41 PM ET</time>');
    const zh = renderToStaticMarkup(<IndexLine line={recap} locale="zh" />);
    expect(zh).toContain(">截至 下午4:41（美东）</time>");
  });

  it("adds the date to the as-of time for a snapshot from an earlier day, in both locales", () => {
    vi.setSystemTime(new Date("2026-11-05T14:00:00Z")); // Thu Nov 5, 9:00 AM EST, before the next snapshot
    const en = renderToStaticMarkup(<IndexLine line={recap} locale="en" />);
    expect(en).toContain(">as of Nov 4, 4:41 PM ET</time>");
    const zh = renderToStaticMarkup(<IndexLine line={recap} locale="zh" />);
    expect(zh).toContain(">截至 11月4日 下午4:41（美东）</time>");
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

/** The bodies of every `@media (min-width: 1000px)` block in globals.css (the calendar board's breakpoint). */
const boardBlocks = () => {
  const text = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  return [...text.matchAll(/@media \(min-width: 1000px\) \{\n([\s\S]*?)\n\}\n/g)].map((m) => m[1]);
};

describe("globals.css", () => {
  const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");

  it("drops the old brand colours, dark mode and the pill", () => {
    for (const gone of ["#00c805", "#ff5000", "prefers-color-scheme", ".pill", ".tile", ".estrip", ".grid-portal"]) {
      expect(css.toLowerCase()).not.toContain(gone);
    }
  });

  it("defines the paper palette tokens", () => {
    for (const hex of ["#faf8f3", "#1a1a1a", "#666", "#d9d5cc", "#222", "#15803d", "#b91c1c", "#8a1c1c", "#e8a317", "#e6e1d6"]) {
      expect(css.toLowerCase()).toContain(hex);
    }
  });

  it("keeps the flat cover colour as a token and styles the flat cover with it", () => {
    expect(css).toMatch(/:root \{[^}]*--cover-flat: #e6e1d6;/);
    expect(css.toLowerCase().split("#e6e1d6")).toHaveLength(2);
    expect(css).toMatch(/\.cover-flat \{[^}]*background: var\(--cover-flat\);[^}]*color: var\(--ink\);/);
    expect(css).not.toMatch(/\.cover-flat \.cover-(shade|stat|tag)/);
    expect(css).not.toMatch(/\.cover-flat \.cover-bar/);
  });

  it("draws covers on paper: contained graphic, tag in flow with a rule border, no shade", () => {
    const block = (sel: string) => css.match(new RegExp(`^${sel.replace(/\./g, "\\.")} \\{[^}]*\\}`, "m"))?.[0] ?? "";
    expect(block(".cover")).toContain("background: var(--paper)");
    expect(block(".cover")).toContain("color: var(--ink)");
    const bg = block(".cover-bg");
    expect(bg).toContain("object-fit: contain");
    expect(bg).toContain("object-position: center bottom");
    expect(bg).not.toContain("opacity");
    expect(bg).not.toContain("filter");
    expect(block(".cover-in")).toContain("justify-content: flex-start");
    const tag = block(".cover-tag");
    expect(tag).toContain("border: 1px solid var(--rule)");
    expect(tag).not.toContain("position: absolute");
    expect(block(".cover-stat")).toContain("color: var(--ink)");
    expect(css).not.toContain("cover-shade");
  });

  it("gives article and lead covers the graphic's shape, so the chart stays in the lower band under the lines", () => {
    const [desk, phone] = css.split(/@media \(max-width: 720px\) \{/);
    const rule = /^\s*\.cover-article, \.cover-lead \{ height: auto; aspect-ratio: ([\d /]+); \}/m;
    expect(desk.match(rule)?.[1]).toBe("16 / 9");
    expect(phone.match(rule)?.[1]).toBe("3 / 2");
    // The phone rule must come after the generic phone `.cover { height }`, or that height wins.
    expect(phone.indexOf(".cover-article, .cover-lead {")).toBeGreaterThan(phone.indexOf(".cover { height:"));
    // No fixed height on the two big covers anywhere else.
    expect(css).not.toMatch(/^\s*\.cover-(article|lead) \{ height: \d/m);
  });

  it("shows only the tag on a small cover with a graphic; the flat tile keeps its lines", () => {
    expect(css).toMatch(/^\.cover-small:not\(\.cover-flat\) \.cover-stat, \.cover-small:not\(\.cover-flat\) \.cover-bar \{ display: none; \}/m);
    expect(css).not.toMatch(/^\.cover-small \.cover-(stat|bar) \{[^}]*display: none/m);
  });

  it("lets the index line scroll in place at every width and never clips its left end", () => {
    const base = css.match(/^\.index-line \{[^}]*\}/m)?.[0] ?? "";
    expect(base).toContain("overflow-x: auto");
    // Its sr-only labels are absolutely positioned: without this they escape the scroller and widen the page.
    expect(base).toContain("position: relative");
    expect(base).not.toContain("justify-content");
    expect(css).toMatch(/\.ix:first-child \{ margin-left: auto; \}/);
  });

  it("uses no colour outside the palette, shadows only on calendar cards, and one mobile breakpoint at 720px", () => {
    const allowed = new Set(["#faf8f3", "#1a1a1a", "#666", "#d9d5cc", "#222", "#15803d", "#b91c1c", "#8a1c1c", "#e8a317", "#e6e1d6", "#fff", "#000"]);
    const hexes = (css.toLowerCase().match(/#[0-9a-f]{3,8}\b/g) ?? []).filter((h) => !allowed.has(h));
    expect(hexes).toEqual([]);
    expect(css).not.toMatch(/text-shadow/);
    // The calendar's company cards lift off the board with a soft ink shadow; nothing else casts one.
    const shadowed = [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{[^}]*box-shadow:\s*([^;}]+)/g)]
      .filter((m) => m[2].trim() !== "none")
      .map((m) => m[1].trim());
    expect(shadowed.length).toBeGreaterThan(0);
    for (const sel of shadowed) expect(sel).toMatch(/^\.cal-card-link/);
    const widths = [...css.matchAll(/@media[^{]*\(\s*(?:max|min)-width:\s*([\d.]+px)/g)].map((m) => m[1]);
    expect(new Set(widths)).toEqual(new Set(["720px", "1000px"]));
    // 1000px is the calendar board's own: everything under it, in every such block, is a calendar rule.
    const rules = boardBlocks().flatMap((block) => [...block.matchAll(/^\s+([^{\s/][^{]*)\{/gm)].map((m) => m[1].trim()));
    expect(rules.length).toBeGreaterThan(0);
    for (const sel of rules) expect(sel).toMatch(/^\.cal-/);
  });

  it("starts the phone's board at today without ever scrolling the desktop page there", () => {
    // Under 1000px the board is a sideways scroller and today's column is its initial target. From 1000px
    // the board is not a scroller, so the target would scroll the whole page down to today: it is turned off.
    expect(css).toMatch(/\.cal-col\.is-today \{[^}]*scroll-initial-target: nearest;/);
    expect(boardBlocks().join("\n")).toMatch(/\.cal-col\.is-today \{ scroll-initial-target: none; \}/);
  });

  it("keeps the day chips on screen above a column the reader jumps to", () => {
    expect(css).toMatch(/\.cal-chips \{ position: sticky; top: 0;[^}]*background: var\(--paper\);/);
    // A jump lands the column below the stuck chips (about 40px tall), not under them.
    expect(css).toMatch(/\.cal-col \{[^}]*scroll-margin-top: 3\.5rem;/);
  });
});

describe("site icon", () => {
  it("ships an SVG favicon and an Apple touch icon beside the app root", () => {
    expect(existsSync("src/app/icon.svg")).toBe(true);
    expect(readFileSync("src/app/icon.svg", "utf8")).toContain("#e8a317");
    expect(statSync("src/app/apple-icon.png").size).toBeGreaterThan(500);
  });
});
