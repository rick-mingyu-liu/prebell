import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { translator } from "../src/lib/translator";
import { memoryD1 } from "./d1";

// The pages render with a fixed photo manifest (one CC BY-SA photo for INTC, one public-domain photo for
// XOM), so these tests do not depend on which photos are pinned. D1, the Worker env and next-intl are
// swapped for in-memory and synchronous stand-ins, as in pages.test.tsx and signuppages.test.tsx.
const env = vi.hoisted(() => ({ db: null as unknown, locale: "en" as string }));
vi.mock("@/lib/db", () => ({ getDb: () => env.db }));
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => ({ env: { FACTS: { get: async () => null, head: async () => null } } }),
}));
vi.mock("next/headers", () => ({ headers: async () => new Headers({ host: "prebell.site" }) }));
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
vi.mock("../src/lib/photos.generated", () => ({
  COMPANY_PHOTOS: {
    INTC: {
      src: "/photos/INTC.webp?v=0123abcd",
      width: 1440,
      height: 960,
      en: "Intel's headquarters in Santa Clara, California, in 2019.",
      zh: "Intel位于加利福尼亚州圣克拉拉的总部，摄于2019年。",
      author: "Jane Doe",
      licence: "CC BY-SA 4.0",
      licenceUrl: "https://creativecommons.org/licenses/by-sa/4.0",
      page: "https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg",
    },
    XOM: {
      src: "/photos/XOM.webp?v=89abcdef",
      width: 1000,
      height: 1250,
      en: "An Exxon station in Texas.",
      zh: "得克萨斯州的一座Exxon加油站。",
      author: "John Roe",
      licence: "Public domain",
      licenceUrl: null,
      page: "https://commons.wikimedia.org/wiki/File:Exxon_station.jpg",
    },
  },
}));

const post = await import("../src/app/[locale]/p/[slug]/page");
const ticker = await import("../src/app/[locale]/t/[ticker]/page");

const earningsPost = (id: number, tk: string, cik: number, acc: string, zhStatus = "published") => `
INSERT INTO runs VALUES ('earnings:${tk}','earnings','2026-10-29','published',1,NULL,'2026-10-29T21:00:00Z','2026-10-29T21:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
  VALUES (${id},'earnings:${tk}','earnings','earnings:v1','2026-10-29','earnings-${tk.toLowerCase()}-2026-10-29',NULL,'2026-10-29T21:10:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (${id},'en','published','${tk} third quarter','${tk} opening paragraph.','# ${tk} third quarter\n\n${tk} opening paragraph.\n\n## Results\n\nRevenue rose.','2026-10-29T21:10:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (${id},'zh','${zhStatus}','${tk} 第三季度','${tk} 开头一段。','# ${tk} 第三季度\n\n${tk} 开头一段。\n\n## 业绩\n\n营收增长。','2026-10-29T21:10:00Z');
INSERT INTO earnings_filings VALUES ('${acc}',${cik},'${tk}','8-K','2.02','2026-10-29','2026-10-29','2026-10-29T20:05:00Z','x.htm','earnings:${tk}','done','2026-10-29T20:06:00Z');
INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, first_seen_at, checked_at, updated_at)
  VALUES ('${tk}','2026-10-29','amc','reported','edgar','2026-10-01','2026-10-29','2026-10-29');
`;
const seed =
  earningsPost(1, "INTC", 50863, "0000050863-26-000041") +
  earningsPost(2, "AAPL", 320193, "0000320193-26-000070") +
  earningsPost(3, "XOM", 34088, "0000034088-26-000050", "pending") +
  `
INSERT INTO runs VALUES ('premarket:2026-10-29','premarket','2026-10-29','published',1,NULL,'2026-10-29T12:00:00Z','2026-10-29T12:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
  VALUES (4,'premarket:2026-10-29','premarket','premarket:v1','2026-10-29','premarket-2026-10-29',NULL,'2026-10-29T12:01:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (4,'en','published','Futures slip','Stocks set to open lower.','# Futures slip\n\nStocks set to open lower.','2026-10-29T12:01:00Z');
`;

const props = (params: Record<string, string>) => ({ params: Promise.resolve(params), searchParams: Promise.resolve({}) }) as never;
const html = async (el: React.ReactElement) => new Response((await prerender(el)).prelude).text();
const postPage = async (locale: Locale, slug: string) => html(await post.default(props({ locale, slug })));
const tickerPage = async (locale: Locale, tk: string) => renderToStaticMarkup(await ticker.default(props({ locale, ticker: tk })));
/** The page's JSON-LD blocks, joined. */
const ldOf = (out: string) => [...out.matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g)].map((m) => m[1]).join("\n");

beforeEach(() => {
  env.db = memoryD1(seed);
});

describe("an earnings check with a company photo", () => {
  it("shows the figure under the opening paragraph, with the caption as alt text, then the credit", async () => {
    const out = await postPage("en", "earnings-intc-2026-10-29");
    const figure = out.match(/<figure class="photo">[\s\S]*?<\/figure>/)?.[0] ?? "";
    expect(figure).toContain(
      '<img src="/photos/INTC.webp?v=0123abcd" alt="Intel&#x27;s headquarters in Santa Clara, California, in 2019." width="1440" height="960" loading="lazy" decoding="async"/>',
    );
    expect(figure).toContain(
      '<figcaption>Intel&#x27;s headquarters in Santa Clara, California, in 2019.<!-- --> <span class="photo-credit">Photo: <a href="https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg" rel="noopener">Jane Doe</a> <!-- -->/ Wikimedia Commons ·<!-- --> <a href="https://creativecommons.org/licenses/by-sa/4.0" rel="license noopener">CC BY-SA 4.0</a></span></figcaption>',
    );
    // Under the lede, before the rest of the body.
    const lede = out.indexOf('<p class="story-lede">INTC opening paragraph.</p>');
    expect(lede).toBeGreaterThan(0);
    expect(out.indexOf('<figure class="photo">')).toBeGreaterThan(lede);
    expect(out.indexOf('<figure class="photo">')).toBeLessThan(out.indexOf(">Results</h2>"));
    expect(out.match(/<figure class="photo">/g)).toHaveLength(1);
  });

  it("shows the Chinese caption and credit on the Chinese page", async () => {
    const out = await postPage("zh", "earnings-intc-2026-10-29");
    expect(out).toContain('alt="Intel位于加利福尼亚州圣克拉拉的总部，摄于2019年。"');
    expect(out).toContain('<span class="photo-credit">图片：<a href="https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg" rel="noopener">Jane Doe</a> <!-- -->/ Wikimedia Commons ·<!-- --> <a href="https://creativecommons.org/licenses/by-sa/4.0" rel="license noopener">CC BY-SA 4.0</a></span>');
    expect(out).not.toContain("Santa Clara");
  });

  it("never puts the photo in the metadata, the share cards or the JSON-LD", async () => {
    for (const locale of ["en", "zh"] as const) {
      const meta = await post.generateMetadata(props({ locale, slug: "earnings-intc-2026-10-29" }));
      expect(JSON.stringify(meta)).not.toContain("/photos/");
      expect(JSON.stringify(meta)).not.toContain("wikimedia");
      const out = await postPage(locale, "earnings-intc-2026-10-29");
      const ld = ldOf(out);
      expect(ld).toContain("NewsArticle");
      expect(ld).not.toContain("/photos/");
      expect(ld).not.toContain("wikimedia");
      // Outside the figure the page has no other trace of the photo (no preload, no head image).
      expect(out.replace(/<figure class="photo">[\s\S]*?<\/figure>/, "")).not.toMatch(/\/photos\/|wikimedia/i);
    }
    for (const locale of ["en", "zh"] as const) {
      const meta = await ticker.generateMetadata(props({ locale, ticker: "INTC" }));
      expect(JSON.stringify(meta)).not.toMatch(/\/photos\/|wikimedia/i);
    }
  });
});

describe("pages without a photo render as before", () => {
  it("an earnings check for a company without a photo has no figure, in both languages", async () => {
    for (const locale of ["en", "zh"] as const) {
      const out = await postPage(locale, "earnings-aapl-2026-10-29");
      expect(out).toContain('class="story-lede"');
      expect(out).not.toContain('class="photo"');
      expect(out).not.toMatch(/\/photos\/|wikimedia|photo-credit/i);
    }
  });

  it("a pending Chinese check shows no photo, and a brief never does", async () => {
    const pending = await postPage("zh", "earnings-xom-2026-10-29");
    expect(pending).not.toContain('class="photo"');
    // The English version of the same check does show it, with the public-domain credit.
    const en = await postPage("en", "earnings-xom-2026-10-29");
    expect(en).toContain("Photo: <a");
    expect(en).toContain("/ Wikimedia Commons ·<!-- --> <!-- -->Public domain</span>");
    expect(en).not.toContain('rel="license');
    const brief = await postPage("en", "premarket-2026-10-29");
    expect(brief).not.toMatch(/class="photo"|\/photos\//);
  });

  it("the ticker page of a company without a photo has no figure", async () => {
    for (const locale of ["en", "zh"] as const) {
      const out = await tickerPage(locale, "AAPL");
      expect(out).not.toMatch(/class="photo"|\/photos\/|wikimedia/i);
    }
  });
});

describe("the ticker page with a company photo", () => {
  it("shows the figure at the top, under the heading and the company name", async () => {
    const out = await tickerPage("en", "INTC");
    const figure = out.indexOf('<figure class="photo">');
    expect(figure).toBeGreaterThan(out.indexOf("<h1>INTC earnings</h1>"));
    expect(figure).toBeLessThan(out.indexOf('id="checks"'));
    expect(out).toContain('alt="Intel&#x27;s headquarters in Santa Clara, California, in 2019." width="1440" height="960" loading="lazy" decoding="async"');
    expect(out).toContain("Photo: <a");
  });

  it("in Chinese, with the Chinese caption and the public-domain credit translated", async () => {
    const out = await tickerPage("zh", "INTC");
    expect(out).toContain("<figcaption>Intel位于加利福尼亚州圣克拉拉的总部，摄于2019年。");
    const xom = await tickerPage("zh", "XOM");
    expect(xom).toContain("图片：<a href=\"https://commons.wikimedia.org/wiki/File:Exxon_station.jpg\" rel=\"noopener\">John Roe</a> / Wikimedia Commons · 公有领域</span>");
    // A 4:5 photo is narrowed so it is no taller than 520px.
    expect(xom).toContain('style="max-width:416px"');
  });
});
