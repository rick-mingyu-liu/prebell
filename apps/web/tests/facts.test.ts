import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadFacts, parseFacts, type R2Like } from "../src/lib/facts";

const fixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const premarket = () => fixture("premarket-2026-10-30.json") as Record<string, unknown>;
const recap = () => fixture("recap-sample.json") as Record<string, unknown>;

describe("parseFacts premarket", () => {
  const v = parseFacts("premarket", premarket())!;

  it("reads the lead future", () => {
    expect(v.kind).toBe("premarket");
    expect(v.tradingDate).toBe("2026-10-30");
    expect(v.asOfUtc).toMatch(/^2026-10-30T/);
    expect(v.lead.symbol).toBe("ES=F");
    expect(v.lead.quote!.last).toBe(7665.5);
    expect(v.lead.quote!.changePct).toBeCloseTo(-1.1445, 3);
    expect(v.lead.quote!.dir).toBe("down");
    expect(v.lead.quote!.pointChange).toBe(-88.75);
  });

  it("reads the other futures in order", () => {
    expect(v.others.map((o) => o.symbol)).toEqual(["NQ=F", "YM=F", "RTY=F"]);
    expect(v.others.every((o) => o.quote !== null)).toBe(true);
  });

  it("reads 5 gainers and 5 losers with reasons", () => {
    expect(v.gainers).toHaveLength(5);
    expect(v.losers).toHaveLength(5);
    const mdlz = v.gainers.find((m) => m.symbol === "MDLZ")!;
    expect(mdlz.name).toBe("Mondelez International");
    expect(mdlz.quote.dir).toBe("up");
    expect(mdlz.reason).toBeTruthy();
    expect(mdlz.reasonUrl).toMatch(/^https:\/\//);
  });

  it("does not expose diagnostics", () => {
    expect(JSON.stringify(v)).not.toContain("diagnostics");
    expect("diagnostics" in v).toBe(false);
  });

  it("matches by symbol, not position", () => {
    const j = premarket();
    (j.futures as unknown[]).reverse();
    const r = parseFacts("premarket", j)!;
    expect(r.lead.symbol).toBe("ES=F");
    expect(r.others.map((o) => o.symbol)).toEqual(["NQ=F", "YM=F", "RTY=F"]);
  });

  it("gives quote null for Missing futures", () => {
    const j = premarket();
    const fut = j.futures as [string, unknown][];
    fut[3] = [fut[3][0], { Missing: { reason: "no data" } }];
    const r = parseFacts("premarket", j)!;
    expect(r.others[2]).toEqual({ symbol: "RTY=F", quote: null });
    expect(r.lead.quote).not.toBeNull();
  });

  it("falls back to position when a symbol is absent", () => {
    const j = premarket();
    const fut = j.futures as [string, { Value: Record<string, unknown> }][];
    fut[1][1].Value.symbol = "OTHER";
    const r = parseFacts("premarket", j)!;
    expect(r.others[0].symbol).toBe("NQ=F");
    expect(r.others[0].quote!.last).toBe(28769.75);
  });

  it("never yields negative zero point changes", () => {
    const j = premarket();
    const q = (j.futures as [string, { Value: Record<string, number> }][])[0][1].Value;
    q.last = q.prev_close - 0.001;
    const r = parseFacts("premarket", j)!;
    expect(Object.is(r.lead.quote!.pointChange, 0)).toBe(true);
  });

  it("keeps only http(s) reason URLs", () => {
    const j = premarket();
    const g = j.gainers as { reasons: { url: string }[] }[];
    g[0].reasons[0].url = "javascript:alert(1)";
    g[1].reasons[0].url = "data:text/html,x";
    g[2].reasons[0].url = "HTTP://example.com/a";
    const r = parseFacts("premarket", j)!;
    expect(r.gainers[0].reason).toBeTruthy();
    expect(r.gainers[0].reasonUrl).toBeNull();
    expect(r.gainers[1].reasonUrl).toBeNull();
    expect(r.gainers[2].reasonUrl).toBe("HTTP://example.com/a");
  });

  it("gives a mover without reasons null reason fields", () => {
    const j = premarket();
    (j.gainers as { reasons: unknown[] }[])[0].reasons = [];
    const r = parseFacts("premarket", j)!;
    expect(r.gainers[0].reason).toBeNull();
    expect(r.gainers[0].reasonUrl).toBeNull();
  });
});

const theme = (over: Record<string, unknown> = {}) => ({
  group: "Memory & storage",
  group_zh: "存储芯片",
  direction: "Down",
  members: [
    { ticker: "MU", name: "Micron Technology", change_pct: -4.12 },
    { ticker: "SNDK", name: "SanDisk", change_pct: -6.01 },
  ],
  more: 2,
  median_pct: -4.1,
  reasons: [],
  ...over,
});

describe("parseFacts themes", () => {
  it("is empty when the facts predate themes", () => {
    expect(parseFacts("premarket", premarket())!.themes).toEqual([]);
    expect(parseFacts("recap", recap())!.themes).toEqual([]);
  });

  it("reads themes in both kinds", () => {
    const up = theme({ group: "Chips", group_zh: "", direction: "Up", median_pct: 2.5, more: 0 });
    for (const [kind, j] of [["premarket", premarket()], ["recap", recap()]] as const) {
      const v = parseFacts(kind, { ...j, themes: [theme(), up], theme_candidates: [{ x: 1 }] })!;
      expect(v.themes).toHaveLength(2);
      expect(v.themes[0]).toEqual({
        group: "Memory & storage",
        groupZh: "存储芯片",
        direction: "down",
        medianPct: -4.1,
        members: [
          { symbol: "MU", changePct: -4.12 },
          { symbol: "SNDK", changePct: -6.01 },
        ],
        more: 2,
      });
      expect(v.themes[1]).toMatchObject({ direction: "up", groupZh: "Chips", more: 0 });
    }
  });

  it("skips a malformed entry and keeps the rest", () => {
    const bad = [
      theme({ direction: "Sideways" }),
      theme({ median_pct: null }),
      theme({ members: "MU" }),
      theme({ members: [{ ticker: "MU" }] }),
      theme({ members: [{ ticker: "MU", change_pct: "-4.1" }] }),
      theme({ members: [{ ticker: "  ", change_pct: -1 }] }),
      theme({ members: [] }),
      null,
    ];
    const v = parseFacts("premarket", { ...premarket(), themes: [bad[0], theme(), ...bad.slice(1)] })!;
    expect(v.themes.map((t) => t.group)).toEqual(["Memory & storage"]);
  });

  it("reads the member symbol from ticker first, then symbol", () => {
    const m = [{ ticker: "MU", symbol: "XX", change_pct: -1 }, { symbol: "SNDK", change_pct: -2 }];
    const v = parseFacts("premarket", { ...premarket(), themes: [theme({ members: m })] })!;
    expect(v.themes[0].members.map((x) => x.symbol)).toEqual(["MU", "SNDK"]);
  });

  it("treats a non-array themes field as none, without failing the facts", () => {
    const v = parseFacts("premarket", { ...premarket(), themes: { group: "x" } });
    expect(v).not.toBeNull();
    expect(v!.themes).toEqual([]);
  });
});

describe("parseFacts recap", () => {
  const v = parseFacts("recap", recap())!;

  it("reads index closes", () => {
    expect(v.kind).toBe("recap");
    expect(v.tradingDate).toBe("2026-10-29");
    expect(v.lead.symbol).toBe("^GSPC");
    expect(v.lead.quote!.last).toBe(6890.12);
    expect(v.lead.quote!.dir).toBe("up");
    expect(v.lead.quote!.pointChange).toBe(39.72);
    expect(v.others).toHaveLength(3);
    expect(v.others.map((o) => o.symbol)).toEqual(["^NDX", "^DJI", "^RUT"]);
    expect(v.others.map((o) => o.quote!.symbol)).toEqual(["^NDX", "^DJI", "^RUT"]);
    expect(v.others[0].quote!.last).toBe(25410.55);
    expect(v.others[1].quote!.dir).toBe("flat");
    expect(v.others[2].quote!.dir).toBe("down");
  });

  it("reports the symbol the facts carry, so an ^IXIC entry stays ^IXIC", () => {
    const j = recap();
    ((j.indexes as [string, { Value: Record<string, unknown> }][])[1][1].Value).symbol = "^IXIC";
    const r = parseFacts("recap", j)!;
    expect(r.others[0].symbol).toBe("^IXIC");
    expect(r.others[0].quote!.symbol).toBe("^IXIC");
  });

  it("names a Missing Nasdaq slot ^NDX, the generator's symbol", () => {
    const j = recap();
    const idx = j.indexes as [string, unknown][];
    idx[1] = [idx[1][0], { Missing: { reason: "x" } }];
    expect(parseFacts("recap", j)!.others[0]).toEqual({ symbol: "^NDX", quote: null });
  });

  it("reads movers", () => {
    expect(v.gainers.map((m) => m.symbol)).toEqual(["AAPL"]);
    expect(v.losers.map((m) => m.symbol)).toEqual(["PYPL", "XYZ"]);
    expect(v.losers[1].reason).toBeNull();
  });
});

describe("preview facts", () => {
  it.each(["2026-10-27", "2026-10-28"])("%s parses", (date) => {
    const json = JSON.parse(
      readFileSync(new URL(`../../../db/preview-facts/premarket-${date}.json`, import.meta.url), "utf8"),
    );
    const v = parseFacts("premarket", json)!;
    expect(v.tradingDate).toBe(date);
    expect(v.lead.quote).not.toBeNull();
  });
});

describe("parseFacts errors", () => {
  it("returns null when a number is null (serde writes NaN as null)", () => {
    const j = premarket();
    (j.futures as [string, { Value: Record<string, unknown> }][])[0][1].Value.change_pct = null;
    expect(parseFacts("premarket", j)).toBeNull();
  });

  it("returns null on any shape error", () => {
    expect(parseFacts("premarket", null)).toBeNull();
    expect(parseFacts("premarket", "x")).toBeNull();
    expect(parseFacts("premarket", {})).toBeNull();
    expect(parseFacts("premarket", { ...premarket(), futures: "no" })).toBeNull();
    expect(parseFacts("premarket", { ...premarket(), gainers: [{}] })).toBeNull();
    expect(parseFacts("premarket", { ...premarket(), trading_date: 5 })).toBeNull();
    expect(parseFacts("recap", premarket())).toBeNull();
    expect(parseFacts("premarket", recap())).toBeNull();
  });

  it("keeps a Missing lead as a null quote", () => {
    const j = premarket();
    const fut = j.futures as [string, unknown][];
    fut[0] = [fut[0][0], { Missing: { reason: "x" } }];
    const r = parseFacts("premarket", j)!;
    expect(r.lead).toEqual({ symbol: "ES=F", quote: null });
  });
});

function bucket(text: string | null): R2Like {
  return { get: async () => (text === null ? null : { text: async () => text }) };
}

describe("loadFacts", () => {
  it("loads and parses an object", async () => {
    const v = await loadFacts(bucket(JSON.stringify(premarket())), "premarket/2026-10-30.json", "premarket");
    expect(v?.lead.symbol).toBe("ES=F");
  });
  it("reads the key it was given", async () => {
    let got = "";
    const b: R2Like = { get: async (k) => ((got = k), null) };
    await loadFacts(b, "premarket/x.json", "premarket");
    expect(got).toBe("premarket/x.json");
  });
  it("returns null for a missing object", async () => {
    expect(await loadFacts(bucket(null), "k", "premarket")).toBeNull();
  });
  it("returns null for bad JSON", async () => {
    expect(await loadFacts(bucket("{not json"), "k", "premarket")).toBeNull();
  });
  it("returns null when the bucket throws", async () => {
    const b: R2Like = { get: async () => { throw new Error("r2 down"); } };
    expect(await loadFacts(b, "k", "premarket")).toBeNull();
    const sync: R2Like = { get: () => { throw new Error("r2 down"); } };
    expect(await loadFacts(sync, "k", "premarket")).toBeNull();
  });
  it("reports R2 errors, bad JSON and shape errors to onError, but not a missing object", async () => {
    const errs: unknown[] = [];
    const onError = (e: unknown) => errs.push(e);
    const boom = new Error("r2 down");
    await loadFacts({ get: async () => { throw boom; } }, "k", "premarket", onError);
    await loadFacts(bucket("{not json"), "k", "premarket", onError);
    await loadFacts(bucket("{}"), "k", "premarket", onError);
    expect(errs).toHaveLength(3);
    expect(errs[0]).toBe(boom);
    await loadFacts(bucket(null), "k", "premarket", onError);
    await loadFacts(bucket("{}"), null, "premarket", onError);
    expect(errs).toHaveLength(3);
  });
  it("still returns null when onError throws", async () => {
    const onError = () => { throw new Error("logger broke"); };
    expect(await loadFacts(bucket("{not json"), "k", "premarket", onError)).toBeNull();
  });
  it("returns null without a key or bucket", async () => {
    expect(await loadFacts(bucket("{}"), null, "premarket")).toBeNull();
    expect(await loadFacts(undefined, "k", "premarket")).toBeNull();
  });
});
