import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import MoversTable from "../src/components/MoversTable";
import type { MoverView } from "../src/lib/facts";
import { dirOf } from "../src/lib/format";

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

describe("MoversTable column", () => {
  it("shows name, grey ticker, coloured percent and a grey reason line on EN", () => {
    const out = renderToStaticMarkup(<MoversTable movers={[mover()]} locale="en" variant="column" />);
    expect(out).toContain('<ol class="movers-col">');
    expect(out).toContain('<b class="mover-name">Apple Inc.</b>');
    expect(out).toContain('<span class="mover-ticker muted">AAPL</span>');
    expect(out).toContain('<span class="num up">+6.12%</span>');
    expect(out).toContain('<p class="mover-why muted">Services revenue beat estimates</p>');
    expect(out).not.toContain("1,234.50");
  });

  it("keeps a flat move grey, and shows the price on request", () => {
    const out = renderToStaticMarkup(<MoversTable movers={[mover({ pct: 0 })]} locale="en" variant="column" showPrice />);
    expect(out).toContain('<span class="num flat">0.00%</span>');
    expect(out).toContain("$1,234.50");
  });

  it("says 'No clear news' in grey italics for a missing or blank reason", () => {
    for (const reason of [null, ""]) {
      const out = renderToStaticMarkup(<MoversTable movers={[mover({ reason })]} locale="en" variant="column" />);
      expect(out).toContain('<p class="mover-why muted"><i>No clear news</i></p>');
    }
  });

  it("links the reason only to an http(s) source", () => {
    const linked = renderToStaticMarkup(
      <MoversTable movers={[mover({ reasonUrl: "https://example.com/a?b=1&c=2" })]} locale="en" variant="column" />,
    );
    expect(linked).toContain(
      '<a href="https://example.com/a?b=1&amp;c=2" rel="nofollow noopener" target="_blank">Services revenue beat estimates</a>',
    );
    const unsafe = renderToStaticMarkup(
      <MoversTable movers={[mover({ reasonUrl: "javascript:alert(1)" })]} locale="en" variant="column" />,
    );
    expect(unsafe).not.toContain("<a ");
  });

  it("escapes the reason text", () => {
    const evil = "<img src=x onerror=alert(1)><script>alert(1)</script>";
    const out = renderToStaticMarkup(<MoversTable movers={[mover({ reason: evil })]} locale="en" variant="column" />);
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<script");
    expect(out).toContain("&lt;script&gt;");
  });

  it("shows no reason line on ZH, and nothing for an empty list", () => {
    const zh = renderToStaticMarkup(<MoversTable movers={[mover({ reason: null })]} locale="zh" variant="column" />);
    expect(zh).not.toContain("mover-why");
    expect(zh).not.toContain("暂无明确消息");
    expect(zh).toContain(">AAPL</span>");
    expect(renderToStaticMarkup(<MoversTable movers={[]} locale="en" variant="column" />)).toBe("");
  });
});

describe("MoversTable table", () => {
  it("renders a ruled table of company, reason, last price and change", () => {
    const out = renderToStaticMarkup(
      <MoversTable movers={[mover(), mover({ symbol: "HPQ", name: "HP Inc.", pct: -13.26, reason: null })]} locale="en" variant="table" showPrice />,
    );
    expect(out).toContain('<table class="table movers-table">');
    expect(out).toMatch(/<th[^>]*>Company<\/th><th[^>]*>Reason<\/th><th[^>]*>Last<\/th><th[^>]*>Change<\/th>/);
    expect(out).toContain('<td class="mover-why muted">Services revenue beat estimates</td>');
    expect(out).toContain('<td class="mover-why muted"><i>No clear news</i></td>');
    expect(out).toContain('<td class="r num">$1,234.50</td>');
    expect(out).toContain('<td class="r"><span class="num down">−13.26%</span></td>');
    expect(out).not.toMatch(/class="[^"]*\b(pill|tlogo)\b/);
  });

  it("heads the price column Close for a recap", () => {
    const en = renderToStaticMarkup(<MoversTable movers={[mover()]} locale="en" variant="table" showPrice priceLabel="close" />);
    expect(en).toMatch(/<th[^>]*>Close<\/th>/);
    const zh = renderToStaticMarkup(<MoversTable movers={[mover()]} locale="zh" variant="table" showPrice priceLabel="close" />);
    expect(zh).toMatch(/<th[^>]*>收盘价<\/th>/);
  });

  it("drops the reason column on ZH and the price column without showPrice", () => {
    const out = renderToStaticMarkup(<MoversTable movers={[mover()]} locale="zh" variant="table" />);
    expect(out).toMatch(/<th[^>]*>公司<\/th><th[^>]*>涨跌幅<\/th><\/tr>/);
    expect(out).not.toContain("Services revenue");
    expect(out).not.toContain("1,234.50");
  });
});
