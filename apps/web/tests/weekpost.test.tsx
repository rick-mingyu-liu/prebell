import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import BriefView from "../src/components/BriefView";
import PostBody from "../src/components/PostBody";
import PostList from "../src/components/PostList";
import { loadBrief } from "../src/lib/brief";
import { listRecentBriefs } from "../src/lib/earnings";
import { typeLabel } from "../src/lib/labels";
import { CONTENT_TYPES, getPostBySlug, listPosts, type Lang } from "../src/lib/posts";
import { sitemapEntries } from "../src/lib/sitemap";
import { translator } from "../src/lib/translator";
import { memoryD1 } from "./d1";

/**
 * The Friday earnings week post (content type `week`, docs/superpowers/specs/2026-11-24-weekly-earnings-design.md):
 * a post written by the generator in both languages. The bodies here are the generator's own output
 * (crates/core/src/week, `fixtures/week-*.md`, kept equal by a Rust test), so the site renders what it will get.
 */
const fixture = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");
const BODY = { en: fixture("week-2026-12-04-en.md"), zh: fixture("week-2026-12-04-zh.md") };
const PEAK = fixture("week-2027-01-08-en.md");
const SLUG = "earnings-week-2026-12-04";
const TITLE = {
  en: "Earnings this week: Salesforce and Marvell Technology; 8 expected next week",
  zh: "本周财报：Salesforce和Marvell Technology；下周预计8家",
};
const SUMMARY = {
  en: "PreBell checked 2 earnings reports published Nov 30–Dec 4. 8 companies are expected to report Dec 7–Dec 11, as of Friday 18:05 ET; 2 dates are confirmed.",
  zh: "盘前铃本周核对了2份财报（11月30日至12月4日发布）。截至美东时间周五18:05，下周（12月7日至12月11日）预计8家公司公布财报，其中2家日期已确认。",
};
const sql = (s: string) => `'${s.replace(/'/g, "''")}'`;

/** The week post as the generator writes it, with a premarket brief and a recap of the same week around it. */
function seed(revisedAt: string | null = null): string {
  const ts = "2026-12-04T23:05:00Z";
  const rows = [
    `INSERT INTO runs VALUES ('week:2026-12-04','week','2026-12-04','published',1,NULL,'${ts}','${ts}');`,
    `INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, revised_at)
       VALUES (950,'week:2026-12-04','week','week-v1','2026-12-04','${SLUG}','week/2026-12-04.json','${ts}',${revisedAt ? sql(revisedAt) : "NULL"});`,
    `INSERT INTO sources (post_id, url, title, quote) VALUES (950,'https://www.bls.gov/schedule/news_release/cpi.htm','November CPI','');`,
  ];
  for (const lang of ["en", "zh"] as const) {
    const [c1, c2] = lang === "en" ? ["2 checks this week", "8 expected next week"] : ["本周 2 篇速评", "下周预计 8 家"];
    rows.push(
      `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at, cover_line1, cover_line2)
         VALUES (950,'${lang}','published',${sql(TITLE[lang])},${sql(SUMMARY[lang])},${sql(BODY[lang])},'${ts}',${sql(c1)},${sql(c2)});`,
    );
  }
  for (const [id, type, date] of [
    [951, "premarket", "2026-12-04"],
    [952, "recap", "2026-12-03"],
  ] as const) {
    const key = `${type}:${date}`;
    rows.push(`INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${date}T13:00:00Z','${date}T13:00:00Z');`);
    rows.push(
      `INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, published_at)
         VALUES (${id},'${key}','${type}','${type}:v1','${date}','${type}-${date}','${date}T13:00:00Z');`,
    );
    for (const lang of ["en", "zh"])
      rows.push(`INSERT INTO post_translations (post_id, lang, status, title, summary) VALUES (${id},'${lang}','published','T${id}','S${id}');`);
  }
  return rows.join("\n");
}

const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, "")
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&amp;/g, "&");

async function page(lang: Lang, revisedAt: string | null = null) {
  const db = memoryD1(seed(revisedAt));
  const post = (await getPostBySlug(db, SLUG, lang))!;
  const data = await loadBrief(db, undefined, post, lang);
  return renderToStaticMarkup(<BriefView data={data} locale={lang} signup={false} />);
}

describe("the earnings week post", () => {
  it("is a content type labelled Earnings week / 财报周报, with an archive filter", () => {
    expect(CONTENT_TYPES).toContain("week");
    expect(typeLabel(translator("en"), "week")).toBe("Earnings week");
    expect(typeLabel(translator("zh"), "week")).toBe("财报周报");
  });

  it.each(["en", "zh"] as const)("renders in %s: kicker, headline, lede, its own checked line, sections and links", async (lang) => {
    const out = await page(lang);
    expect(out).toContain(`<p class="kicker story-kicker">${lang === "en" ? "Earnings week" : "财报周报"}</p>`);
    expect(text(out)).toContain(TITLE[lang]);
    expect(text(out)).toContain(SUMMARY[lang]);
    // Not the earnings checks' claim: the dates are the calendar's, as of the build.
    expect(out).toContain(
      `<span class="checked">${
        lang === "en"
          ? "✓ Figures from PreBell&#x27;s earnings checks. Dates as of the time shown below: confirmed ones announced by the company, the rest from an aggregator."
          : "✓ 数字出自盘前铃财报速评。日期以下文所示时间为准：已确认的由公司公布，其余来自第三方汇总。"
      }</span>`,
    );
    expect(out).not.toContain("Every figure traced to the filing");
    expect(out).not.toContain("每个数字均出自申报文件");
    // The body starts after the headline and lede: its own "# title" is not repeated.
    expect(out.match(/<h1/g)?.length).toBe(1);
    expect(out).toContain(lang === "en" ? "<h2>Earnings checks published Nov 30–Dec 4</h2>" : "<h2>11月30日至12月4日发布的财报速评</h2>");
    expect(out).toContain(`<a href="/${lang}/p/crm-q3-fy2027-earnings">Salesforce (CRM)</a>`);
    expect(out).toContain(`<a href="/${lang}/t/ADBE">Adobe (ADBE)</a>`);
    expect(out).toContain(`<a href="/${lang}/earnings?week=2026-12-07">`);
    // The event's official page is a source, so it is a real link.
    expect(out).toContain('href="https://www.bls.gov/schedule/news_release/cpi.htm"');
    // No raw Markdown left over.
    expect(text(out)).not.toMatch(/\]\(|\*\*/);
  });

  it("shows the date of a correction days later, not only its time", async () => {
    expect(text(await page("en", "2026-12-08T15:05:00Z"))).toContain("Updated Dec 8, 2026, 10:05 AM ET");
    expect(text(await page("zh", "2026-12-08T15:05:00Z"))).toMatch(/更新于 2026年12月8日.*10:05（美东）/);
  });

  it("is in the home page's recent list and in the archive's default view, and alone under its filter", async () => {
    const db = memoryD1(seed());
    expect((await listRecentBriefs(db, "en", 5)).map((p) => p.slug)).toEqual([SLUG, "premarket-2026-12-04", "recap-2026-12-03"]);
    const archive = await listPosts(db, "zh", 1);
    expect(archive.map((p) => p.slug)).toEqual([SLUG, "premarket-2026-12-04", "recap-2026-12-03"]);
    expect((await listPosts(db, "en", 1, "week")).map((p) => p.slug)).toEqual([SLUG]);
    expect((await listPosts(db, "en", 1, "premarket")).map((p) => p.slug)).toEqual(["premarket-2026-12-04"]);
    // A list row says what it is, with the flat tile (no cover yet).
    const row = renderToStaticMarkup(<PostList locale="zh" items={archive.slice(0, 1)} />);
    expect(text(row)).toContain("财报周报");
    expect(text(row)).toContain(TITLE.zh);
  });

  it("is in the sitemap in both languages", async () => {
    const urls = (await sitemapEntries(memoryD1(seed()), new URL("https://prebell.site"))).map((e) => e.url);
    expect(urls).toContain(`https://prebell.site/en/p/${SLUG}`);
    expect(urls).toContain(`https://prebell.site/zh/p/${SLUG}`);
  });

  it("renders a peak week of 60 checks and 200 companies whole, through PostBody", () => {
    const out = renderToStaticMarkup(<PostBody markdown={PEAK} sources={["https://www.bls.gov/schedule/news_release/cpi.htm"]} />);
    expect(out.match(/<a href="\/en\/p\//g)?.length).toBe(60);
    expect(out.match(/<a href="\/en\/t\//g)?.length).toBe(200);
    expect(out.match(/<h3>/g)?.length).toBe(5);
    expect(text(out)).not.toMatch(/\]\(/);
  });
});
