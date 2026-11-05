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
