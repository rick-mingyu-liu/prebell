import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import PostBody from "../src/components/PostBody";

const html = (md: string) => renderToStaticMarkup(<PostBody markdown={md} />);

describe("PostBody", () => {
  it("renders GFM tables", () => {
    const out = html("| Contract | Last | Change |\n|---|---:|---:|\n| S&P 500 | 5,912.25 | +0.42% |\n");
    expect(out).toContain('<div class="table-wrap"><table>');
    expect(out).toContain("<th>Contract</th>");
    expect(out).toContain('<td style="text-align:right">+0.42%</td>');
  });

  it("does not render raw HTML", () => {
    const out = html('Hi <script>alert(1)</script> <b onclick="x()">bold</b>');
    expect(out).not.toContain("<script");
    expect(out).not.toContain("<b");
  });

  it("marks external links nofollow and strips javascript: URLs", () => {
    const out = html("[Apple](https://news.test/aapl) [x](javascript:alert(1)) [about](/en/about)");
    expect(out).toContain('<a href="https://news.test/aapl" rel="nofollow noopener" target="_blank">Apple</a>');
    expect(out).not.toContain("javascript:");
    expect(out).toContain('<a href="/en/about">about</a>');
  });
});
