import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  loadEarningsFacts,
  METRIC_KEYS,
  METRIC_LABELS,
  parseEarningsFacts,
  periodFromSlug,
  periodLabel,
} from "../src/lib/earningsfacts";

/** Built from `facts()` in crates/core/src/earnings/facts.rs (the Rust test fixture), as serde writes it. */
const base = () =>
  JSON.parse(readFileSync(new URL("./fixtures/earnings-AAPL-0000320193-26-000070.json", import.meta.url), "utf8"));
/** The same with the fields added after launch: a share move, a cause note with a table scale, a table_scale. */
const newer = () => JSON.parse(readFileSync(new URL("./fixtures/earnings-AAPL-share-move.json", import.meta.url), "utf8"));

describe("parseEarningsFacts", () => {
  it("reads the Rust fixture: figures with quotes, period, links, guidance", () => {
    const v = parseEarningsFacts(base(), "AAPL")!;
    expect(v.company).toBe("Apple");
    expect(v.filingDate).toBe("2026-10-30");
    expect(v.filingUrl).toMatch(/^https:\/\/www\.sec\.gov\/Archives\/edgar\/data\/320193\//);
    expect(v.pressReleaseUrl).toMatch(/a8-kex991q4202609262026\.htm$/);
    expect(v.fiscalPeriod).toEqual({
      quarter: 4,
      fiscalYear: 2026,
      quote: "financial results for its fiscal 2026 fourth quarter ended September 26, 2026",
    });
    expect(v.figures.map((f) => f.key)).toEqual(["revenue", "eps_gaap", "gross_margin"]);
    expect(v.figures[0]).toEqual({
      key: "revenue",
      value: 102.5e9,
      unit: "usd",
      amount: "$102.5 billion",
      quote: "The Company posted quarterly revenue of $102.5 billion, up 8 percent year over year",
    });
    expect(v.figures[1].unit).toBe("usd_per_share");
    expect(v.figures[2].unit).toBe("percent");
    expect(v.guidance).toEqual([
      {
        summaryEn: "Apple expects revenue growth of 10 to 12 percent.",
        quote: "the Company expects total revenue to grow 10 to 12 percent year over year",
      },
    ]);
    expect(v.highlights).toEqual([]);
    // Stored before the share move existed: no key at all.
    expect(v.shareMove).toBeNull();
  });

  it("reads the fields added after launch when present", () => {
    const v = parseEarningsFacts(newer(), "AAPL")!;
    expect(v.shareMove).toEqual({ session: "after_hours", changePct: -4.2, last: 251.3, asOf: "2026-10-30T21:05:00Z" });
    expect(v.figures.map((f) => f.key)).toContain("eps_non_gaap");
    expect(v.highlights).toEqual([{ summaryEn: "Services revenue was $28,750 million.", quote: "Services revenue $28,750 $24,972" }]);
    // The reference close is not passed on.
    expect(JSON.stringify(v)).not.toContain("262.32");
  });

  it("drops a malformed section instead of failing the whole check", () => {
    const f = base();
    f.metrics.revenue = { Value: { value: "lots", unit: "Usd", amount: "x", quote: "y" } };
    f.guidance = { Value: "oops" };
    f.fiscal_period = { Missing: { reason: "not stated" } };
    f.share_move = { Value: { session: "lunch", change_pct: 1, last: 2, as_of: "2026-10-30T21:05:00Z" } };
    f.exhibit_url = { Value: "javascript:alert(1)" };
    const v = parseEarningsFacts(f, "AAPL")!;
    expect(v.figures.map((x) => x.key)).toEqual(["eps_gaap", "gross_margin"]);
    expect(v.guidance).toEqual([]);
    expect(v.fiscalPeriod).toBeNull();
    expect(v.shareMove).toBeNull();
    expect(v.pressReleaseUrl).toBeNull();
  });

  it("refuses another company's facts or a non-object", () => {
    expect(parseEarningsFacts(base(), "MSFT")).toBeNull();
    expect(parseEarningsFacts([], "AAPL")).toBeNull();
    expect(parseEarningsFacts(null, "AAPL")).toBeNull();
  });
});

describe("loadEarningsFacts", () => {
  const KEY = "earnings/AAPL/0000320193-26-000070.json";
  const bucket = (body: string | null, fail = false) => ({
    gets: [] as string[],
    async get(key: string) {
      this.gets.push(key);
      if (fail) throw new Error("r2 down");
      return body === null ? null : { text: async () => body };
    },
  });

  it("reads one object", async () => {
    const b = bucket(JSON.stringify(base()));
    expect((await loadEarningsFacts(b, KEY, "AAPL"))?.company).toBe("Apple");
    expect(b.gets).toEqual([KEY]);
  });

  it("is null for a missing object, bad JSON, an R2 error or a key the generator does not write", async () => {
    const errors: unknown[] = [];
    const onError = (e: unknown) => errors.push(e);
    expect(await loadEarningsFacts(bucket(null), KEY, "AAPL", onError)).toBeNull();
    expect(errors).toHaveLength(0);
    expect(await loadEarningsFacts(bucket("{not json"), KEY, "AAPL", onError)).toBeNull();
    expect(await loadEarningsFacts(bucket("", true), KEY, "AAPL", onError)).toBeNull();
    expect(errors).toHaveLength(2);
    const b = bucket(JSON.stringify(base()));
    expect(await loadEarningsFacts(b, "../secrets.json", "AAPL")).toBeNull();
    expect(await loadEarningsFacts(b, null, "AAPL")).toBeNull();
    expect(await loadEarningsFacts(undefined, KEY, "AAPL")).toBeNull();
    expect(b.gets).toEqual([]);
  });
});

describe("periods", () => {
  it("labels in both languages", () => {
    expect(periodLabel({ quarter: 4, fiscalYear: 2026 }, "en")).toBe("Q4 FY2026");
    expect(periodLabel({ quarter: 4, fiscalYear: 2026 }, "zh")).toBe("2026财年第四季度");
    expect(periodLabel({ quarter: null, fiscalYear: 2026 }, "en")).toBe("FY2026");
    expect(periodLabel({ quarter: null, fiscalYear: 2026 }, "zh")).toBe("2026财年");
  });

  it("reads the period a slug was named from", () => {
    expect(periodFromSlug("aapl-q4-fy2026-earnings")).toEqual({ quarter: 4, fiscalYear: 2026 });
    expect(periodFromSlug("brk-b-fy2026-earnings")).toEqual({ quarter: null, fiscalYear: 2026 });
    expect(periodFromSlug("aapl-q4-fy2026-earnings-000070")).toEqual({ quarter: 4, fiscalYear: 2026 });
    expect(periodFromSlug("aapl-2026-10-30-earnings")).toBeNull();
  });
});

describe("metric labels match crates/core/src/earnings/facts.rs", () => {
  const rust = readFileSync(new URL("../../../crates/core/src/earnings/facts.rs", import.meta.url), "utf8");

  /** The `Metric::X => "..."` (or `=> &self.x`) arms of the Rust fn named `name`. */
  function arms(name: string, rhs: RegExp): Map<string, string> {
    const start = rust.indexOf(`fn ${name}(`);
    expect(start, name).toBeGreaterThan(-1);
    const body = rust.slice(start, rust.indexOf("\n    }\n", start));
    const out = new Map<string, string>();
    for (const m of body.matchAll(new RegExp(`Metric::(\\w+) => ${rhs.source}`, "g"))) out.set(m[1], m[2]);
    return out;
  }

  it("has the same keys and labels", () => {
    // Variant -> struct field (the JSON key), from `Metrics::get`.
    const field = arms("get", /&self\.(\w+)/);
    const en = arms("label_en", /"([^"]*)"/);
    const zh = arms("label_zh", /"([^"]*)"/);
    expect(field.size).toBe(8);
    expect([...field.values()].sort()).toEqual([...METRIC_KEYS].sort());
    for (const [variant, key] of field) {
      const label = METRIC_LABELS[key as keyof typeof METRIC_LABELS];
      expect(label, key).toBeDefined();
      expect(label.en, key).toBe(en.get(variant));
      expect(label.zh, key).toBe(zh.get(variant));
    }
  });
});
