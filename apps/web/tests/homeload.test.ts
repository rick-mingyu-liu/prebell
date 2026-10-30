import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadHome } from "../src/lib/home";
import type { R2Like } from "../src/lib/facts";
import { memoryD1 } from "./d1";

const fixtureText = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");

/** A published EN+ZH post; `factsKey` null stores none. */
function post(id: number, type: string, date: string, publishedAt: string, factsKey: string | null): string {
  const key = `${type}:${date}`;
  return `INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${publishedAt}','${publishedAt}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${key}','${type}','${type}:v1','${date}','${type}-${date}',${factsKey ? `'${factsKey}'` : "NULL"},'${publishedAt}');
INSERT INTO post_translations VALUES (${id},'en','published','EN ${type} ${date}','Summary','# Body','${publishedAt}');
INSERT INTO post_translations VALUES (${id},'zh','published','中文 ${type} ${date}','摘要','# 正文','${publishedAt}');\n`;
}

const seed =
  post(1, "premarket", "2026-10-30", "2026-10-30T13:20:43Z", "premarket/2026-10-30.json") +
  post(2, "recap", "2026-10-29", "2026-10-29T20:45:00Z", "recap/2026-10-29.json") +
  post(3, "premarket", "2026-10-29", "2026-10-29T12:34:00Z", "premarket/2026-10-29.json");

/** An R2 bucket holding the fixtures that records every key it is asked for. */
function fakeR2() {
  const objects: Record<string, string> = {
    "premarket/2026-10-30.json": fixtureText("premarket-2026-10-30.json"),
    "recap/2026-10-29.json": fixtureText("recap-sample.json"),
  };
  const gets: string[] = [];
  const bucket: R2Like = {
    async get(key) {
      gets.push(key);
      const text = objects[key];
      return text === undefined ? null : { text: async () => text };
    },
  };
  return { bucket, gets };
}

const TODAY = "2026-10-30";

describe("loadHome", () => {
  it("defaults to the later-published brief and reads only its facts", async () => {
    const { bucket, gets } = fakeR2();
    const home = await loadHome(memoryD1(seed), bucket, "en", undefined, TODAY);
    expect(home.tab).toBe("premarket");
    expect(home.tabs).toEqual(["premarket", "recap"]);
    expect(home.post?.slug).toBe("premarket-2026-10-30");
    expect(gets).toEqual(["premarket/2026-10-30.json"]);
    expect(home.facts?.kind).toBe("premarket");
    expect(home.facts?.tradingDate).toBe("2026-10-30");
  });

  it("follows ?tab=recap: the recap post, its key and the recap kind", async () => {
    const { bucket, gets } = fakeR2();
    const home = await loadHome(memoryD1(seed), bucket, "zh", "recap", TODAY);
    expect(home.tab).toBe("recap");
    expect(home.post?.slug).toBe("recap-2026-10-29");
    expect(gets).toEqual(["recap/2026-10-29.json"]);
    expect(home.facts?.kind).toBe("recap");
    expect(home.facts?.lead.symbol).toBe("^GSPC");
  });

  it("defaults to the recap when it was published last", async () => {
    const { bucket, gets } = fakeR2();
    const later = seed + post(4, "recap", "2026-10-30", "2026-10-30T20:45:00Z", "recap/2026-10-29.json");
    const home = await loadHome(memoryD1(later), bucket, "en", undefined, TODAY);
    expect(home.tab).toBe("recap");
    expect(home.post?.slug).toBe("recap-2026-10-30");
    expect(gets).toHaveLength(1);
  });

  it("makes no R2 read without a post or without a facts key", async () => {
    const none = fakeR2();
    const empty = await loadHome(memoryD1(""), none.bucket, "en", "recap", TODAY);
    expect(empty).toMatchObject({ tab: null, tabs: [], post: null, facts: null });
    expect(none.gets).toEqual([]);

    const keyless = fakeR2();
    const home = await loadHome(
      memoryD1(post(1, "premarket", "2026-10-30", "2026-10-30T13:20:43Z", null)),
      keyless.bucket,
      "en",
      undefined,
      TODAY,
    );
    expect(home.post?.slug).toBe("premarket-2026-10-30");
    expect(home.facts).toBeNull();
    expect(keyless.gets).toEqual([]);
  });

  it("lists recent briefs without the two shown in the tabs", async () => {
    const { bucket } = fakeR2();
    const home = await loadHome(memoryD1(seed), bucket, "en", undefined, TODAY);
    expect(home.recent.map((p) => p.slug)).toEqual(["premarket-2026-10-29"]);
    expect(home.earnings).toEqual([]);
    expect(home.strip).toBeNull();
  });
});
