import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import EarningsStrip from "../src/components/EarningsStrip";
import MoversList from "../src/components/MoversList";
import NumbersBlock from "../src/components/NumbersBlock";
import Pill from "../src/components/Pill";
import type { StripDay } from "../src/lib/earnings";
import { parseFacts, type FactsView, type MoverView } from "../src/lib/facts";
import { dirOf } from "../src/lib/format";

const fixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const premarket = () => parseFacts("premarket", fixture("premarket-2026-10-30.json"))!;
const recap = () => parseFacts("recap", fixture("recap-sample.json"))!;

const mover = (over: Partial<MoverView> & { pct?: number } = {}): MoverView => {
  const pct = over.pct ?? 6.12;
  return {
    symbol: "AAPL",
    name: "Apple Inc.",
    quote: { symbol: "AAPL", last: 1234.5, changePct: pct, pointChange: 1, dir: dirOf(pct) },
    reason: "Services revenue beat estimates",
    reasonUrl: null,
    ...over,
  };
};

describe("Pill", () => {
  it("shows a down move with the real minus sign", () => {
    const out = renderToStaticMarkup(<Pill pct={-26} />);
    expect(out).toContain("−26.00%");
    expect(out).toMatch(/class="[^"]*\bpill\b[^"]*\bdown\b/);
  });
  it("shows an up move with a plus sign", () => {
    const out = renderToStaticMarkup(<Pill pct={1.14} />);
    expect(out).toContain("+1.14%");
    expect(out).toMatch(/class="[^"]*\bup\b/);
  });
  it("is grey and unsigned when the move rounds to zero", () => {
    const out = renderToStaticMarkup(<Pill pct={0.004} />);
    expect(out).toContain(">0.00%<");
    expect(out).toMatch(/class="[^"]*\bflat\b/);
  });
});

describe("NumbersBlock", () => {
  it("renders the lead label, ET time, number, point change and percent (EN)", () => {
    const out = renderToStaticMarkup(<NumbersBlock facts={premarket()} variant="home" locale="en" />);
    expect(out).toContain("S&amp;P 500 futures");
    expect(out).toMatch(/\d{1,2}:\d{2} (AM|PM) ET/);
    expect(out).toContain("7,665.50");
    expect(out).toContain("−88.75");
    expect(out).toContain("−1.14%");
    expect(out).toMatch(/class="[^"]*\bnum\b[^"]*\bdown\b/);
  });

  it("uses Chinese index names on ZH pages", () => {
    const out = renderToStaticMarkup(<NumbersBlock facts={premarket()} variant="home" locale="zh" />);
    expect(out).toContain("标普500期货");
    expect(out).toContain("纳指100期货");
    expect(out).toContain("（美东）");
  });

  it("names the recap indexes, Nasdaq 100 included", () => {
    const out = renderToStaticMarkup(<NumbersBlock facts={recap()} variant="home" locale="en" />);
    expect(out).toContain("S&amp;P 500");
    expect(out).toContain("Nasdaq 100");
    expect(out).toContain("Russell 2000");
    expect(out).not.toContain("futures");
  });

  it("reserves an empty chart slot", () => {
    const out = renderToStaticMarkup(<NumbersBlock facts={premarket()} variant="home" locale="en" />);
    expect(out).toMatch(/<div class="chart-slot"[^>]*><\/div>/);
  });

  it("shows the other three as tiles on home and rows on the page", () => {
    const home = renderToStaticMarkup(<NumbersBlock facts={premarket()} variant="home" locale="en" />);
    const page = renderToStaticMarkup(<NumbersBlock facts={premarket()} variant="page" locale="en" />);
    expect(home.match(/class="tile"/g)).toHaveLength(3);
    expect(home).not.toContain('class="row"');
    expect(page.match(/class="row"/g)).toHaveLength(3);
    expect(page).not.toContain('class="tile"');
  });

  it("colors tiles and rows by direction", () => {
    const facts: FactsView = { ...premarket() };
    const q = (symbol: string, changePct: number) => ({
      symbol,
      last: 100,
      changePct,
      pointChange: changePct,
      dir: dirOf(changePct),
    });
    facts.others = [
      { symbol: "NQ=F", quote: q("NQ=F", 1.5) },
      { symbol: "YM=F", quote: q("YM=F", -1.5) },
      { symbol: "RTY=F", quote: q("RTY=F", 0) },
    ];
    for (const variant of ["home", "page"] as const) {
      const out = renderToStaticMarkup(<NumbersBlock facts={facts} variant={variant} locale="en" />);
      expect(out).toMatch(/class="num up">\+1\.50%</);
      expect(out).toMatch(/class="num down">−1\.50%</);
      expect(out).toMatch(/class="num flat">0\.00%</);
    }
  });

  it("puts the as-of timestamp on the time element", () => {
    const out = renderToStaticMarkup(<NumbersBlock facts={premarket()} variant="home" locale="en" />);
    expect(out).toMatch(/<time dateTime="2026-10-30T13:20:43\.115Z">/);
  });

  it("renders a missing index as a grey dash", () => {
    const facts: FactsView = { ...premarket() };
    facts.others = [{ symbol: "NQ=F", quote: null }, ...facts.others.slice(1)];
    for (const variant of ["home", "page"] as const) {
      const out = renderToStaticMarkup(<NumbersBlock facts={facts} variant={variant} locale="en" />);
      expect(out).toMatch(/<[^>]*class="[^"]*\bflat\b[^"]*"[^>]*>—</);
      expect(out).toContain("Nasdaq 100 futures");
    }
  });

  it("renders a missing lead as a grey dash", () => {
    const facts: FactsView = { ...premarket(), lead: { symbol: "ES=F", quote: null } };
    const out = renderToStaticMarkup(<NumbersBlock facts={facts} variant="home" locale="en" />);
    expect(out).toMatch(/<[^>]*class="[^"]*\bflat\b[^"]*"[^>]*>—</);
    expect(out).toContain("S&amp;P 500 futures");
  });

  it("shows a flat quote with no sign on the point change", () => {
    const facts: FactsView = { ...premarket() };
    facts.lead = {
      symbol: "ES=F",
      quote: { symbol: "ES=F", last: 100, changePct: 0.001, pointChange: 0.3, dir: "flat" },
    };
    const out = renderToStaticMarkup(<NumbersBlock facts={facts} variant="home" locale="en" />);
    expect(out).toContain("0.30 (0.00%)");
    expect(out).not.toContain("+0.30");
  });
});

describe("MoversList", () => {
  it("shows ticker, name, reason and pill on EN", () => {
    const out = renderToStaticMarkup(<MoversList movers={[mover()]} locale="en" />);
    expect(out).toContain("<b>AAPL</b>");
    expect(out).toContain("Apple Inc.");
    expect(out).toContain("Services revenue beat estimates");
    expect(out).toContain("+6.12%");
  });

  it("shows a flat mover in grey", () => {
    const out = renderToStaticMarkup(<MoversList movers={[mover({ pct: 0 })]} locale="en" />);
    expect(out).toMatch(/class="[^"]*\bpill\b[^"]*\bflat\b/);
    expect(out).not.toMatch(/\b(up|down)\b"/);
  });

  it("says 'No clear news' on EN when there is no reason", () => {
    const out = renderToStaticMarkup(<MoversList movers={[mover({ reason: null })]} locale="en" />);
    expect(out).toContain("No clear news");
    expect(out).not.toContain("muted\">No clear news");
    const blank = renderToStaticMarkup(<MoversList movers={[mover({ reason: "" })]} locale="en" />);
    expect(blank).toContain("No clear news");
  });

  it("shows no reason line on ZH", () => {
    const withReason = renderToStaticMarkup(<MoversList movers={[mover()]} locale="zh" />);
    const without = renderToStaticMarkup(<MoversList movers={[mover({ reason: null })]} locale="zh" />);
    expect(withReason).not.toContain("Services revenue");
    expect(withReason).not.toContain('class="why"');
    expect(without).not.toContain("No clear news");
    expect(withReason).toContain("<b>AAPL</b>");
    expect(withReason).toContain("+6.12%");
  });

  it("escapes the reason text", () => {
    const evil = '<img src=x onerror=alert(1)><script>alert(1)</script>';
    const out = renderToStaticMarkup(<MoversList movers={[mover({ reason: evil })]} locale="en" />);
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<script");
    expect(out).toContain("&lt;script&gt;");
  });

  it("honours limit and showPrice", () => {
    const movers = [mover({ symbol: "A" }), mover({ symbol: "B" }), mover({ symbol: "C" })];
    const out = renderToStaticMarkup(<MoversList movers={movers} locale="en" limit={2} showPrice />);
    expect(out).toContain("<b>A</b>");
    expect(out).toContain("<b>B</b>");
    expect(out).not.toContain("<b>C</b>");
    expect(out).toContain("$1,234.50");
    const noPrice = renderToStaticMarkup(<MoversList movers={movers} locale="en" />);
    expect(noPrice).not.toContain("1,234.50");
  });

  it("renders nothing for an empty list", () => {
    expect(renderToStaticMarkup(<MoversList movers={[]} locale="en" />)).toBe("");
  });
});

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
