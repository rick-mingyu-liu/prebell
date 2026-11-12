import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import BriefView from "../src/components/BriefView";
import HomeView from "../src/components/HomeView";
import IpoView from "../src/components/IpoView";
import type { Locale } from "../src/i18n/routing";
import { loadBrief } from "../src/lib/brief";
import type { R2Like } from "../src/lib/facts";
import { loadHome, type HomeData } from "../src/lib/home";
import { listHomeIpos, listIpoWriteups, loadIpo, writeupIpoSlug } from "../src/lib/ipo";
import { CONTENT_TYPES, getPostBySlug, listPosts, type D1Like } from "../src/lib/posts";
import { sitemapEntries } from "../src/lib/sitemap";
import { memoryD1 } from "./d1";

/**
 * The home page's IPO section, IPO write-ups as a content type, the write-up page's link to its IPO, and the IPO
 * page's list of write-ups. Data: every migration plus db/seed-ipo-preview.sql, adjusted per state.
 */
const SEED = readFileSync(new URL("../../../db/seed-ipo-preview.sql", import.meta.url), "utf8");
const TODAY = "2026-11-11";

/** Only Anthropic, as in production on 2026-11-11: range set, no date, no news, no write-up. */
const PRODUCTION = `${SEED}
DELETE FROM ipo_trading WHERE slug <> 'anthropic';
DELETE FROM ipo_items;
DELETE FROM ipo_facts WHERE slug <> 'anthropic';
DELETE FROM ipo_filings WHERE slug <> 'anthropic';
DELETE FROM ipo_watch WHERE slug <> 'anthropic';
UPDATE ipo_watch SET first_trade_date = NULL, date_status = NULL, date_source = NULL, date_source_name = NULL,
  date_source_url = NULL, date_quote = NULL, date_as_of = NULL WHERE slug = 'anthropic';`;

/** A published IPO write-up (EN and, unless `zh` is false, ZH) for Anthropic's range-set milestone. */
function writeup(id: number, milestone: string, key: string, date: string, title: string, zh = true): string {
  const run = `ipo:anthropic:${milestone}:${key}`;
  const ts = `${date}T16:00:00Z`;
  return `INSERT INTO runs VALUES ('${run}','ipo','${date}','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
  VALUES (${id},'${run}','ipo','ipo-${milestone}','${date}','anthropic-ipo-${milestone.replace("_", "-")}-${date}','ipo/anthropic/${milestone}-${key}.json','${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'en','published','${title}','Sum','# ${title}','${ts}');
${zh ? `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'zh','published','中文 ${title}','摘要','# 中文','${ts}');` : ""}
`;
}

const RANGE_SET = writeup(901, "range_set", "0001628280-26-072484", "2026-11-10", "Anthropic sets IPO range at $70 to $78 a share");
const FILED = writeup(900, "filed", "0001628280-26-067168", "2026-10-26", "Anthropic files to go public", false);

const homeData = (ipos: HomeData["ipos"]): HomeData => ({ lead: null, leadFacts: null, earnings: [], recent: [], reporting: null, ipos });
const home = (ipos: HomeData["ipos"], locale: Locale = "en") =>
  renderToStaticMarkup(<HomeView data={homeData(ipos)} locale={locale} signup={false} />);
const section = (out: string) => {
  const at = out.indexOf('<section class="front-ipo"');
  return at < 0 ? "" : out.slice(at, out.indexOf("</section>", at) + "</section>".length);
};
const textOf = (html: string) =>
  html
    .replace(/<[^>]+>/g, "")
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&amp;/g, "&");

describe("listHomeIpos", () => {
  it("production today: Anthropic with its range from the S-1/A, no date, no valuation, no write-up", async () => {
    const [a, ...rest] = await listHomeIpos(memoryD1(PRODUCTION), "en", TODAY);
    expect(rest).toEqual([]);
    expect(a).toMatchObject({
      slug: "anthropic",
      name: "Anthropic",
      symbol: "ANT",
      exchange: "Nasdaq Global Select Market",
      status: "range_set",
      date: null,
      price: { low: 70, high: 78 },
      priceFiling: { form: "S-1/A", filingDate: "2026-11-10" },
      valuation: null,
      latest: null,
      writeup: null,
    });
  });

  it("a reported date, the newest valuation with its outlet, and the newest write-up in the page's language", async () => {
    const db = memoryD1(`${SEED}\n${FILED}\n${RANGE_SET}`);
    const [a] = (await listHomeIpos(db, "en", TODAY)).filter((i) => i.slug === "anthropic");
    expect(a.date).toMatchObject({ date: "2026-11-16", status: "reported", sourceName: "Example News" });
    expect(a.valuation).toEqual({ usd: 1.4e12, outlet: "Example News", publishedOn: "2026-11-10", url: "https://news.example/2026/11/10/anthropic-ipo-pricing" });
    expect(a.writeup).toEqual({ slug: "anthropic-ipo-range-set-2026-11-10", title: "Anthropic sets IPO range at $70 to $78 a share", tradingDate: "2026-11-10" });
    // The older write-up has no Chinese text; the Chinese page's newest is still the range one.
    const [z] = (await listHomeIpos(db, "zh", TODAY)).filter((i) => i.slug === "anthropic");
    expect(z.writeup?.title).toBe("中文 Anthropic sets IPO range at $70 to $78 a share");
  });

  it("after listing: the latest close with the change from the IPO price, the final price from the 424B4", async () => {
    const [n] = (await listHomeIpos(memoryD1(SEED), "en", TODAY)).filter((i) => i.slug === "northwind-robotics");
    expect(n.price).toEqual({ final: 24 });
    expect(n.priceFiling?.form).toBe("424B4");
    expect(n.latest).toMatchObject({ sessionDate: "2026-11-10", close: 28.45, ipoPrice: 24, changePct: 18.54 });
    expect(n.date).toMatchObject({ status: "confirmed", source: "trading" });
  });

  it("leaves out withdrawn IPOs and those listed over 30 days ago, counting from the first stored session", async () => {
    const db = memoryD1(`${SEED}\nUPDATE ipo_watch SET status = 'withdrawn', first_trade_date = NULL, date_status = NULL, date_source = NULL,
      date_source_name = NULL, date_source_url = NULL, date_quote = NULL, date_as_of = NULL WHERE slug = 'harbor-bio';`);
    const slugs = async (today: string) => (await listHomeIpos(db, "en", today)).map((i) => i.slug).sort();
    // Northwind's first session is Nov 5: shown through Dec 5, gone from Dec 6.
    expect(await slugs("2026-12-05")).toEqual(["anthropic", "northwind-robotics"]);
    expect(await slugs("2026-12-06")).toEqual(["anthropic"]);
  });

  it("shows no price without the filing it came from, and no half range", async () => {
    const noFiling = memoryD1(`${PRODUCTION}\nDELETE FROM ipo_filings WHERE accession = '0001628280-26-072484';`);
    expect((await listHomeIpos(noFiling, "en", TODAY))[0]).toMatchObject({ price: null, priceFiling: null });
    const half = memoryD1(`${PRODUCTION}\nDELETE FROM ipo_facts WHERE kind = 'price_high';`);
    expect((await listHomeIpos(half, "en", TODAY))[0].price).toBeNull();
  });

  it("drops a valuation it cannot print or attribute", async () => {
    const tiny = memoryD1(`${SEED}\nUPDATE ipo_items SET value_usd = 500 WHERE kind = 'valuation';`);
    expect((await listHomeIpos(tiny, "en", TODAY)).find((i) => i.slug === "anthropic")!.valuation).toBeNull();
  });

  it("is empty with no IPO tracked, and before migration 0009 (no tables)", async () => {
    expect(await listHomeIpos(memoryD1(""), "en", TODAY)).toEqual([]);
    const missing: D1Like = {
      prepare() {
        throw new Error("no such table: ipo_watch");
      },
    };
    expect(await listHomeIpos(missing, "en", TODAY)).toEqual([]);
  });
});

describe("loadHome with the IPO section", () => {
  const r2: R2Like = { get: async () => null };

  it("adds one D1 read: six without a reporting day (five before), seven with one (six before)", async () => {
    const reportingDay = `INSERT INTO earnings_calendar VALUES ('KO','${TODAY}','bmo','confirmed','yahoo',NULL,NULL,NULL,'t','t',NULL,'t');`;
    for (const [seed, reads] of [
      ["", 6],
      [PRODUCTION, 6],
      [`${PRODUCTION}\n${reportingDay}`, 7],
    ] as const) {
      const db = memoryD1(seed);
      const prepare = vi.spyOn(db, "prepare");
      await loadHome(db, r2, "en", TODAY);
      expect(prepare).toHaveBeenCalledTimes(reads);
      // Exactly one of them is the IPO section's.
      expect(prepare.mock.calls.filter(([sql]) => sql.includes("FROM ipo_watch"))).toHaveLength(1);
    }
  });

  it("carries the active IPOs, and none when the tables are missing", async () => {
    expect((await loadHome(memoryD1(PRODUCTION), r2, "en", TODAY)).ipos.map((i) => i.slug)).toEqual(["anthropic"]);
    expect((await loadHome(memoryD1(""), r2, "en", TODAY)).ipos).toEqual([]);
  });
});

describe("the home section", () => {
  it("production today: name, symbol, exchange, status, range with its filing, date not set, the IPO page link", async () => {
    const ipos = await listHomeIpos(memoryD1(PRODUCTION), "en", TODAY);
    const s = section(home(ipos));
    expect(s.startsWith('<section class="front-ipo" aria-labelledby="front-ipo"><h2 class="section-title" id="front-ipo">IPO watch</h2>')).toBe(true);
    expect(textOf(s)).toBe(
      "IPO watch" +
        "Price range set" +
        "Anthropic" +
        "ANT · Nasdaq Global Select Market" +
        "Price range$70.00 to $78.00 per share S-1/A, filed Nov 10, 2026" +
        "First trading day: date not set." +
        "Filings, figures and sources →",
    );
    expect(s).toContain('<a href="/en/ipo/anthropic">Anthropic</a>');
    expect(s).toContain('<a class="more" href="/en/ipo/anthropic">');
    const zh = textOf(section(home(await listHomeIpos(memoryD1(PRODUCTION), "zh", TODAY), "zh")));
    expect(zh).toBe(
      "IPO 追踪" +
        "已定发行价区间" +
        "Anthropic" +
        "ANT · Nasdaq Global Select Market" +
        "发行价区间每股70.00至78.00美元 S-1/A，2026年11月10日提交" +
        "首个交易日：日期未定。" +
        "申报文件、数字及来源 →",
    );
  });

  it("a reported date worded as on the IPO page, the valuation with its outlet, the newest write-up", async () => {
    const db = memoryD1(`${SEED}\n${RANGE_SET}\nDELETE FROM ipo_watch WHERE slug <> 'anthropic';`);
    const en = section(home(await listHomeIpos(db, "en", TODAY)));
    const text = textOf(en);
    expect(text).toContain("Reported valuation$1.4 trillion, as reported by Example News on Nov 10, 2026 ↗");
    expect(text).toContain("Unconfirmed Expected to start trading Mon, Nov 16, 2026, as reported by Example News on Nov 10, 2026. Article ↗");
    expect(text).toContain("Latest write-up: Anthropic sets IPO range at $70 to $78 a share");
    expect(en).toContain('<a href="/en/p/anthropic-ipo-range-set-2026-11-10">');
    expect(en).toContain('<span class="ipo-flag">Unconfirmed</span>');
    // The home section shows the date line without the source's words (the IPO page has them).
    expect(en).not.toContain("ipo-src-quote");
    // Never a change between valuations, never a valuation without its outlet.
    expect(text).not.toMatch(/%/);
    const zh = textOf(section(home(await listHomeIpos(db, "zh", TODAY), "zh")));
    expect(zh).toContain("媒体报道的估值1.4万亿美元，据 Example News 2026年11月10日报道 ↗");
    expect(zh).toContain("未确认 预计2026年11月16日开始交易，据 Example News 2026年11月10日报道。 原文 ↗");
    expect(zh).toContain("最新文章： 中文 Anthropic sets IPO range at $70 to $78 a share");
  });

  it("after listing: the IPO price, the latest close and its change beside both inputs, Yahoo named", async () => {
    const db = memoryD1(`${SEED}\nDELETE FROM ipo_watch WHERE slug <> 'northwind-robotics';`);
    const text = textOf(section(home(await listHomeIpos(db, "en", TODAY))));
    expect(text).toContain("Listed");
    expect(text).toContain("IPO price$24.00 per share 424B4, filed Nov 5, 2026");
    expect(text).toContain("Last close$28.45, Nov 10, 2026 · +18.54% (close $28.45 vs. IPO price $24.00) Source: Yahoo Finance");
    expect(text).toContain("Confirmed First trading day Thu, Nov 5, 2026: its first regular session, per Yahoo Finance.");
    const zh = textOf(section(home(await listHomeIpos(db, "zh", TODAY), "zh")));
    expect(zh).toContain("北风机器人");
    expect(zh).toContain("最新收盘价28.45美元，2026年11月10日 · +18.54%（收盘价28.45美元，发行价24.00美元） 来源：Yahoo Finance");
  });

  it("no section and no empty box when no IPO is active", () => {
    for (const locale of ["en", "zh"] as const) {
      const out = home([], locale);
      expect(out).not.toContain("front-ipo");
      expect(out).not.toMatch(/IPO/);
    }
  });

  it("sits first in the side column, so on a phone it follows the lead and comes before the movers", async () => {
    const ipos = await listHomeIpos(memoryD1(PRODUCTION), "en", TODAY);
    const out = renderToStaticMarkup(
      <HomeView
        data={{
          ...homeData(ipos),
          lead: {
            slug: "premarket-2026-11-11",
            contentType: "premarket",
            tradingDate: "2026-11-11",
            publishedAt: "2026-11-11T12:40:00Z",
            title: "Lead",
            summary: "Sum",
            status: "published",
            hasOtherLang: true,
            factsKey: null,
            revisedAt: null,
            coverKey: null,
            coverLines: null,
            ticker: null,
          },
        }}
        locale="en"
        signup={false}
      />,
    );
    const at = (s: string) => out.indexOf(s);
    expect(at('<aside class="front-side"><section class="front-ipo"')).toBeGreaterThan(at('<section class="front-lead">'));
    expect(at('class="front-ipo"')).toBeLessThan(at('class="front-movers"'));
    const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");
    expect(css).toMatch(/\.front-ipo, \.front-movers, \.front-themes \{ order: 2; \}/);
  });

  it("renders hostile text as text", async () => {
    const db = memoryD1(`${PRODUCTION}
      UPDATE ipo_watch SET name = '<script>alert(1)</script>', exchange = '<b>x</b>' WHERE slug = 'anthropic';
      ${RANGE_SET.replace("Anthropic sets IPO range at $70 to $78 a share", "<img src=x onerror=alert(1)>")}`);
    const out = section(home(await listHomeIpos(db, "en", TODAY)));
    expect(out).not.toMatch(/<script|<img/);
    expect(out).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(out).toContain("&lt;b&gt;x&lt;/b&gt;");
    expect(out).toContain("&lt;img src=x onerror=alert(1)&gt;");
  });
});

describe("IPO write-ups as posts", () => {
  it("are a content type with the label IPO in both languages, in the archive filter", async () => {
    expect(CONTENT_TYPES).toContain("ipo");
    const en = JSON.parse(readFileSync(new URL("../messages/en.json", import.meta.url), "utf8"));
    const zh = JSON.parse(readFileSync(new URL("../messages/zh.json", import.meta.url), "utf8"));
    expect([en.types.ipo, zh.types.ipo]).toEqual(["IPO", "IPO"]);
    const db = memoryD1(`${SEED}\n${FILED}\n${RANGE_SET}`);
    expect((await listPosts(db, "en", 1, "ipo")).map((p) => p.slug)).toEqual([
      "anthropic-ipo-range-set-2026-11-10",
      "anthropic-ipo-filed-2026-10-26",
    ]);
    // The default archive (the daily briefs) is unchanged.
    expect(await listPosts(db, "en", 1)).toEqual([]);
  });

  it("are in the sitemap like any post: paired when both languages are published", async () => {
    const urls = (await sitemapEntries(memoryD1(`${SEED}\n${FILED}\n${RANGE_SET}`), new URL("https://prebell.site"))).map((e) =>
      e.url.replace("https://prebell.site", ""),
    );
    expect(urls).toContain("/en/p/anthropic-ipo-range-set-2026-11-10");
    expect(urls).toContain("/zh/p/anthropic-ipo-range-set-2026-11-10");
    expect(urls).toContain("/en/p/anthropic-ipo-filed-2026-10-26");
    expect(urls).not.toContain("/zh/p/anthropic-ipo-filed-2026-10-26");
  });

  it("the write-up page links to its IPO page, with the briefs' checked line, and reads no charts", async () => {
    const db = memoryD1(`${SEED}\n${RANGE_SET}`);
    const post = (await getPostBySlug(db, "anthropic-ipo-range-set-2026-11-10", "en"))!;
    const prepare = vi.spyOn(db, "prepare");
    const data = await loadBrief(db, undefined, post, "en");
    expect(prepare).not.toHaveBeenCalled();
    for (const locale of ["en", "zh"] as const) {
      const out = renderToStaticMarkup(<BriefView data={data} locale={locale} signup={false} />);
      expect(out).toContain(
        locale === "en"
          ? '<p class="story-ipo"><a class="more" href="/en/ipo/anthropic">IPO watch: every filing and figure with its source →</a></p>'
          : '<p class="story-ipo"><a class="more" href="/zh/ipo/anthropic">IPO 追踪：每份申报文件与每个数字的来源 →</a></p>',
      );
      expect(out).toContain(locale === "en" ? "✓ Numbers checked against source data" : "数字");
      expect(out).not.toContain("Every figure traced to the filing");
    }
  });

  it("names its IPO by the facts key, and nothing for any other key", () => {
    expect(writeupIpoSlug("ipo/anthropic/range_set-0001628280-26-072484.json")).toBe("anthropic");
    expect(writeupIpoSlug("ipo/anthropic/listed-2026-11-16.json")).toBe("anthropic");
    for (const bad of [null, "premarket/2026-11-11.json", "ipo/../x.json", "ipo/Anthropic/x-1.json", "ipo/anthropic/x.json"])
      expect(writeupIpoSlug(bad), String(bad)).toBeNull();
  });
});

describe("the IPO page lists its write-ups", () => {
  it("newest first, in the page's language, linked; none and no section without any", async () => {
    const db = memoryD1(`${SEED}\n${FILED}\n${RANGE_SET}`);
    expect((await listIpoWriteups(db, "anthropic", "en")).map((w) => w.slug)).toEqual([
      "anthropic-ipo-range-set-2026-11-10",
      "anthropic-ipo-filed-2026-10-26",
    ]);
    expect((await listIpoWriteups(db, "anthropic", "zh")).map((w) => w.slug)).toEqual(["anthropic-ipo-range-set-2026-11-10"]);
    // The range does not reach another IPO whose slug starts the same way.
    expect(await listIpoWriteups(db, "anthropi", "en")).toEqual([]);
    const ipo = (await loadIpo(db, "anthropic", "en", TODAY))!;
    const out = renderToStaticMarkup(<IpoView ipo={ipo} locale="en" />);
    expect(textOf(out)).toContain(
      "Write-ups" + "Nov 10, 2026 Anthropic sets IPO range at $70 to $78 a share" + "Oct 26, 2026 Anthropic files to go public",
    );
    expect(out).toContain('<a class="ipo-writeup-link" href="/en/p/anthropic-ipo-range-set-2026-11-10">');
    const none = (await loadIpo(memoryD1(SEED), "anthropic", "en", TODAY))!;
    expect(renderToStaticMarkup(<IpoView ipo={none} locale="en" />)).not.toContain('id="writeups"');
  });
});
