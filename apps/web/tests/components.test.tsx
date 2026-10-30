import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import MoversList from "../src/components/MoversList";
import NumbersBlock from "../src/components/NumbersBlock";
import Pill from "../src/components/Pill";
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
    expect(out).toContain("1,234.50");
    const noPrice = renderToStaticMarkup(<MoversList movers={movers} locale="en" />);
    expect(noPrice).not.toContain("1,234.50");
  });

  it("renders nothing for an empty list", () => {
    expect(renderToStaticMarkup(<MoversList movers={[]} locale="en" />)).toBe("");
  });
});
