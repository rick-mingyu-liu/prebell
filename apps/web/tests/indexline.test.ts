import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadIndexLine, parseIndexLine } from "../src/lib/indexline";
import { memoryD1 } from "./d1";

const good = {
  kind: "recap",
  as_of_utc: "2026-11-04T21:41:00Z",
  entries: [
    { symbol: "^GSPC", last: 7688.62, change_pct: 1.58 },
    { symbol: "^DJI", last: 48000.1, change_pct: -0.4 },
    { symbol: "^RUT", last: 2500, change_pct: 0 },
  ],
};

describe("parseIndexLine", () => {
  it("accepts the generator shape and rejects junk", () => {
    expect(parseIndexLine(good)).toEqual({
      kind: "recap",
      asOfUtc: "2026-11-04T21:41:00Z",
      entries: [
        { symbol: "^GSPC", last: 7688.62, changePct: 1.58, dir: "up" },
        { symbol: "^DJI", last: 48000.1, changePct: -0.4, dir: "down" },
        { symbol: "^RUT", last: 2500, changePct: 0, dir: "flat" },
      ],
    });
    expect(parseIndexLine({ ...good, kind: "premarket" })?.kind).toBe("premarket");
    for (const bad of [
      null,
      "x",
      [],
      {},
      { ...good, kind: "weekly" },
      { ...good, as_of_utc: 5 },
      { ...good, entries: [] },
      { ...good, entries: "nope" },
      { ...good, entries: [{ symbol: "^GSPC", last: "1", change_pct: 1 }] },
      { ...good, entries: [{ symbol: "^GSPC", last: 1, change_pct: null }] },
      { ...good, entries: [{ symbol: "^GSPC", last: Infinity, change_pct: 1 }] },
      { ...good, entries: [{ symbol: 3, last: 1, change_pct: 1 }] },
      { ...good, entries: [null] },
    ]) {
      expect(parseIndexLine(bad)).toBeNull();
    }
  });
});

describe("loadIndexLine", () => {
  it("returns null without the kv row", async () => {
    expect(await loadIndexLine(memoryD1(""))).toBeNull();
  });

  it("parses the stored JSON and tolerates a corrupt value", async () => {
    const put = (v: string) =>
      memoryD1(`INSERT INTO kv VALUES ('index_line','${v.replace(/'/g, "''")}','2026-11-04T21:41:00Z');`);
    expect((await loadIndexLine(put(JSON.stringify(good))))?.entries).toHaveLength(3);
    expect(await loadIndexLine(put("{not json"))).toBeNull();
    expect(await loadIndexLine(put('{"kind":"x"}'))).toBeNull();
  });
});

describe("index line styles", () => {
  const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");
  const phones = css.indexOf("@media (max-width: 720px)");
  const desktopAt = css.indexOf("@media not all and (max-width: 720px)");
  const desktop = css.match(/@media not all and \(max-width: 720px\) \{\n([\s\S]*?)\n\}/)?.[1] ?? "";

  it("keeps the sideways scroll on phones and wraps centred rows on desktop widths", () => {
    expect(css).toMatch(/^\.index-line \{[^}]*display: flex;[^}]*overflow-x: auto;[^}]*white-space: nowrap/m);
    expect(desktop).toMatch(/\.index-line \{[^}]*display: block; text-align: center; white-space: normal/);
    expect(desktop).toMatch(/\.ix \{[^}]*display: inline-block;[^}]*white-space: nowrap/);
    // After the base rules it overrides, and ahead of the phone block.
    expect(desktopAt).toBeGreaterThan(css.indexOf(".ix:last-child { margin-right: auto; }"));
    expect(desktopAt).toBeLessThan(phones);
  });

  it("draws dividers on a separator, hidden on phones, so a wrapped row never starts or ends with one", () => {
    expect(css).toMatch(/^\.ix-sep \{ display: none; \}/m);
    expect(css).toMatch(/^\.ix-sep \+ \.ix \{ border-left: 1px solid var\(--rule\); \}/m);
    expect(css).not.toMatch(/\.ix \+ \.ix/);
    expect(desktop).toMatch(/\.ix-sep \{[^}]*display: inline;[^}]*linear-gradient\(var\(--rule\), var\(--rule\)\)/);
  });
});

describe("IndexLine markup", () => {
  const line = parseIndexLine({
    kind: "premarket",
    as_of_utc: "2026-11-12T13:41:00Z",
    entries: [
      { symbol: "ES=F", last: 6812.25, change_pct: 0.31 },
      { symbol: "NQ=F", last: 25134.5, change_pct: -1.42 },
    ],
  });

  it("puts one hidden one-space separator between items and none at either end", async () => {
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { default: IndexLine } = await import("../src/components/IndexLine");
    const html = renderToStaticMarkup(IndexLine({ line, locale: "en" }));
    expect(html.match(/<span class="ix-sep" aria-hidden="true"> <\/span>/g)).toHaveLength(2);
    expect(html).toMatch(/^<p class="index-line"><span class="ix">/);
    expect(html.match(/<span class="ix-sep"[^>]*> <\/span><(span|time) class="ix/g)).toHaveLength(2);
  });

  it("ends on an index, with no trailing separator, when the as-of time cannot be read", async () => {
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { default: IndexLine } = await import("../src/components/IndexLine");
    const html = renderToStaticMarkup(IndexLine({ line: { ...line!, asOfUtc: "garbage" }, locale: "en" }));
    expect(html).not.toContain("<time");
    expect(html.match(/<span class="ix"/g)).toHaveLength(2);
    expect(html.match(/class="ix-sep"/g)).toHaveLength(1);
    expect(html).toMatch(/<\/span><\/p>$/);
  });

  it("keeps the dividers in forced-colours mode, where gradient backgrounds are dropped", () => {
    const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");
    expect(css).toMatch(/@media \(forced-colors: active\) \{\s*\.ix-sep \{ forced-color-adjust: none; background-image: linear-gradient\(CanvasText, CanvasText\); \}/);
  });
});
