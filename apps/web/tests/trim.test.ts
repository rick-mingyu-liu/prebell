import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { splitTail, trimHead, trimStory } from "../src/lib/trim";

/** The live 2026-10-30 premarket bodies, as the generator stored them. */
const body = (lang: "en" | "zh") =>
  readFileSync(new URL(`./fixtures/premarket-2026-10-30.${lang}.md`, import.meta.url), "utf8");

/** The summary column, which the generator also writes as the body's first paragraph. */
const summaryOf = (md: string) => md.split("\n\n")[1];

const links = (md: string) => md.match(/\]\(https:\/\/[^)]+\)/g) ?? [];

describe("trimStory", () => {
  it("drops the EN title, summary and tables and keeps every section, the sources and the disclaimer", () => {
    const md = body("en");
    const out = trimStory(md, summaryOf(md));
    expect(out).not.toMatch(/^\|/m);
    expect(out).not.toMatch(/^# /m);
    expect(out).not.toContain("Tech earnings drag futures lower");
    expect(out).not.toContain("US futures fell for a second day as weak tech earnings drove a rotation");
    for (const h of ["## Futures", "## Biggest premarket movers", "## Macro & overnight", "## What to watch", "## Sources"])
      expect(out).toContain(h);
    expect(out).toContain("S&P 500 futures were down 1.14%");
    expect(links(out)).toEqual(links(md));
    expect(links(out)).toHaveLength(11);
    // A source title with a pipe is a list item, not a table row.
    expect(out).toContain("(AZN:NYSE) | Seeking Alpha](");
    expect(out.trimEnd().endsWith("It is not investment advice.")).toBe(true);
    expect(out.startsWith("## Futures")).toBe(true);
  });

  it("drops the ZH title, summary and tables and keeps every section, the sources and the disclaimer", () => {
    const md = body("zh");
    const out = trimStory(md, summaryOf(md));
    expect(out).not.toMatch(/^\|/m);
    expect(out).not.toMatch(/^# /m);
    expect(out).not.toContain("科技股财报拖累");
    expect(out).not.toContain("科技股财报不佳引发资金撤离");
    for (const h of ["## 期货", "## 盘前异动", "## 宏观与隔夜", "## 今日关注", "## 来源"]) expect(out).toContain(h);
    expect(links(out)).toEqual(links(md));
    expect(out.trimEnd().endsWith("本简报根据公开数据自动生成，仅供参考，不构成投资建议。")).toBe(true);
  });

  it("keeps a paragraph after a table when there is no summary, and a pipe-free setext heading", () => {
    const md = "# Title\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nFirst prose.\n\nSection\n---\n\nMore.\n";
    const out = trimStory(md, "Summary.");
    expect(out).toContain("First prose.");
    expect(out).toContain("Section\n---");
    expect(out).not.toContain("| 1 |");
  });

  it("ends a table at the next heading", () => {
    const out = trimStory("# T\n\nSummary.\n\n| A |\n| --- |\n| 1 |\n## Next\n\nText.\n", "Summary.");
    expect(out).toBe("## Next\n\nText.\n");
  });

  it("keeps the first paragraph when it is not the summary", () => {
    expect(trimStory("Real first paragraph.\n\n## S\n\nBody.\n", "Another summary.")).toBe(
      "Real first paragraph.\n\n## S\n\nBody.\n",
    );
    expect(trimStory("# T\n\nReal first paragraph.\n\n## S\n", "Another summary.")).toBe(
      "Real first paragraph.\n\n## S\n",
    );
  });

  it("matches a multi-line summary by its words and stops it at a heading", () => {
    expect(trimStory("# T\n\nStocks fell\nas yields rose.\n## Futures\n\nFirst section.\n", "Stocks fell as yields rose.")).toBe(
      "## Futures\n\nFirst section.\n",
    );
  });

  it("drops a BOM-prefixed, indented or setext title, and handles CRLF", () => {
    for (const md of [
      "\uFEFF# Title\n\nSum.\n\n## S\n",
      "   # Title\n\nSum.\n\n## S\n",
      "Title\n=====\n\nSum.\n\n## S\n",
      "# Title\r\n\r\nSum.\r\n\r\n| A |\r\n|---|\r\n| 1 |\r\n\r\n## S\r\n",
    ])
      expect(trimStory(md, "Sum.")).toBe("## S\n");
  });

  it("leaves pipes inside code fences alone", () => {
    const md = "# T\n\nSum.\n\n## S\n\n```\n| a |\n|---|\n```\n\n[l](https://x.test) after\n";
    expect(trimStory(md, "Sum.")).toBe("## S\n\n```\n| a |\n|---|\n```\n\n[l](https://x.test) after\n");
  });

  it("ends a table at a blockquote or fence", () => {
    expect(trimStory("## S\n\n| a |\n|---|\n| 1 |\n> quote\n")).toBe("## S\n\n> quote\n");
    expect(trimStory("## S\n\n| a |\n|---|\n| 1 |\n```\ncode\n```\n")).toBe("## S\n\n```\ncode\n```\n");
  });

  it("checks a long near-delimiter line in linear time", () => {
    const md = `## S\n\n| a |\n|-${" ".repeat(50_000)}x\n`;
    const start = performance.now();
    trimStory(md);
    // A generous bound: the quadratic regex this guards against takes far longer at this size.
    expect(performance.now() - start).toBeLessThan(1000);
  });
});

describe("trimHead", () => {
  it("drops the title and summary and keeps the tables", () => {
    const md = body("en");
    const out = trimHead(md, summaryOf(md));
    expect(out).not.toMatch(/^# /m);
    expect(out).not.toContain("US futures fell for a second day as weak tech earnings drove a rotation");
    expect(out.startsWith("| Contract | Last | Change |")).toBe(true);
    expect(out).toContain("| INTC | Intel | 68.13 | -26.00% |");
    expect(out).toContain("## Sources");
  });

  it("keeps a first paragraph that is not the summary", () => {
    expect(trimHead("# T\n\nSomething else.\n\n## A\n\nx\n", "The summary.")).toBe("Something else.\n\n## A\n\nx\n");
  });
});

describe("splitTail", () => {
  it("splits the sources list and the closing disclaimer off a brief", () => {
    const md = body("en");
    const { body: rest, sources, disclaimer } = splitTail(trimStory(md, summaryOf(md)));
    expect(rest.startsWith("## Futures")).toBe(true);
    expect(rest.trimEnd().endsWith("## What to watch\n\nn/a")).toBe(true);
    expect(rest).not.toContain("## Sources");
    expect(sources.startsWith("- [Why Is Mondelez")).toBe(true);
    expect(links(sources)).toEqual(links(md));
    expect(disclaimer).toBe("This brief is generated automatically from public data for information only. It is not investment advice.");
  });

  it("splits a ZH brief at its 来源 heading", () => {
    const { body: rest, sources, disclaimer } = splitTail(trimStory(body("zh")));
    expect(rest).not.toContain("## 来源");
    expect(links(sources)).toHaveLength(links(body("zh")).length);
    expect(disclaimer).toBe("本简报根据公开数据自动生成，仅供参考，不构成投资建议。");
  });

  it("finds the disclaimer without a sources section, and nothing without a section heading", () => {
    expect(splitTail("## A\n\nText.\n\nNot advice.\n")).toEqual({ body: "## A\n\nText.\n", sources: "", disclaimer: "Not advice." });
    expect(splitTail("Only a paragraph.\n")).toEqual({ body: "Only a paragraph.\n", sources: "", disclaimer: "" });
    expect(splitTail("## A\n\n- one\n- two\n")).toEqual({ body: "## A\n\n- one\n- two\n", sources: "", disclaimer: "" });
  });

  it("ignores a sources heading or paragraph inside a code fence", () => {
    const md = "## A\n\n```\n## Sources\n\nlast\n```\n";
    expect(splitTail(md)).toEqual({ body: md, sources: "", disclaimer: "" });
  });
});
