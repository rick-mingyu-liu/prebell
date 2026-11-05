import { existsSync, statSync } from "node:fs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import nextConfig from "../next.config";
import AuthorBlock from "../src/components/AuthorBlock";
import { hasAuthorPhoto } from "../src/lib/author";

// next.config starts the Cloudflare dev proxy on import; the test only reads its env.
vi.mock("@opennextjs/cloudflare", () => ({ initOpenNextCloudflareForDev: () => {} }));

const photo = new URL("../public/author.jpg", import.meta.url);

/** AuthorBlock rendered against a stubbed `hasAuthorPhoto`, so both branches are tested whether or not the file ships. */
async function renderWithPhotoFlag(flag: boolean): Promise<string> {
  vi.resetModules();
  vi.doMock("../src/lib/author", () => ({ hasAuthorPhoto: flag }));
  try {
    const { default: Block } = await import("../src/components/AuthorBlock");
    return renderToStaticMarkup(createElement(Block, { locale: "en" }));
  } finally {
    vi.doUnmock("../src/lib/author");
    vi.resetModules();
  }
}

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

  it("shows the photo when the build says it was shipped", async () => {
    const out = await renderWithPhotoFlag(true);
    expect(out).toContain('<img class="author-photo" src="/author.jpg"');
  });

  it("renders no image when the photo was not shipped", async () => {
    const out = await renderWithPhotoFlag(false);
    expect(out).not.toContain("<img");
    expect(out).toContain("Rick Liu");
    expect(out).toContain('href="/en/archive"');
  });

  it("decides the photo flag from public/author.jpg, at build time and in tests alike", () => {
    const shipped = existsSync(photo);
    expect(nextConfig.env?.HAS_AUTHOR_PHOTO).toBe(shipped ? "1" : "0");
    expect(hasAuthorPhoto).toBe(shipped);
  });

  it("keeps a shipped photo under 60 KB", () => {
    // No photo ships until Rick supplies one; once he does, it must stay small.
    if (existsSync(photo)) expect(statSync(photo).size).toBeLessThan(60 * 1024);
  });
});
