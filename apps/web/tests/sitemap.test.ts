import { resolveRobots, resolveSitemap } from "next/dist/build/webpack/loaders/metadata/resolve-route-data";
import { describe, expect, it, vi } from "vitest";
import { cachedSitemap, type EdgeCache, sitemapEntries, sitemapXml } from "../src/lib/sitemap";
import { memoryD1 } from "./d1";

vi.mock("next/headers", () => ({ headers: async () => new Headers({ host: "prebell.site" }) }));

const run = (key: string, type: string, date: string) =>
  `INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${date}T12:00:00Z','${date}T12:00:00Z');`;
const tr = (id: number, lang: string, status: string) =>
  `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'${lang}','${status}','t','s','# t','x');`;

const seed = [
  run("premarket:2026-11-05", "premarket", "2026-11-05"),
  run("recap:2026-11-04", "recap", "2026-11-04"),
  run("earnings:intc", "earnings", "2026-10-29"),
  run("recap:2026-11-03", "recap", "2026-11-03"),
  `INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, published_at, revised_at) VALUES
    (1,'premarket:2026-11-05','premarket','v1','2026-11-05','premarket-2026-11-05','2026-11-05T12:01:00Z','2026-11-05T12:37:00Z'),
    (2,'recap:2026-11-04','recap','v1','2026-11-04','recap-2026-11-04','2026-11-04T21:00:00Z',NULL),
    (3,'earnings:intc','earnings','v1','2026-10-29','earnings-intc-2026-10-29','2026-10-29T21:10:00Z',NULL),
    (4,'recap:2026-11-03','recap','v1','2026-11-03','recap-2026-11-03',NULL,NULL);`,
  tr(1, "en", "published"),
  tr(1, "zh", "published"),
  tr(2, "en", "published"),
  tr(2, "zh", "pending"),
  tr(3, "en", "published"),
  tr(3, "zh", "published"),
  tr(4, "en", "failed"),
  `INSERT INTO earnings_filings VALUES ('0000050863-26-000041',50863,'INTC','8-K','2.02','2026-10-29','2026-10-29','2026-10-29T20:05:00Z','x.htm','earnings:intc','done','2026-10-29T20:06:00Z');`,
  // A calendar-only company: dates but no post, so its noindex page stays out.
  `INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, first_seen_at, checked_at, updated_at)
    VALUES ('A','2026-10-28','amc','reported','edgar','2026-10-01','2026-10-28','2026-10-28');`,
].join("\n");

const origin = new URL("https://prebell.site");

describe("sitemap", () => {
  it("lists the fixed pages, each published translation and the checked tickers, in both languages", async () => {
    const entries = await sitemapEntries(memoryD1(seed), origin);
    const urls = entries.map((e) => e.url.replace("https://prebell.site", ""));
    expect(urls).toEqual([
      "/en",
      "/zh",
      "/en/archive",
      "/zh/archive",
      "/en/earnings",
      "/zh/earnings",
      "/en/about",
      "/zh/about",
      "/en/legal",
      "/zh/legal",
      "/en/p/premarket-2026-11-05",
      "/zh/p/premarket-2026-11-05",
      // The Chinese recap is pending (noindex): English only, not paired.
      "/en/p/recap-2026-11-04",
      "/en/p/earnings-intc-2026-10-29",
      "/zh/p/earnings-intc-2026-10-29",
      "/en/t/INTC",
      "/zh/t/INTC",
    ]);
    const by = (u: string) => entries.find((e) => e.url === `https://prebell.site${u}`)!;
    // lastmod is the update time, else the first publish; the home page follows the newest post.
    expect(by("/zh/p/premarket-2026-11-05").lastModified).toBe("2026-11-05T12:37:00Z");
    expect(by("/en/p/recap-2026-11-04").lastModified).toBe("2026-11-04T21:00:00Z");
    expect(by("/en").lastModified).toBe("2026-11-05T12:37:00Z");
    expect(by("/en/t/INTC").lastModified).toBe("2026-10-29T21:10:00Z");
    expect(by("/en/about").lastModified).toBeUndefined();
    expect(by("/en/p/recap-2026-11-04").alternates).toBeUndefined();
    expect(by("/zh/p/premarket-2026-11-05").alternates).toEqual({
      languages: {
        en: "https://prebell.site/en/p/premarket-2026-11-05",
        "zh-Hans": "https://prebell.site/zh/p/premarket-2026-11-05",
        "x-default": "https://prebell.site/en/p/premarket-2026-11-05",
      },
    });
  });

  it("renders as a sitemap with xhtml:link hreflang alternates, the same XML Next writes", async () => {
    const entries = await sitemapEntries(memoryD1(seed), origin);
    const xml = sitemapXml(entries);
    expect(xml).toBe(resolveSitemap(entries));
    expect(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>\n<urlset')).toBe(true);
    expect(xml).toContain('xmlns:xhtml="http://www.w3.org/1999/xhtml"');
    expect(xml).toContain("<loc>https://prebell.site/zh/p/premarket-2026-11-05</loc>");
    expect(xml).toContain(
      '<xhtml:link rel="alternate" hreflang="zh-Hans" href="https://prebell.site/zh/p/premarket-2026-11-05" />',
    );
    expect(xml).toContain("<lastmod>2026-11-05T12:37:00Z</lastmod>");
    expect(xml).not.toContain("/t/A<");
  });

  it("escapes XML in URLs", () => {
    expect(sitemapXml([{ url: "https://x.test/?a=1&b=<2>" }])).toContain("<loc>https://x.test/?a=1&amp;b=&lt;2&gt;</loc>");
  });

  it("costs two D1 queries: the posts and the tracked IPOs", async () => {
    const db = memoryD1(seed);
    const prepare = vi.spyOn(db, "prepare");
    await sitemapEntries(db, origin);
    expect(prepare).toHaveBeenCalledTimes(2);
  });

  it("robots.txt allows every crawler and points at the sitemap", async () => {
    const { default: robots } = await import("../src/app/robots");
    const txt = resolveRobots(await robots());
    expect(txt).toBe("User-Agent: *\nAllow: /\n\nSitemap: https://prebell.site/sitemap.xml\n");
  });
});

describe("sitemap edge cache", () => {
  /** An in-memory `caches.default`, keyed by URL. */
  const memoryCache = () => {
    const store = new Map<string, Response>();
    const cache: EdgeCache = {
      match: async (r) => store.get(r.url)?.clone(),
      put: async (r, res) => {
        store.set(r.url, res);
      },
    };
    return { store, cache };
  };

  it("builds once an hour per colo: a miss builds and stores, a hit skips D1 and the build", async () => {
    const { store, cache } = memoryCache();
    const pending: Promise<unknown>[] = [];
    const build = vi.fn(async () => "<urlset/>");
    const first = await cachedSitemap("https://prebell.site/sitemap.xml", cache, (p) => pending.push(p), build);
    await Promise.all(pending);
    expect(await first.text()).toBe("<urlset/>");
    expect(first.headers.get("Content-Type")).toBe("application/xml; charset=utf-8");
    expect(first.headers.get("Cache-Control")).toBe("public, max-age=3600");
    expect([...store.keys()]).toEqual(["https://prebell.site/sitemap.xml"]);
    // A query string does not bypass the cache.
    const second = await cachedSitemap("https://prebell.site/sitemap.xml?x=1", cache, (p) => pending.push(p), build);
    expect(await second.text()).toBe("<urlset/>");
    expect(build).toHaveBeenCalledTimes(1);
  });

  it("still answers without a cache (next dev)", async () => {
    const res = await cachedSitemap("http://localhost:3000/sitemap.xml", undefined, () => {}, async () => "<urlset/>");
    expect(await res.text()).toBe("<urlset/>");
  });
});
