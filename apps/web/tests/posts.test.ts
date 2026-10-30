import { describe, expect, it } from "vitest";
import {
  getLatestPost,
  getLatestPremarket,
  getPostBySlug,
  listPosts,
  PAGE_SIZE,
  parseContentType,
} from "../src/lib/posts";
import { memoryD1 } from "./d1";

/** One post per trading date; `en`/`zh` give each translation's status, or null for no row. */
function post(id: number, date: string, en: string | null, zh: string | null, type = "premarket"): string {
  const key = `${type}:${date}`;
  const slug = `${type}-${date}`;
  const ts = `${date}T12:34:00Z`;
  let sql = `INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${key}','${type}','${type}:v1','${date}','${slug}',NULL,'${ts}');\n`;
  if (en) sql += `INSERT INTO post_translations VALUES (${id},'en','${en}','EN ${date}','EN summary ${date}','# EN ${date}','${ts}');\n`;
  if (zh)
    sql +=
      zh === "published"
        ? `INSERT INTO post_translations VALUES (${id},'zh','published','中文 ${date}','摘要 ${date}','# 中文 ${date}','${ts}');\n`
        : `INSERT INTO post_translations VALUES (${id},'zh','${zh}',NULL,NULL,NULL,'${ts}');\n`;
  return sql;
}

const seed =
  post(1, "2026-10-27", "published", "published") +
  post(2, "2026-10-28", "published", "failed") +
  post(3, "2026-10-29", "published", "pending") +
  post(4, "2026-10-26", "failed", "failed") +
  `INSERT INTO sources VALUES (1,'https://news.test/aapl','Apple jumps','q');
INSERT INTO sources VALUES (1,'https://news.test/fed',NULL,'q');`;

describe("getPostBySlug", () => {
  it("returns a pending ZH row as status pending", async () => {
    const db = memoryD1(seed);
    const p = await getPostBySlug(db, "premarket-2026-10-29", "zh");
    expect(p).not.toBeNull();
    expect(p!.status).toBe("pending");
    expect(p!.slug).toBe("premarket-2026-10-29");
    expect(p!.hasOtherLang).toBe(true);
  });

  it("returns the published EN row with sources, and knows ZH is not published", async () => {
    const db = memoryD1(seed);
    const p = await getPostBySlug(db, "premarket-2026-10-29", "en");
    expect(p).toMatchObject({
      slug: "premarket-2026-10-29",
      contentType: "premarket",
      tradingDate: "2026-10-29",
      publishedAt: "2026-10-29T12:34:00Z",
      title: "EN 2026-10-29",
      summary: "EN summary 2026-10-29",
      bodyMd: "# EN 2026-10-29",
      status: "published",
      hasOtherLang: false,
    });
  });

  it("returns sources, falling back to the URL for a missing title", async () => {
    const db = memoryD1(seed);
    const p = await getPostBySlug(db, "premarket-2026-10-27", "zh");
    expect(p!.status).toBe("published");
    expect(p!.hasOtherLang).toBe(true);
    expect(p!.sources).toEqual([
      { url: "https://news.test/aapl", title: "Apple jumps" },
      { url: "https://news.test/fed", title: "https://news.test/fed" },
    ]);
  });

  it("hides failed translations and unknown slugs", async () => {
    const db = memoryD1(seed);
    expect(await getPostBySlug(db, "premarket-2026-10-28", "zh")).toBeNull();
    expect(await getPostBySlug(db, "premarket-2026-10-26", "en")).toBeNull();
    expect(await getPostBySlug(db, "nope", "en")).toBeNull();
  });
});

describe("listPosts", () => {
  it("zh excludes failed and pending translations", async () => {
    const db = memoryD1(seed);
    const items = await listPosts(db, "zh", 1);
    expect(items.map((i) => i.slug)).toEqual(["premarket-2026-10-27"]);
    expect(items[0].title).toBe("中文 2026-10-27");
  });

  it("en excludes failed, newest first", async () => {
    const db = memoryD1(seed);
    const items = await listPosts(db, "en", 1);
    expect(items.map((i) => i.slug)).toEqual([
      "premarket-2026-10-29",
      "premarket-2026-10-28",
      "premarket-2026-10-27",
    ]);
  });

  it("pages by PAGE_SIZE", async () => {
    let many = "";
    for (let i = 0; i < PAGE_SIZE + 5; i++) {
      const d = new Date(Date.UTC(2026, 0, 1 + i)).toISOString().slice(0, 10);
      many += post(100 + i, d, "published", "published");
    }
    const db = memoryD1(many);
    const p1 = await listPosts(db, "en", 1);
    const p2 = await listPosts(db, "en", 2);
    expect(PAGE_SIZE).toBe(20);
    expect(p1).toHaveLength(20);
    expect(p2).toHaveLength(5);
    expect(p1[0].tradingDate > p2[0].tradingDate).toBe(true);
    expect(await listPosts(db, "en", 0)).toEqual(p1);
  });
});

describe("getLatestPremarket", () => {
  it("en returns the newest trading_date", async () => {
    const db = memoryD1(seed);
    const p = await getLatestPremarket(db, "en");
    expect(p!.tradingDate).toBe("2026-10-29");
    expect(p!.status).toBe("published");
  });

  it("zh returns the newest brief even while its ZH text is pending", async () => {
    const db = memoryD1(seed);
    const p = await getLatestPremarket(db, "zh");
    expect(p!.tradingDate).toBe("2026-10-29");
    expect(p!.status).toBe("pending");
  });

  it("returns null when there is nothing", async () => {
    expect(await getLatestPremarket(memoryD1(""), "en")).toBeNull();
  });
});

describe("content types", () => {
  const mixed =
    post(1, "2026-10-28", "published", "published") +
    post(2, "2026-10-28", "published", "published", "recap") +
    post(3, "2026-10-29", "published", "pending") +
    post(4, "2026-10-27", "published", "pending", "recap");

  it("getLatestPost returns the newest of the type and never another type", async () => {
    const db = memoryD1(mixed);
    const r = await getLatestPost(db, "en", "recap");
    expect(r).toMatchObject({ slug: "recap-2026-10-28", contentType: "recap", status: "published" });
    const p = await getLatestPost(db, "en", "premarket");
    expect(p).toMatchObject({ slug: "premarket-2026-10-29", contentType: "premarket" });
    expect(await getLatestPost(memoryD1(post(1, "2026-10-28", "published", null)), "en", "recap")).toBeNull();
  });

  it("listPosts filters by type and lists both without one", async () => {
    const db = memoryD1(mixed);
    expect((await listPosts(db, "en", 1, "recap")).map((i) => i.slug)).toEqual([
      "recap-2026-10-28",
      "recap-2026-10-27",
    ]);
    expect((await listPosts(db, "zh", 1, "premarket")).map((i) => i.slug)).toEqual(["premarket-2026-10-28"]);
    const all = await listPosts(db, "en", 1);
    expect(all.map((i) => i.contentType).sort()).toEqual(["premarket", "premarket", "recap", "recap"]);
  });

  it("getPostBySlug carries the content type", async () => {
    const db = memoryD1(mixed);
    expect((await getPostBySlug(db, "recap-2026-10-27", "zh"))!).toMatchObject({
      contentType: "recap",
      status: "pending",
    });
  });

  it("parseContentType accepts only known types", () => {
    expect(parseContentType("recap")).toBe("recap");
    expect(parseContentType("premarket")).toBe("premarket");
    expect(parseContentType(["recap", "x"])).toBe("recap");
    expect(parseContentType("earnings")).toBe("earnings");
    expect(parseContentType("x")).toBeNull();
    expect(parseContentType(undefined)).toBeNull();
  });

  it("listPosts with type earnings returns only earnings posts", async () => {
    const db = memoryD1(mixed + post(5, "2026-10-29", "published", "published", "earnings"));
    expect((await listPosts(db, "en", 1, "earnings")).map((i) => i.slug)).toEqual(["earnings-2026-10-29"]);
    expect((await listPosts(db, "zh", 1, "earnings")).map((i) => i.contentType)).toEqual(["earnings"]);
  });

  it("listPosts without a type lists briefs only, never earnings checks", async () => {
    const db = memoryD1(mixed + post(5, "2026-10-29", "published", "published", "earnings"));
    const all = await listPosts(db, "en", 1);
    expect(all.map((i) => i.contentType).sort()).toEqual(["premarket", "premarket", "recap", "recap"]);
  });
});
