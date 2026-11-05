import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import Cover, { type CoverProps } from "../src/components/Cover";
import { companyLogo } from "../src/lib/logos";

const render = (over: Partial<CoverProps> = {}) =>
  renderToStaticMarkup(
    createElement(Cover, {
      coverKey: "covers/aapl-q4.png",
      lines: ["Revenue $94.9B", "Beat by 2.1%"],
      tag: { kind: "logo", ticker: "AAPL" },
      size: "lead",
      alt: "Apple earnings",
      ...over,
    }),
  );

describe("Cover", () => {
  it("renders scene, tag logo and two lines for an earnings cover", () => {
    const logo = companyLogo("AAPL");
    expect(logo).not.toBeNull();
    const out = render();
    expect(out).toContain('<figure class="cover cover-lead" aria-label="Apple earnings">');
    expect(out).toContain('<img class="cover-bg" src="/covers/aapl-q4.png" alt="" loading="lazy"/>');
    expect(out).toContain('<div class="cover-shade"></div>');
    expect(out).toContain('<div class="cover-in">');
    expect(out).toContain(`<span class="cover-tag"><img src="${logo}" alt="AAPL" loading="lazy"/></span>`);
    // A lazy image is not hoisted into a preload link.
    expect(out).not.toContain("<link");
    expect(out).toContain('<p class="cover-stat">Revenue $94.9B<br/>Beat by 2.1%</p>');
    expect(out).toContain('<span class="cover-bar"></span>');
  });

  it("renders a flat tag-only cover without key or lines", () => {
    const out = render({ coverKey: null, lines: null, tag: { kind: "label", text: "Nov 4 · Recap" }, size: "small" });
    expect(out).toContain('class="cover cover-small"');
    expect(out).not.toContain("<img");
    expect(out).not.toContain("cover-stat");
    expect(out).not.toContain("cover-bar");
    expect(out).toContain('<span class="cover-tag">Nov 4 · Recap</span>');
  });

  it("renders every key/lines combination", () => {
    const keyOnly = render({ lines: null });
    expect(keyOnly).toContain("cover-bg");
    expect(keyOnly).not.toContain("cover-stat");
    const linesOnly = render({ coverKey: null });
    expect(linesOnly).not.toContain("cover-bg");
    expect(linesOnly).toContain("cover-stat");
    expect(linesOnly).toContain("cover-bar");
  });

  it("falls back to the ticker text when no logo exists", () => {
    const out = render({ tag: { kind: "logo", ticker: "ZZZZNOLOGO" } });
    expect(out).toContain('<span class="cover-tag">ZZZZNOLOGO</span>');
  });

  it("renders nothing for an empty second line and treats an empty first line as no lines", () => {
    const one = render({ lines: ["Futures up 0.8%", ""] });
    expect(one).toContain('<p class="cover-stat">Futures up 0.8%</p>');
    expect(one).not.toContain("<br/>");
    const none = render({ lines: ["", "ignored"] });
    expect(none).not.toContain("cover-stat");
    expect(none).not.toContain("cover-bar");
  });

  it("supports the three sizes", () => {
    for (const size of ["lead", "article", "small"] as const) expect(render({ size })).toContain(`cover cover-${size}`);
  });
});
