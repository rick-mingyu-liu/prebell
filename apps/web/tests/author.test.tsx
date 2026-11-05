import { statSync } from "node:fs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import AuthorBlock from "../src/components/AuthorBlock";
import { hasAuthorPhoto } from "../src/lib/author";

const photo = new URL("../public/author.jpg", import.meta.url);

describe("AuthorBlock", () => {
  it("renders name, bio, three links and the archive link in both locales", () => {
    const en = renderToStaticMarkup(createElement(AuthorBlock, { locale: "en" }));
    expect(en).toContain("Rick Liu");
    expect(en).toContain(
      "AI &amp; software engineer, University of Waterloo. Built PreBell to read earnings the way an analyst would, without the analyst.",
    );
    expect(en).toContain('href="https://rickmingyuliu.com"');
    expect(en).toContain('href="https://github.com/rick-mingyu-liu"');
    expect(en).toContain('href="https://www.linkedin.com/in/rick-mingyu-liu/"');
    expect(en).toContain('href="/en/archive"');
    expect(en).toContain("All briefs →");

    const zh = renderToStaticMarkup(createElement(AuthorBlock, { locale: "zh" }));
    expect(zh).toContain("Rick Liu");
    expect(zh).toContain("滑铁卢大学 AI 与软件工程师");
    expect(zh).toContain('href="/zh/archive"');
    expect(zh).toContain("全部简报 →");
    expect(zh).toContain('href="https://github.com/rick-mingyu-liu"');
  });

  it("shows the photo only when public/author.jpg exists", () => {
    const out = renderToStaticMarkup(createElement(AuthorBlock, { locale: "en" }));
    expect(hasAuthorPhoto).toBe(true);
    expect(out).toContain('<img class="author-photo" src="/author.jpg"');
  });

  it("renders no image when the photo was not shipped", async () => {
    vi.resetModules();
    vi.doMock("../src/lib/author", () => ({ hasAuthorPhoto: false }));
    try {
      const { default: NoPhoto } = await import("../src/components/AuthorBlock");
      const out = renderToStaticMarkup(createElement(NoPhoto, { locale: "en" }));
      expect(out).not.toContain("<img");
      expect(out).toContain("Rick Liu");
      expect(out).toContain('href="/en/archive"');
    } finally {
      vi.doUnmock("../src/lib/author");
      vi.resetModules();
    }
  });

  it("public/author.jpg exists and is under 60 KB", () => {
    expect(statSync(photo).size).toBeLessThan(60 * 1024);
  });
});
