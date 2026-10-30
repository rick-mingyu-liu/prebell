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

  it("gives a mover without reasons null reason fields", () => {
    const j = premarket();
    (j.gainers as { reasons: unknown[] }[])[0].reasons = [];
    const r = parseFacts("premarket", j)!;
    expect(r.gainers[0].reason).toBeNull();
    expect(r.gainers[0].reasonUrl).toBeNull();
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
    expect(v.others[0].quote!.last).toBe(25410.55);
    expect(v.others[1].quote!.dir).toBe("flat");
    expect(v.others[2].quote!.dir).toBe("down");
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
  it("returns null without a key or bucket", async () => {
    expect(await loadFacts(bucket("{}"), null, "premarket")).toBeNull();
    expect(await loadFacts(undefined, "k", "premarket")).toBeNull();
  });
});
