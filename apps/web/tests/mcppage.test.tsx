import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../messages/en.json";
import zh from "../messages/zh.json";
import type { Locale } from "../src/i18n/routing";
import { NOTICE, TOOLS } from "../src/lib/mcptools";
import { memoryD1 } from "./d1";

vi.mock("next-intl/server", () => ({ setRequestLocale: () => {} }));
// The page builds the address from the site origin: SITE_URL, else the request host.
vi.mock("next/headers", () => ({ headers: async () => new Headers({ host: "prebell.site" }) }));
// The language link reads the router; these tests only need it in the row.
vi.mock("../src/components/LangSwitchLink", () => ({
  default: ({ label, className }: { label: string; className: string }) => <a className={className}>{label}</a>,
}));
// The page, the footer link and the sitemap entry follow MCP_ENABLED, read from the Worker env as the route reads it.
const cf = vi.hoisted(() => ({ env: {} as Record<string, unknown> }));
vi.mock("@opennextjs/cloudflare", () => ({ getCloudflareContext: () => ({ env: cf.env, ctx: { waitUntil: () => {} } }) }));

const { default: Mcp, generateMetadata } = await import("../src/app/[locale]/mcp/page");
const { default: FooterLinks } = await import("../src/components/FooterLinks");
const { GET: sitemapGet } = await import("../src/app/sitemap.xml/route");

const props = (locale: Locale) => ({ params: Promise.resolve({ locale }) }) as never;
const render = async (locale: Locale) => renderToStaticMarkup(await Mcp(props(locale)));
const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/'/g, "&#x27;").replace(/"/g, "&quot;");
const COMMAND = "claude mcp add --transport http prebell https://prebell.site/mcp";
const OFF = [undefined, "false", "TRUE", "1", ""];

describe("For AI agents page", () => {
  beforeEach(() => {
    cf.env = { MCP_ENABLED: "true" };
  });
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("shows the address, the Claude Code command, the three tools and the limits, in both locales", async () => {
    for (const [locale, messages] of [["en", en], ["zh", zh]] as const) {
      const out = await render(locale);
      expect(out.startsWith('<article class="prose prose-mcp"><h1>')).toBe(true);
      expect(out).toContain(`<h1>${esc(messages.mcp.title)}</h1>`);
      expect(out).toContain(`<p>${esc(messages.mcp.intro)}</p>`);
      expect(out).toContain("<pre><code>https://prebell.site/mcp</code></pre>");
      expect(out).toContain(`<pre><code>${COMMAND}</code></pre>`);
      expect(out).toContain(esc(messages.mcp.otherClients));
      for (const tool of TOOLS) expect(out, tool.name).toContain(`<li><code>${tool.name}</code>`);
      expect((out.match(/<li><code>/g) ?? []).length).toBe(TOOLS.length);
      for (const q of messages.mcp.ask) expect(out, q).toContain(`<li>${esc(q)}</li>`);
      for (const l of messages.mcp.limits) expect(out, l).toContain(`<li>${esc(l)}</li>`);
      // The tools' own fixed disclaimer, word for word.
      expect(out).toContain(`<li>${esc(NOTICE[locale])}</li>`);
      expect(out).not.toMatch(/<(div|section)/);
    }
  });

  it("names exactly the server's tools, and keeps commands and tool names in English on the Chinese page", async () => {
    expect(Object.keys(en.mcp.tools)).toEqual(TOOLS.map((t) => t.name));
    expect(Object.keys(zh.mcp.tools)).toEqual(TOOLS.map((t) => t.name));
    const out = await render("zh");
    expect(out).toContain("<h1>AI 智能体接入</h1>");
    expect(out).toContain("<li><code>earnings_report</code>：");
    expect(out).toContain("Streamable HTTP");
    expect(out).toContain("美东时间上午 8:00");
    expect(out).toContain("8:35");
  });

  it("states the limits and makes no live-data or forecast claims", async () => {
    const out = await render("en");
    for (const fact of ["8:00 AM Eastern", "8:35", "4:45 PM Eastern", "no live quotes", "not investment advice", "no sign-in"]) {
      expect(out, fact).toContain(fact);
    }
    for (const messages of [en, zh]) {
      const text = JSON.stringify(messages.mcp);
      for (const banned of [/real-time/i, /\bbeat\b/i, /\bmiss(ed)?\b/i, /estimate/i, /consensus/i, /forecast/i, /预期/, /超出/]) {
        expect(text, String(banned)).not.toMatch(banned);
      }
    }
  });

  it("builds the address from the site origin", async () => {
    vi.stubEnv("SITE_URL", "http://localhost:8787");
    const out = await render("en");
    expect(out).toContain("<pre><code>http://localhost:8787/mcp</code></pre>");
    expect(out).toContain("<pre><code>claude mcp add --transport http prebell http://localhost:8787/mcp</code></pre>");
  });

  it("has a title, a description and paired alternates in both locales", async () => {
    expect(await generateMetadata(props("en"))).toMatchObject({
      title: "For AI agents",
      description: en.mcp.description,
      alternates: { canonical: "/en/mcp", languages: { en: "/en/mcp", "zh-Hans": "/zh/mcp", "x-default": "/en/mcp" } },
      openGraph: { url: "/en/mcp", title: "For AI agents" },
    });
    expect(await generateMetadata(props("zh"))).toMatchObject({ title: "AI 智能体接入", alternates: { canonical: "/zh/mcp" } });
  });

  it("answers 404 unless MCP_ENABLED is exactly \"true\"", async () => {
    for (const v of OFF) {
      cf.env = { MCP_ENABLED: v };
      for (const locale of ["en", "zh"] as const) {
        await expect(Mcp(props(locale)), String(v)).rejects.toMatchObject({ digest: "NEXT_HTTP_ERROR_FALLBACK;404" });
        expect(await generateMetadata(props(locale))).toEqual({});
      }
    }
  });

  it("styles its code blocks with palette tokens, wrapping on a phone instead of widening the page", () => {
    const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");
    const pre = css.match(/^\.prose-mcp > pre \{([^}]*)\}/m)?.[1] ?? "";
    expect(pre).toContain("white-space: pre-wrap");
    expect(pre).toContain("overflow-wrap: anywhere");
    expect(pre).not.toMatch(/#[0-9a-f]{3,6}\b/i);
  });
});

describe("footer link", () => {
  it("links For AI agents after About and before Privacy & Terms while the server is on", () => {
    cf.env = { MCP_ENABLED: "true" };
    const enOut = renderToStaticMarkup(<FooterLinks locale="en" />);
    expect(enOut).toContain('<a href="/en/about">About</a><a href="/en/mcp">For AI agents</a><a href="/en/legal">Privacy &amp; Terms</a>');
    const zhOut = renderToStaticMarkup(<FooterLinks locale="zh" />);
    expect(zhOut).toContain('<a href="/zh/about">关于</a><a href="/zh/mcp">AI 智能体接入</a><a href="/zh/legal">隐私与条款</a>');
  });

  it("is left out while the server is off", () => {
    for (const v of OFF) {
      cf.env = { MCP_ENABLED: v };
      for (const locale of ["en", "zh"] as const) {
        const out = renderToStaticMarkup(<FooterLinks locale={locale} />);
        expect(out, String(v)).not.toContain("/mcp");
        expect(out).toContain(`<a href="/${locale}/about">`);
      }
    }
  });

  it("lets the row wrap between links on a narrow phone", () => {
    const css = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");
    expect(css).toMatch(/^\.footer-links \{[^}]*flex-wrap: wrap/m);
    expect(css).toMatch(/^\.footer-links a \{[^}]*white-space: nowrap/m);
  });
});

describe("sitemap entry", () => {
  // No posts: the fixed pages only.
  const seed = "";
  const urls = async () => {
    const xml = await (await sitemapGet(new Request("https://prebell.site/sitemap.xml"))).text();
    return [...xml.matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => m[1]);
  };

  it("lists the page in both languages, with hreflang alternates, only while the server is on", async () => {
    cf.env = { MCP_ENABLED: "true", DB: memoryD1(seed) };
    const on = await urls();
    const about = on.indexOf("https://prebell.site/zh/about");
    expect(on.slice(about + 1, about + 3)).toEqual(["https://prebell.site/en/mcp", "https://prebell.site/zh/mcp"]);
    const xml = await (await sitemapGet(new Request("https://prebell.site/sitemap.xml"))).text();
    expect(xml).toContain('<xhtml:link rel="alternate" hreflang="zh-Hans" href="https://prebell.site/zh/mcp" />');

    for (const v of OFF) {
      cf.env = { MCP_ENABLED: v, DB: memoryD1(seed) };
      const off = await urls();
      expect(off).toContain("https://prebell.site/en/about");
      expect(off.some((u) => u.endsWith("/mcp")), String(v)).toBe(false);
    }
  });
});
