import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import PostBody from "../src/components/PostBody";

const SOURCE = "https://news.test/aapl";
const html = (md: string, sources: string[] = [SOURCE]) =>
  renderToStaticMarkup(<PostBody markdown={md} sources={sources} />);

/** No link, image or preload in the output points at the attacker host. */
const expectNoEvil = (out: string) => {
  expect(out).not.toMatch(/(?:href|src|srcset|imagesrcset)="[^"]*evil/i);
  expect(out).not.toContain("<img");
  expect(out).not.toContain('rel="preload"');
};

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

  it("keeps a listed source as an external nofollow link and strips javascript: URLs", () => {
    const out = html(`[Apple](${SOURCE}) [x](javascript:alert(1)) [about](/en/about)`);
    expect(out).toContain(`<a href="${SOURCE}" rel="nofollow noopener" target="_blank">Apple</a>`);
    expect(out).not.toContain("javascript:");
    expect(out).toContain('<a href="/en/about">about</a>');
  });

  it("renders an unlisted external link as plain text", () => {
    const out = html("[Apple](https://news.test/other) [evil](https://evil.example/x)");
    expect(out).not.toContain("<a");
    expect(out).toContain("Apple");
    expectNoEvil(out);
  });

  it("does not link an upper-case scheme autolink", () => {
    const out = html("Visit HTTPS://evil.example/x now");
    expect(out).not.toContain("<a");
    expectNoEvil(out);
  });

  it("does not link a www. autolink", () => {
    const out = html("Visit www.evil.example now");
    expect(out).not.toContain("<a");
    expectNoEvil(out);
  });

  it("does not link a bare email (mailto)", () => {
    const out = html("Mail ceo@evil.example today");
    expect(out).not.toContain("<a");
    expect(out).not.toContain("mailto:");
    expectNoEvil(out);
  });

  it("drops reference-style links and images to a protocol-relative URL", () => {
    const out = html("Read [here][r] and see ![p][r].\n\n[r]: //evil.example/p.png");
    expect(out).not.toContain("<a");
    expect(out).toContain("here");
    expectNoEvil(out);
  });

  it("treats //host as external, not same-site", () => {
    const out = html("[x](//evil.example/a)");
    expect(out).not.toContain("<a");
    expectNoEvil(out);
    // "/\host" is percent-encoded by the parser into a same-site path, never a raw "/\".
    const slash = html("[y](/\\evil.example/b)");
    expect(slash).not.toMatch(/href="\/[/\\]/);
  });

  it("drops links and images inside GFM table cells", () => {
    const out = html(
      "| Ticker | Why |\n|---|---|\n| AAPL | [x](https://evil.example/a) ![i](https://evil.example/i.png) www.evil.example HTTPS://evil.example/b |\n",
    );
    expect(out).toContain("<table>");
    expect(out).not.toContain("<a");
    expectNoEvil(out);
  });

  it("drops inline images even from a listed source", () => {
    const out = html(`![chart](${SOURCE})`);
    expect(out).not.toContain("<img");
    expect(out).not.toContain('rel="preload"');
  });
});
