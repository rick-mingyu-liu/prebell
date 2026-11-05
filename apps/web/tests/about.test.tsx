import { readFileSync } from "node:fs";
import { prerender } from "react-dom/static";
import { describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { TERMPOCKET_URL } from "../src/lib/partners";
import { translator } from "../src/lib/translator";

// The page reads messages through next-intl/server; swap in the synchronous translator.
const env = vi.hoisted(() => ({ locale: "en" as string }));
vi.mock("next-intl/server", () => ({
  setRequestLocale: (l: string) => {
    env.locale = l;
  },
  getTranslations: async (arg?: string | { locale: string; namespace?: string }) => {
    const ns = typeof arg === "string" ? arg : arg?.namespace;
    const t = translator((typeof arg === "object" ? arg.locale : env.locale) as Locale);
    return ns ? (k: string, v?: Record<string, unknown>) => t(`${ns}.${k}` as never, v as never) : t;
  },
}));

const { default: About } = await import("../src/app/[locale]/about/page");

// The page ends with the async Disclaimer server component, which only a streaming renderer can resolve.
const render = async (locale: Locale) => {
  const { prelude } = await prerender(await About({ params: Promise.resolve({ locale }) } as never));
  return new Response(prelude).text();
};
const at = (out: string, needle: string) => {
  const i = out.indexOf(needle);
  expect(i, needle).toBeGreaterThan(-1);
  return i;
};

describe("About page", () => {
  it("about page renders the author block, the TermPocket line and the signup", async () => {
    const out = await render("en");
    const order = [
      at(out, "<h1>About</h1>"),
      at(out, "PreBell publishes a short premarket brief"),
      at(out, "English is published first"),
      at(out, '<hr class="rule-strong"/>'),
      at(out, "<h2>Who is behind it</h2>"),
      at(out, 'class="author"'),
      at(out, "Rick Liu"),
      at(out, "From the PreBell team."),
      at(out, 'class="signup"'),
      at(out, 'class="disclaimer"'),
    ];
    expect(order).toEqual([...order].sort((a, b) => a - b));
    // One TermPocket line with the link.
    expect(out).toContain(`href="${TERMPOCKET_URL}"`);
    expect(out).toContain("Try TermPocket →");
    // The signup is the About one.
    expect(out).toContain("Get the weekly digest");
    expect(out).toContain("Content on this site is generated automatically");
  });

  it("renders in Chinese", async () => {
    const out = await render("zh");
    expect(out).toContain("<h2>关于作者</h2>");
    expect(out).toContain("来自 PreBell 团队。");
    expect(out).toContain("试用 TermPocket →");
    expect(out).toContain('href="/zh/archive"');
    expect(out).toContain("本站内容根据公开数据自动生成");
  });
});

describe("About article styles", () => {
  const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");

  it("sizes only its own paragraphs and headings, so the embedded author and signup blocks keep their type", () => {
    // `.prose p` would beat .author-name, .author-bio, .signup-copy and the like (specificity 0,1,1).
    expect(css).not.toMatch(/\.prose\s+(p|h[1-6])\b/);
    expect(css).toMatch(/^\.prose > p \{/m);
    expect(css).toMatch(/^\.prose > h2 \{/m);
  });

  it("puts the author and signup blocks directly under the article, not inside a paragraph", async () => {
    const out = await render("en");
    expect(out).toMatch(/<\/h2><section class="author">/);
    expect(out).not.toMatch(/<p(?: [^>]*)?>(?:(?!<\/p>).)*<section/s);
  });
});
