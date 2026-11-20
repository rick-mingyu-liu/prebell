import { readFileSync } from "node:fs";
import type { Metadata } from "next";
import { prerender } from "react-dom/static";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { cardPath } from "../src/lib/covers";
import { homeLd, jsonLd, newsArticleLd, AUTHOR } from "../src/lib/seo";
import { translator } from "../src/lib/translator";
import { memoryD1 } from "./d1";

// The pages read D1 through getDb(), R2 through the Cloudflare context and messages through
// next-intl/server; each is swapped for an in-memory or synchronous stand-in.
const env = vi.hoisted(() => ({ db: null as unknown, locale: "en" as string, cards: new Set<string>() }));
vi.mock("@/lib/db", () => ({ getDb: () => env.db }));
// R2 holds the cards in `env.cards`; nothing else.
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => ({
    env: { FACTS: { get: async () => null, head: async (k: string) => (env.cards.has(k) ? { key: k } : null) } },
  }),
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
// next/font needs the Next compiler; the layout only reads the class names.
vi.mock("@/lib/fonts", () => ({ inter: { variable: "f-inter" }, newsreader: { variable: "f-serif" } }));
vi.mock("../src/components/LangSwitchLink", () => ({ default: () => null }));

const post = await import("../src/app/[locale]/p/[slug]/page");
const home = await import("../src/app/[locale]/page");
const calendar = await import("../src/app/[locale]/earnings/page");
const ticker = await import("../src/app/[locale]/t/[ticker]/page");
const about = await import("../src/app/[locale]/about/page");
const layout = await import("../src/app/[locale]/layout");

const seed = `
INSERT INTO runs VALUES ('premarket:2026-11-05','premarket','2026-11-05','published',1,NULL,'2026-11-05T12:00:00Z','2026-11-05T12:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key, revised_at)
  VALUES (1,'premarket:2026-11-05','premarket','premarket:v1','2026-11-05','premarket-2026-11-05',NULL,'2026-11-05T12:01:00Z','covers/premarket-2026-11-05-update-1.svg','2026-11-05T12:37:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (1,'en','published','Futures slip <b> as yields rise','Stocks set to open lower.','# x','2026-11-05T12:37:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (1,'zh','published','收益率上升，期货走低','美股料低开。','# x','2026-11-05T12:37:00Z');
`;

const props = <P extends Record<string, string>>(params: P, search: Record<string, string> = {}) =>
  ({ params: Promise.resolve(params), searchParams: Promise.resolve(search) }) as never;
const html = async (el: React.ReactElement) => {
  const { prelude } = await prerender(el);
  return new Response(prelude).text();
};
/** The JSON-LD blocks in rendered HTML, parsed. */
const ldBlocks = (out: string) =>
  [...out.matchAll(/<script type="application\/ld\+json">(.*?)<\/script>/g)].map((m) => JSON.parse(m[1]));

type Og = NonNullable<Metadata["openGraph"]> & Record<string, unknown>;
type Tw = NonNullable<Metadata["twitter"]> & Record<string, unknown>;
const og = (m: Metadata) => m.openGraph as Og;
const tw = (m: Metadata) => m.twitter as Tw;

const CARD = "og/premarket-2026-11-05-update-1.png";

describe("share cards", () => {
  beforeEach(() => {
    env.db = memoryD1(seed);
    env.cards = new Set([CARD]);
  });

  it("a post is an article card in its language, with its rendered cover card and both times", async () => {
    for (const [locale, title, ogLocale, alt] of [
      ["en", "Futures slip <b> as yields rise", "en_US", "zh_CN"],
      ["zh", "收益率上升，期货走低", "zh_CN", "en_US"],
    ] as const) {
      const m = await post.generateMetadata(props({ locale, slug: "premarket-2026-11-05" }));
      expect(og(m)).toMatchObject({
        type: "article",
        url: `/${locale}/p/premarket-2026-11-05`,
        title,
        locale: ogLocale,
        alternateLocale: alt,
        siteName: translator(locale)("site.name"),
        publishedTime: "2026-11-05T12:01:00Z",
        modifiedTime: "2026-11-05T12:37:00Z",
        images: [{ url: "/og/premarket-2026-11-05-update-1.png", width: 1200, height: 630, type: "image/png", alt: title }],
      });
      expect(tw(m)).toMatchObject({
        card: "summary_large_image",
        title,
        images: [{ url: "/og/premarket-2026-11-05-update-1.png" }],
      });
    }
  });

  it("an IPO write-up with a cover names its own card in both languages", async () => {
    env.db = memoryD1(`${seed}
INSERT INTO runs VALUES ('ipo:anthropic:range_set:0001628280-26-072484','ipo','2026-11-10','published',1,NULL,'2026-11-11T22:00:00Z','2026-11-11T22:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key)
  VALUES (2,'ipo:anthropic:range_set:0001628280-26-072484','ipo','ipo-range_set','2026-11-10','anthropic-ipo-price-range-2026-11-10',NULL,'2026-11-11T22:00:00Z','covers/anthropic-ipo-price-range-2026-11-10.svg');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (2,'en','published','Anthropic''s IPO filing gives a price range of $70 to $78 a share','S.','# x','2026-11-11T22:00:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (2,'zh','published','Anthropic IPO文件载明价格区间：每股70至78美元','S.','# x','2026-11-11T22:00:00Z');`);
    env.cards = new Set(["og/anthropic-ipo-price-range-2026-11-10.png"]);
    for (const locale of ["en", "zh"] as const) {
      const props_ = props({ locale, slug: "anthropic-ipo-price-range-2026-11-10" });
      const m = await post.generateMetadata(props_);
      expect(og(m)).toMatchObject({ type: "article", images: [{ url: "/og/anthropic-ipo-price-range-2026-11-10.png", width: 1200, height: 630 }] });
      expect(tw(m).images).toEqual([expect.objectContaining({ url: "/og/anthropic-ipo-price-range-2026-11-10.png" })]);
      const out = await html(await post.default(props_));
      expect(ldBlocks(out)[0].image).toEqual(["https://prebell.site/og/anthropic-ipo-price-range-2026-11-10.png"]);
    }
  });

  it("an earnings check's search title names the company and period; its cards and JSON-LD keep the headline", async () => {
    env.db = memoryD1(`${seed}
INSERT INTO runs VALUES ('earnings:nvda','earnings','2026-11-17','published',1,NULL,'2026-11-17T21:00:00Z','2026-11-17T21:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
  VALUES (3,'earnings:nvda','earnings','earnings:v1','2026-11-17','nvda-q3-fy2027-earnings',NULL,'2026-11-17T21:26:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (3,'en','published','Nvidia''s quarterly revenue reaches $115.8 billion','S.','# x','2026-11-17T21:26:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (3,'zh','published','英伟达营收升至1158.4亿美元','S.','# x','2026-11-17T21:26:00Z');
INSERT INTO earnings_filings VALUES ('0001045810-26-000100',1045810,'NVDA','8-K','2.02','2026-11-17','2026-11-17','2026-11-17T21:05:00Z','x.htm','earnings:nvda','done','2026-11-17T21:06:00Z');`);
    const en = await post.generateMetadata(props({ locale: "en", slug: "nvda-q3-fy2027-earnings" }));
    expect(en.title).toBe("Nvidia Q3 fiscal 2027 earnings results");
    expect(og(en).title).toBe("Nvidia's quarterly revenue reaches $115.8 billion");
    expect(tw(en).title).toBe("Nvidia's quarterly revenue reaches $115.8 billion");
    const zh = await post.generateMetadata(props({ locale: "zh", slug: "nvda-q3-fy2027-earnings" }));
    expect(zh.title).toBe("Nvidia 2027财年第三季度财报");
    // A brief keeps its headline as the title.
    const brief = await post.generateMetadata(props({ locale: "en", slug: "premarket-2026-11-05" }));
    expect(brief.title).toBe("Futures slip <b> as yields rise");
  });

  it("a post names the default card until its own is rendered, so an early share never caches a URL that changes", async () => {
    env.cards = new Set();
    const props_ = props({ locale: "en", slug: "premarket-2026-11-05" });
    const m = await post.generateMetadata(props_);
    expect(og(m).images).toEqual([expect.objectContaining({ url: "/og-default.png" })]);
    expect(tw(m).images).toEqual([expect.objectContaining({ url: "/og-default.png" })]);
    const out = await html(await post.default(props_));
    expect(ldBlocks(out)[0].image).toEqual(["https://prebell.site/og-default.png"]);
  });

  it("home, calendar, ticker and about pages are website cards with the default card", async () => {
    const pages: [string, Metadata, string, string][] = [
      ["home", await home.generateMetadata(props({ locale: "en" })), "/en", "PreBell"],
      ["calendar", await calendar.generateMetadata(props({ locale: "zh" })), "/zh/earnings", "市场日历"],
      ["ticker", await ticker.generateMetadata(props({ locale: "en", ticker: "intc" })), "/en/t/INTC", "Intel (INTC) earnings"],
      ["about", await about.generateMetadata(props({ locale: "zh" })), "/zh/about", "关于"],
    ];
    for (const [name, m, url, title] of pages) {
      expect(og(m), name).toMatchObject({ type: "website", url, title, images: [{ url: "/og-default.png" }] });
      expect(og(m).description, name).toBeTruthy();
      expect(tw(m).card, name).toBe("summary_large_image");
    }
    expect(og(pages[1][1]).locale).toBe("zh_CN");
    expect(og(pages[2][1]).description).toBe("Intel (INTC) earnings dates and PreBell earnings checks, with links to the SEC filings.");
    // The page's own description, not the site tagline, for search results too.
    expect(pages[2][1].title).toBe("Intel (INTC) earnings");
    expect(pages[2][1].description).toBe(og(pages[2][1]).description);
    expect(pages[1][1].description).toBe(og(pages[1][1]).description);
    // A ticker outside the watchlist keeps the symbol-only title.
    const unknown = await ticker.generateMetadata(props({ locale: "zh", ticker: "zzzz" }));
    expect(unknown.title).toBe("ZZZZ 财报");
  });

  it("the layout's fallback card has no URL of its own", async () => {
    const m = await layout.generateMetadata(props({ locale: "en" }));
    expect(og(m)).toMatchObject({ type: "website", title: "PreBell", images: [{ url: "/og-default.png" }] });
    expect(og(m).url).toBeUndefined();
  });

  it("only SVG covers have a rendered card", () => {
    expect(cardPath("covers/recap-2026-11-04.svg")).toBe("/og/recap-2026-11-04.png");
    expect(cardPath("covers/recap-2026-10-30.png")).toBe("/og-default.png");
    expect(cardPath(null)).toBe("/og-default.png");
  });
});

describe("article markup", () => {
  beforeEach(() => {
    env.db = memoryD1(seed);
    env.cards = new Set([CARD]);
  });

  it("a post carries one NewsArticle with absolute URLs, its language and both dates", async () => {
    const out = await html(await post.default(props({ locale: "zh", slug: "premarket-2026-11-05" })));
    const ld = ldBlocks(out);
    expect(ld).toHaveLength(1);
    expect(ld[0]).toEqual({
      "@context": "https://schema.org",
      "@type": "NewsArticle",
      mainEntityOfPage: "https://prebell.site/zh/p/premarket-2026-11-05",
      headline: "收益率上升，期货走低",
      description: "美股料低开。",
      image: ["https://prebell.site/og/premarket-2026-11-05-update-1.png"],
      inLanguage: "zh-Hans",
      datePublished: "2026-11-05T12:01:00Z",
      dateModified: "2026-11-05T12:37:00Z",
      author: { "@type": "Person", name: "Rick Liu", url: "https://rickmingyuliu.com" },
      publisher: {
        "@type": "Organization",
        name: "PreBell",
        url: "https://prebell.site/",
        logo: { "@type": "ImageObject", url: "https://prebell.site/apple-icon.png", width: 180, height: 180 },
      },
    });
  });

  it("text can never close the script: < is escaped", async () => {
    const out = await html(await post.default(props({ locale: "en", slug: "premarket-2026-11-05" })));
    expect(out).toContain('"headline":"Futures slip \\u003cb> as yields rise"');
    expect(ldBlocks(out)[0].headline).toBe("Futures slip <b> as yields rise");
    expect(jsonLd({ a: "</script><script>" })).toBe('{"a":"\\u003c/script>\\u003cscript>"}');
  });

  it("the home page carries Organization and WebSite", async () => {
    const out = await html(await home.default(props({ locale: "en" })));
    const [ld] = ldBlocks(out);
    expect(ld.map((b: { "@type": string }) => b["@type"])).toEqual(["Organization", "WebSite"]);
    expect(ld[0]).toMatchObject({ name: "PreBell", url: "https://prebell.site/", founder: { name: "Rick Liu" } });
    expect(ld[1]).toMatchObject({ url: "https://prebell.site/en", inLanguage: "en", name: "PreBell" });
  });

  it("the author is the About page's", () => {
    expect(AUTHOR.name).toBe(translator("en")("author.name"));
    expect(AUTHOR.name).toBe(translator("zh")("author.name"));
    const src = readFileSync(new URL("../src/components/AuthorBlock.tsx", import.meta.url), "utf8");
    expect(src).toContain(`href: "${AUTHOR.url}"`);
    const origin = new URL("https://prebell.site");
    const a = newsArticleLd(origin, {
      url: "/en/p/x",
      headline: "h",
      description: "d",
      image: "/og-default.png",
      lang: "en",
      publishedAt: null,
      revisedAt: null,
    });
    expect(a).not.toHaveProperty("datePublished");
    const [, zh] = homeLd(origin, { url: "/zh", name: "盘前铃 PreBell", description: "d", lang: "zh-Hans" });
    expect(zh).toHaveProperty("alternateName", "PreBell");
    expect(homeLd(origin, { url: "/en", name: "PreBell", description: "d", lang: "en" })[1]).not.toHaveProperty("alternateName");
  });
});

describe("Cloudflare Web Analytics", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  const render = async () =>
    html(await layout.default({ children: <p>x</p>, params: Promise.resolve({ locale: "en" }) } as never));

  it("no beacon without CF_BEACON_TOKEN", async () => {
    vi.stubEnv("CF_BEACON_TOKEN", "");
    expect(await render()).not.toContain("cloudflareinsights");
  });

  it("with the token, one async beacon carrying the token", async () => {
    vi.stubEnv("CF_BEACON_TOKEN", "abc123");
    const out = await render();
    // React hoists an async script into <head>; async never blocks parsing or DOMContentLoaded.
    const tags = out.match(/<script[^>]*cloudflareinsights[^>]*>/g) ?? [];
    expect(tags).toHaveLength(1);
    expect(tags[0]).toContain('src="https://static.cloudflareinsights.com/beacon.min.js"');
    expect(tags[0]).toMatch(/\sasync[\s=>]/);
    expect(tags[0]).not.toContain("defer");
    expect(tags[0]).toContain(`data-cf-beacon="{&quot;token&quot;:&quot;abc123&quot;}"`);
    expect(out.match(/cloudflareinsights/g)).toHaveLength(1);
  });
});
