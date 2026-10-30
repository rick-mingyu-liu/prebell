import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import PostBody from "../src/components/PostBody";
import { trimStory } from "../src/lib/trim";

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

describe("PostBody colorSigned", () => {
  const colored = (md: string, sources: string[] = [SOURCE]) =>
    renderToStaticMarkup(<PostBody markdown={md} sources={sources} colorSigned />);

  it("colors only explicitly signed percentages, keeping their text", () => {
    const out = colored("Futures rose 0.42% while the Dow was -1.14% and Nasdaq +2.50%; RTY −0.61%.");
    expect(out).toContain("rose 0.42% while");
    expect(out).not.toMatch(/<span[^>]*>0\.42%/);
    expect(out).toContain('<span class="num down">-1.14%</span>');
    expect(out).toContain('<span class="num up">+2.50%</span>');
    expect(out).toContain('<span class="num down">−0.61%</span>');
  });

  it("leaves ranges, zero and code alone", () => {
    const out = colored("A 5-7% range, a +0.00% move and `+3%` in code.");
    expect(out).toContain("5-7% range");
    expect(out).not.toMatch(/<span[^>]*>-7%/);
    expect(out).toContain('<span class="num flat">+0.00%</span>');
    expect(out).toContain("<code>+3%</code>");
  });

  it("is off by default", () => {
    expect(html("Down -1.14% today.")).not.toContain("<span");
  });

  it("keeps the link allowlist after trimStory and coloring", () => {
    const md = [
      "# Title [t](https://evil.example/t)",
      "",
      "Summary with [s](https://evil.example/s) +1.00%.",
      "",
      "| Ticker | Why |",
      "|---|---|",
      "| AAPL | [x](https://evil.example/a) -2.00% |",
      "",
      "## Futures",
      "",
      `Futures -1.14% per [Apple +23%](${SOURCE}), not [evil +5%](https://evil.example/x) or www.evil.example.`,
      "Same-site [about](/en/about) and ![img](https://evil.example/i.png).",
      "",
      "## Sources",
      "",
      `- [Apple](${SOURCE})`,
      "",
      "Not investment advice.",
    ].join("\n");
    const out = colored(trimStory(md));
    expectNoEvil(out);
    expect(out).not.toContain("<table");
    expect(out).not.toContain("Summary with");
    expect(out).toContain(
      `<a href="${SOURCE}" rel="nofollow noopener" target="_blank">Apple <span class="num up">+23%</span></a>`,
    );
    expect(out).toContain(`<a href="${SOURCE}" rel="nofollow noopener" target="_blank">Apple</a>`);
    expect(out).toContain('<a href="/en/about">about</a>');
    expect(out).toContain('evil <span class="num up">+5%</span>');
    expect(out).toContain('<span class="num down">-1.14%</span>');
    expect(out.match(/<a /g)).toHaveLength(3);
  });
});
