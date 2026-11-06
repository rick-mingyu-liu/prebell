import { readFileSync } from "node:fs";
import type { Metadata } from "next";
import { prerender } from "react-dom/static";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { cardPath, homeLd, jsonLd, newsArticleLd, AUTHOR } from "../src/lib/seo";
import { translator } from "../src/lib/translator";
import { memoryD1 } from "./d1";

// The pages read D1 through getDb(), R2 through the Cloudflare context and messages through
// next-intl/server; each is swapped for an in-memory or synchronous stand-in.
const env = vi.hoisted(() => ({ db: null as unknown, locale: "en" as string }));
vi.mock("@/lib/db", () => ({ getDb: () => env.db }));
vi.mock("@opennextjs/cloudflare", () => ({ getCloudflareContext: () => ({ env: { FACTS: { get: async () => null } } }) }));
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

describe("share cards", () => {
  beforeEach(() => {
    env.db = memoryD1(seed);
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
        images: [{ url: "/og/premarket-2026-11-05-update-1.png", width: 1200, height: 630, alt: title }],
      });
      expect(tw(m)).toMatchObject({
        card: "summary_large_image",
        title,
        images: [{ url: "/og/premarket-2026-11-05-update-1.png" }],
      });
    }
  });

  it("home, calendar, ticker and about pages are website cards with the default card", async () => {
    const pages: [string, Metadata, string, string][] = [
      ["home", await home.generateMetadata(props({ locale: "en" })), "/en", "PreBell"],
      ["calendar", await calendar.generateMetadata(props({ locale: "zh" })), "/zh/earnings", "财报日历"],
      ["ticker", await ticker.generateMetadata(props({ locale: "en", ticker: "intc" })), "/en/t/INTC", "INTC earnings"],
      ["about", await about.generateMetadata(props({ locale: "zh" })), "/zh/about", "关于"],
    ];
    for (const [name, m, url, title] of pages) {
      expect(og(m), name).toMatchObject({ type: "website", url, title, images: [{ url: "/og-default.png" }] });
      expect(og(m).description, name).toBeTruthy();
      expect(tw(m).card, name).toBe("summary_large_image");
    }
    expect(og(pages[1][1]).locale).toBe("zh_CN");
    expect(og(pages[2][1]).description).toBe("INTC earnings dates and PreBell earnings checks, with links to the SEC filings.");
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

  it("with the token, one deferred beacon at the end of the body, carrying the token", async () => {
    vi.stubEnv("CF_BEACON_TOKEN", "abc123");
    const out = await render();
    const tags = out.match(/<script[^>]*cloudflareinsights[^>]*>/g) ?? [];
    expect(tags).toHaveLength(1);
    expect(tags[0]).toContain('src="https://static.cloudflareinsights.com/beacon.min.js"');
    expect(tags[0]).toMatch(/\sdefer[\s=>]/);
    expect(tags[0]).toContain(`data-cf-beacon="{&quot;token&quot;:&quot;abc123&quot;}"`);
    expect(out.indexOf("cloudflareinsights")).toBeGreaterThan(out.indexOf('<footer class="site-footer">'));
  });
});
