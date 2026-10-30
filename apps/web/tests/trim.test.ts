import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { trimStory } from "../src/lib/trim";

/** The live 2026-10-30 premarket bodies, as the generator stored them. */
const body = (lang: "en" | "zh") =>
  readFileSync(new URL(`./fixtures/premarket-2026-10-30.${lang}.md`, import.meta.url), "utf8");

const links = (md: string) => md.match(/\]\(https:\/\/[^)]+\)/g) ?? [];

describe("trimStory", () => {
  it("drops the EN title, summary and tables and keeps every section, the sources and the disclaimer", () => {
    const md = body("en");
    const out = trimStory(md);
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
    const out = trimStory(md);
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
    const out = trimStory(md);
    expect(out).toContain("First prose.");
    expect(out).toContain("Section\n---");
    expect(out).not.toContain("| 1 |");
  });

  it("ends a table at the next heading", () => {
    const out = trimStory("# T\n\nSummary.\n\n| A |\n| --- |\n| 1 |\n## Next\n\nText.\n");
    expect(out).toBe("## Next\n\nText.\n");
  });
});
