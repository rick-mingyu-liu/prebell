import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import en from "../messages/en.json";
import zh from "../messages/zh.json";
import type { Locale } from "../src/i18n/routing";
import { translator } from "../src/lib/translator";

// The page reads its title through next-intl/server; swap in the synchronous translator.
const env = vi.hoisted(() => ({ locale: "en" as string }));
vi.mock("next-intl/server", () => ({
  setRequestLocale: (l: string) => {
    env.locale = l;
  },
  getTranslations: async ({ locale, namespace }: { locale: string; namespace: string }) => {
    const t = translator(locale as Locale);
    return (k: string) => t(`${namespace}.${k}` as never);
  },
}));

const { default: Legal, generateMetadata } = await import("../src/app/[locale]/legal/page");

const render = async (locale: Locale) =>
  renderToStaticMarkup(await Legal({ params: Promise.resolve({ locale }) } as never));

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/'/g, "&#x27;");

describe("Privacy & Terms page", () => {
  it("renders the title, the date and every section heading in both locales", async () => {
    for (const [locale, messages] of [["en", en], ["zh", zh]] as const) {
      const out = await render(locale);
      expect(out).toContain(`<h1>${esc(messages.legal.title)}</h1>`);
      expect(out).toContain(messages.legal.updated);
      const sections = [...messages.legal.privacy, ...messages.legal.terms];
      expect(sections).toHaveLength(17);
      for (const s of sections) expect(out, s.h).toContain(`<h2>${esc(s.h)}</h2>`);
      expect((out.match(/<h2>/g) ?? []).length).toBe(sections.length);
    }
  });

  it("carries the nature-of-content statement as the first terms section, in both locales", async () => {
    expect(en.legal.terms[0].h).toBe("Terms: nature of content");
    expect(zh.legal.terms[0].h).toBe("条款：内容性质声明");
    const enOut = await render("en");
    expect(enOut).toContain("<h2>Terms: nature of content</h2>");
    expect(enOut).toContain("not investment, financial, tax or legal advice");
    expect(enOut).toContain("professional adviser");
    const zhOut = await render("zh");
    expect(zhOut).toContain("<h2>条款：内容性质声明</h2>");
    expect(zhOut).toContain("不构成任何投资、财务、税务或法律建议");
    expect(zhOut).toContain("专业顾问");
  });

  it("states the dated English facts", async () => {
    const out = await render("en");
    expect(out).toContain("<h1>Privacy &amp; Terms</h1>");
    expect(out).toContain("Last updated: November 5, 2026");
    for (const fact of ["prebell-web.rickmyliu.workers.dev", "Buttondown", "Cloudflare Web Analytics", "Ontario, Canada", "rickmyliu@gmail.com", "under 16"]) {
      expect(out, fact).toContain(fact);
    }
  });

  it("states the Chinese facts and the Chinese date", async () => {
    const out = await render("zh");
    expect(out).toContain("<h1>隐私与条款</h1>");
    expect(out).toContain("最后更新：2026 年 11 月 5 日");
    for (const fact of ["Buttondown", "Cloudflare", "安大略省", "rickmyliu@gmail.com", "16 岁"]) expect(out, fact).toContain(fact);
  });

  it("keeps headings, paragraphs and lists direct children of the article, as the .prose rules need", async () => {
    const out = await render("en");
    expect(out.startsWith('<article class="prose prose-legal"><h1>')).toBe(true);
    expect(out).toMatch(/<\/h2><p>/);
    expect(out).toMatch(/<\/p><ul><li>/);
    expect(out).not.toMatch(/<(div|section)/);
    expect(out).toContain('<hr class="rule-strong"/>');
  });

  it("has a metadata title and paired alternates in both locales", async () => {
    const meta = async (locale: Locale) => generateMetadata({ params: Promise.resolve({ locale }) } as never);
    expect(await meta("en")).toMatchObject({
      title: "Privacy & Terms",
      alternates: { canonical: "/en/legal", languages: { en: "/en/legal", "zh-Hans": "/zh/legal", "x-default": "/en/legal" } },
    });
    expect(await meta("zh")).toMatchObject({ title: "隐私与条款", alternates: { canonical: "/zh/legal" } });
  });

  it("styles its list and spacing with palette tokens only", () => {
    const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");
    expect(css).toMatch(/^\.prose > ul \{/m);
    expect(css).toMatch(/^\.prose-legal > h2 \{/m);
  });
});
