import { describe, expect, it } from "vitest";
import {
  getAdjacentPosts,
  getLatestPost,
  getLatestPremarket,
  getPostBySlug,
  isPublishedIn,
  listPosts,
  PAGE_SIZE,
  parseContentType,
  resolvePost,
  type D1Like,
} from "../src/lib/posts";
import { memoryD1 } from "./d1";

/** One post per trading date; `en`/`zh` give each translation's status, or null for no row. */
function post(
  id: number,
  date: string,
  en: string | null,
  zh: string | null,
  type = "premarket",
  ts = `${date}T12:34:00Z`,
): string {
  const key = `${type}:${date}`;
  const slug = `${type}-${date}`;
  let sql = `INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${key}','${type}','${type}:v1','${date}','${slug}',NULL,'${ts}');\n`;
  if (en) sql += `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'en','${en}','EN ${date}','EN summary ${date}','# EN ${date}','${ts}');\n`;
  if (zh)
    sql +=
      zh === "published"
        ? `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'zh','published','中文 ${date}','摘要 ${date}','# 中文 ${date}','${ts}');\n`
        : `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'zh','${zh}',NULL,NULL,NULL,'${ts}');\n`;
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

describe("facts key", () => {
  it("exposes posts.facts_r2_key on the view, null when unset", async () => {
    const keyed = post(1, "2026-10-29", "published", "pending").replace(
      "'premarket-2026-10-29',NULL",
      "'premarket-2026-10-29','premarket/2026-10-29.json'",
    );
    const db = memoryD1(keyed + post(2, "2026-10-28", "published", null, "recap"));
    expect((await getLatestPost(db, "zh", "premarket"))!.factsKey).toBe("premarket/2026-10-29.json");
    expect((await getPostBySlug(db, "premarket-2026-10-29", "en"))!.factsKey).toBe("premarket/2026-10-29.json");
    expect((await getLatestPost(db, "en", "recap"))!.factsKey).toBeNull();
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

  it("getLatestPost reads only the heading: one statement, no body or sources", async () => {
    const db = memoryD1(mixed);
    const sql: string[] = [];
    const spy = { prepare: (q: string) => (sql.push(q), db.prepare(q)) };
    const p = await getLatestPost(spy, "en", "premarket");
    expect(sql).toHaveLength(1);
    expect(sql[0]).not.toContain("body_md");
    expect(p).not.toHaveProperty("bodyMd");
    expect(p).not.toHaveProperty("sources");
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

describe("getAdjacentPosts", () => {
  // Briefs in archive order: pm 27, recap 27, pm 28 (ZH failed), earnings 28, recap 28, pm 29 (ZH pending).
  const seq =
    post(1, "2026-10-27", "published", "published", "premarket", "2026-10-27T12:34:00Z") +
    post(2, "2026-10-27", "published", "published", "recap", "2026-10-27T20:45:00Z") +
    post(3, "2026-10-28", "published", "failed", "premarket", "2026-10-28T12:34:00Z") +
    post(4, "2026-10-28", "published", "published", "earnings", "2026-10-28T16:00:00Z") +
    post(5, "2026-10-28", "published", "published", "recap", "2026-10-28T20:45:00Z") +
    post(6, "2026-10-29", "published", "pending", "premarket", "2026-10-29T12:34:00Z");
  const at = (tradingDate: string, publishedAt: string | null, slug: string) => ({ tradingDate, publishedAt, slug });

  it("returns the previous and next published brief in archive order, skipping earnings checks", async () => {
    const adj = await getAdjacentPosts(memoryD1(seq), "en", at("2026-10-28", "2026-10-28T12:34:00Z", "premarket-2026-10-28"));
    expect(adj.prev).toMatchObject({ slug: "recap-2026-10-27", contentType: "recap", title: "EN 2026-10-27" });
    expect(adj.next).toMatchObject({ slug: "recap-2026-10-28", tradingDate: "2026-10-28" });
  });

  it("skips translations that are not published in the language", async () => {
    const recap28 = at("2026-10-28", "2026-10-28T20:45:00Z", "recap-2026-10-28");
    const adj = await getAdjacentPosts(memoryD1(seq), "zh", recap28);
    expect(adj.prev?.slug).toBe("recap-2026-10-27");
    expect(adj.next).toBeNull();
    expect((await getAdjacentPosts(memoryD1(seq), "en", recap28)).next?.slug).toBe("premarket-2026-10-29");
  });

  it("keeps a republished old brief in trading-date order", async () => {
    // premarket 10-27 re-run on 10-30: its published_at is now the newest of all.
    const rerun = seq.replace("'premarket-2026-10-27',NULL,'2026-10-27T12:34:00Z'", "'premarket-2026-10-27',NULL,'2026-10-30T09:00:00Z'");
    expect(rerun).not.toBe(seq);
    const db = memoryD1(rerun);
    const pm27 = await getAdjacentPosts(db, "en", at("2026-10-27", "2026-10-30T09:00:00Z", "premarket-2026-10-27"));
    // Archive order within 10-27 follows publish time, so the re-run now comes after the recap,
    // but it stays on its own date instead of jumping past 10-29.
    expect(pm27.prev?.slug).toBe("recap-2026-10-27");
    expect(pm27.next?.slug).toBe("premarket-2026-10-28");
    const pm29 = await getAdjacentPosts(db, "en", at("2026-10-29", "2026-10-29T12:34:00Z", "premarket-2026-10-29"));
    expect(pm29.next).toBeNull();
    expect(pm29.prev?.slug).toBe("recap-2026-10-28");
  });

  it("orders briefs with the same date and publish time by slug, without skipping either", async () => {
    const tie =
      post(1, "2026-10-27", "published", "published", "premarket", "2026-10-27T20:00:00Z") +
      post(2, "2026-10-27", "published", "published", "recap", "2026-10-27T20:00:00Z");
    const db = memoryD1(tie);
    expect((await getAdjacentPosts(db, "en", at("2026-10-27", "2026-10-27T20:00:00Z", "premarket-2026-10-27"))).next?.slug).toBe(
      "recap-2026-10-27",
    );
    expect((await getAdjacentPosts(db, "en", at("2026-10-27", "2026-10-27T20:00:00Z", "recap-2026-10-27"))).prev?.slug).toBe(
      "premarket-2026-10-27",
    );
  });

  it("is null at the ends and without a publish time, in one statement", async () => {
    const db = memoryD1(seq);
    const sql: string[] = [];
    const spy = { prepare: (q: string) => (sql.push(q), db.prepare(q)) };
    const first = await getAdjacentPosts(spy, "en", at("2026-10-27", "2026-10-27T12:34:00Z", "premarket-2026-10-27"));
    expect(first.prev).toBeNull();
    expect(first.next?.slug).toBe("recap-2026-10-27");
    expect(sql).toHaveLength(1);
    expect(sql[0]).toContain("CROSS JOIN");
    expect(await getAdjacentPosts(spy, "en", at("2026-10-28", null, "premarket-2026-10-28"))).toEqual({
      prev: null,
      next: null,
    });
    expect(sql).toHaveLength(1);
  });

  it("reads through the trading_date index", async () => {
    const db = memoryD1(seq);
    const seen: { sql: string; args: unknown[] }[] = [];
    const spy: D1Like = {
      prepare(sql) {
        const st = db.prepare(sql);
        const bind = st.bind.bind(st);
        st.bind = (...args) => {
          seen.push({ sql, args });
          return bind(...args);
        };
        return st;
      },
    };
    await getAdjacentPosts(spy, "en", at("2026-10-28", "2026-10-28T12:34:00Z", "premarket-2026-10-28"));
    expect(seen).toHaveLength(1);
    const { results } = await db
      .prepare(`EXPLAIN QUERY PLAN ${seen[0].sql}`)
      .bind(...seen[0].args)
      .all<{ detail: string }>();
    const plan = results.map((r) => r.detail).join("\n");
    // Both sides start from posts by date; neither scans the translations by language.
    expect(plan.match(/SEARCH p USING INDEX idx_posts_trading_date/g)).toHaveLength(2);
    expect(plan).not.toContain("idx_post_translations_lang_status");
  });
});

describe("resolvePost", () => {
  const db = () => memoryD1(seed);

  it("returns a visible post, published or pending", async () => {
    expect(await resolvePost(db(), "premarket-2026-10-27", "zh")).toMatchObject({ post: { status: "published" } });
    expect(await resolvePost(db(), "premarket-2026-10-29", "zh")).toMatchObject({ post: { status: "pending" } });
  });

  it("sends a failed translation to the home page of the language asked for", async () => {
    expect(await resolvePost(db(), "premarket-2026-10-28", "zh")).toEqual({ redirectTo: "/zh" });
    const enFailed = memoryD1(post(9, "2026-10-25", "failed", "published"));
    expect(await resolvePost(enFailed, "premarket-2026-10-25", "en")).toEqual({ redirectTo: "/en" });
  });

  it("is null for an unknown slug or a post published in neither language", async () => {
    expect(await resolvePost(db(), "nope", "en")).toBeNull();
    expect(await resolvePost(db(), "premarket-2026-10-26", "zh")).toBeNull();
    expect(await resolvePost(db(), "premarket-2026-10-26", "en")).toBeNull();
  });
});

describe("isPublishedIn", () => {
  it("is true only for a published translation", async () => {
    const db = memoryD1(seed);
    expect(await isPublishedIn(db, "premarket-2026-10-28", "en")).toBe(true);
    expect(await isPublishedIn(db, "premarket-2026-10-28", "zh")).toBe(false);
    expect(await isPublishedIn(db, "premarket-2026-10-29", "zh")).toBe(false);
    expect(await isPublishedIn(db, "premarket-2026-10-26", "en")).toBe(false);
    expect(await isPublishedIn(db, "nope", "en")).toBe(false);
  });
});

describe("cover fields and ticker", () => {
  const ts = "2026-10-30T21:00:00Z";
  const coverSeed =
    post(10, "2026-10-29", "published", "published") +
    `INSERT INTO runs VALUES ('earnings:AMGN:acc1','earnings','2026-10-30','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key)
VALUES (11,'earnings:AMGN:acc1','earnings','earnings:v1','2026-10-30','amgn-q3-fy2026-earnings','x','${ts}','covers/amgn-q3-fy2026-earnings.png');
INSERT INTO earnings_filings VALUES ('acc1',1,'AMGN','8-K','2.02','2026-10-30','2026-10-30','${ts}','x.htm','earnings:AMGN:acc1','done','${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, cover_line1, cover_line2, updated_at)
VALUES (11,'en','published','EN AMGN','sum','# EN','Amgen beats','',  '${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, cover_line1, cover_line2, updated_at)
VALUES (11,'zh','published','中文 AMGN','摘要','# 中文','安进超预期','营收创新高','${ts}');`;

  it("lists cover key, lines and ticker on heads, lists and neighbours", async () => {
    const db = memoryD1(coverSeed);
    const earn = { ticker: "AMGN", coverKey: "covers/amgn-q3-fy2026-earnings.png" };
    const none = { ticker: null, coverKey: null, coverLines: null };

    const view = await getPostBySlug(db, "amgn-q3-fy2026-earnings", "zh");
    expect(view).toMatchObject({ ...earn, coverLines: ["安进超预期", "营收创新高"] });
    const en = await getPostBySlug(db, "amgn-q3-fy2026-earnings", "en");
    expect(en!.coverLines).toEqual(["Amgen beats", ""]);

    const head = await getLatestPost(db, "en", "earnings");
    expect(head).toMatchObject({ ...earn, coverLines: ["Amgen beats", ""] });
    expect(await getLatestPost(db, "en", "premarket")).toMatchObject(none);

    const list = await listPosts(db, "zh", 1, "earnings");
    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({ ...earn, coverLines: ["安进超预期", "营收创新高"] });
    const briefs = await listPosts(db, "en", 1);
    expect(briefs).toHaveLength(1);
    expect(briefs[0]).toMatchObject(none);

    const adj = await getAdjacentPosts(db, "en", {
      tradingDate: "2026-10-30",
      publishedAt: ts,
      slug: "zzz",
    });
    expect(adj.prev).toMatchObject(none);
  });
});
