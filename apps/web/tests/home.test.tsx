import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import HomeView, { type HomeViewProps } from "../src/components/HomeView";
import type { StripDay } from "../src/lib/earnings";
import { parseFacts } from "../src/lib/facts";
import { selectTab } from "../src/lib/home";
import type { PostListItem, PostView } from "../src/lib/posts";

const fixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const premarketFacts = () => parseFacts("premarket", fixture("premarket-2026-10-30.json"))!;

const post = (over: Partial<PostView> = {}): PostView => ({
  slug: "premarket-2026-10-30",
  contentType: "premarket",
  tradingDate: "2026-10-30",
  publishedAt: "2026-10-30T12:40:00Z",
  title: "Futures slide as Intel sinks",
  summary: "Stock futures are lower before the open.",
  bodyMd: "# Futures slide as Intel sinks\n\nStock futures are lower before the open.",
  status: "published",
  sources: [],
  hasOtherLang: true,
  factsKey: "premarket/2026-10-30.json",
  coverKey: null,
  coverLines: null,
  ticker: null,
  ...over,
});

const recapPost = (over: Partial<PostView> = {}): PostView =>
  post({
    slug: "recap-2026-10-29",
    contentType: "recap",
    tradingDate: "2026-10-29",
    publishedAt: "2026-10-29T20:45:00Z",
    title: "Stocks close mixed",
    summary: "The S&P 500 ended flat.",
    factsKey: "recap/2026-10-29.json",
    ...over,
  });

const check: PostListItem = {
  slug: "earnings-aapl-2026-10-29",
  contentType: "earnings",
  tradingDate: "2026-10-29",
  publishedAt: "2026-10-29T21:00:00Z",
  title: "Apple Q4: revenue up 8%",
  summary: "Services carried the quarter.",
  coverKey: null,
  coverLines: null,
  ticker: "AAPL",
};

const props = (over: Partial<HomeViewProps> = {}): HomeViewProps => ({
  locale: "en",
  tab: "premarket",
  tabs: ["premarket", "recap"],
  post: post(),
  facts: premarketFacts(),
  earnings: [check],
  strip: null,
  today: "2026-10-30",
  ...over,
});

const render = (over: Partial<HomeViewProps> = {}) => renderToStaticMarkup(<HomeView {...props(over)} />);

describe("selectTab", () => {
  const pm = post();
  const rc = recapPost();

  it("defaults to the more recently published of the two", () => {
    expect(selectTab(pm, rc, undefined)).toBe("premarket");
    expect(selectTab(pm, recapPost({ publishedAt: "2026-10-30T20:45:00Z" }), undefined)).toBe("recap");
  });

  it("uses the requested tab when that post exists", () => {
    expect(selectTab(pm, rc, "recap")).toBe("recap");
    expect(selectTab(pm, rc, ["recap", "premarket"])).toBe("recap");
    expect(selectTab(pm, null, "recap")).toBe("premarket");
    expect(selectTab(pm, rc, "earnings")).toBe("premarket");
  });

  it("takes whichever exists, prefers premarket on a tie, and is null with no posts", () => {
    expect(selectTab(null, rc, undefined)).toBe("recap");
    expect(selectTab(pm, recapPost({ publishedAt: pm.publishedAt }), undefined)).toBe("premarket");
    expect(selectTab(post({ publishedAt: null }), rc, undefined)).toBe("recap");
    expect(selectTab(null, null, "recap")).toBeNull();
  });
});

describe("HomeView with facts", () => {
  it("shows the lead number, its change and the index tiles", () => {
    const out = render();
    expect(out).toContain("S&amp;P 500 futures");
    expect(out).toContain('class="big"');
    expect(out).toContain("7,665.50");
    expect(out).toContain("−1.14%");
    expect(out).toContain('class="tiles"');
  });

  it("shows the headline, summary and a link to the brief", () => {
    const out = render();
    expect(out).toContain("Futures slide as Intel sinks");
    expect(out).toContain("Stock futures are lower before the open.");
    expect(out).toContain('href="/en/p/premarket-2026-10-30"');
    expect(out).toContain("Read the full brief →");
    expect(out).toMatch(/<h1[^>]*>.*Futures slide as Intel sinks/);
  });

  it("shows three movers per side as pills", () => {
    const out = render();
    for (const s of ["MDLZ", "REGN", "CL", "INTC", "HPQ", "DELL"]) expect(out).toContain(`<b>${s}</b>`);
    for (const s of ["AZN", "EFX", "SYK", "ARM"]) expect(out).not.toContain(`<b>${s}</b>`);
    expect(out).toMatch(/class="pill up"/);
    expect(out).toMatch(/class="pill down"/);
  });

  it("never renders the facts diagnostics", () => {
    expect(render()).not.toContain("diagnostics");
  });
});

describe("HomeView without facts", () => {
  it("falls back to the title, summary and link", () => {
    const out = render({ facts: null });
    expect(out).not.toContain('class="big"');
    expect(out).not.toContain('class="pill');
    expect(out).toContain("Futures slide as Intel sinks");
    expect(out).toContain("Stock futures are lower before the open.");
    expect(out).toContain('href="/en/p/premarket-2026-10-30"');
    expect(out).toContain("Movers are in the full brief →");
    expect(out).not.toContain("No movers for this brief.");
  });

  it("says there were no movers only when the facts have none", () => {
    const facts = { ...premarketFacts(), gainers: [], losers: [] };
    const out = render({ facts });
    expect(out).toContain('class="big"');
    expect(out).toContain("No movers for this brief.");
    expect(out).not.toContain("Movers are in the full brief");
  });
});

describe("HomeView ZH pending", () => {
  it("keeps the numbers and movers, and shows the notice instead of the story", () => {
    const out = render({
      locale: "zh",
      // A pending row may carry stale text; none of it may show.
      post: post({ status: "pending", title: "旧标题", summary: "旧摘要", hasOtherLang: true }),
    });
    expect(out).not.toContain("旧标题");
    expect(out).not.toContain("旧摘要");
    expect(out).not.toContain('class="home-sum"');
    expect(out).toContain("标普500期货");
    expect(out).toContain("7,665.50");
    expect(out).toContain("<b>INTC</b>");
    expect(out).toContain("中文版稍后推出");
    expect(out).toContain('href="/en/p/premarket-2026-10-30"');
    expect(out).toMatch(/<h1[^>]*>盘前简报 2026年10月30日<\/h1>/);
    expect(out).not.toContain("阅读完整简报");
  });
});

/** The brief tab link with this text: its href and whether it is marked current. */
function tabLink(out: string, text: string) {
  const a = out.match(new RegExp(`<a ([^>]*)>${text}</a>`))?.[1] ?? "";
  return { href: a.match(/href="([^"]*)"/)?.[1], current: a.includes('aria-current="page"') };
}

describe("HomeView tabs", () => {
  it("links both brief tabs and marks the selected one", () => {
    const out = render();
    expect(tabLink(out, "Premarket brief")).toEqual({ href: "/en?tab=premarket", current: true });
    expect(tabLink(out, "After-close recap")).toEqual({ href: "/en?tab=recap", current: false });
  });

  it("only offers tabs that have a post", () => {
    const out = render({ tabs: ["premarket"] });
    expect(out).toContain('href="/en?tab=premarket"');
    expect(out).not.toContain("tab=recap");
  });

  it("selects the recap with its own link text", () => {
    const out = render({ tab: "recap", post: recapPost(), facts: null, locale: "zh" });
    expect(tabLink(out, "收盘回顾")).toEqual({ href: "/zh?tab=recap", current: true });
    expect(out).toContain('href="/zh/p/recap-2026-10-29"');
  });

  it("puts movers and earnings checks behind a CSS-only toggle, movers first", () => {
    const out = render();
    expect(out).toMatch(/<input[^>]*type="radio"[^>]*checked=""/);
    expect(out).toMatch(/role="radiogroup" aria-label="Movers and earnings checks"/);
    // Headings for the stacked (below 1024px) layout, where the radios are hidden.
    expect(out).toContain('<h2 class="panel-title">Top movers</h2>');
    expect(out).toContain('<h2 class="panel-title">Earnings checks</h2>');
    expect(out.indexOf("panel-movers")).toBeLessThan(out.indexOf("panel-earnings"));
    expect(out).toContain("Top movers");
    expect(out).toContain("Earnings checks");
    expect(out).not.toContain("<script");
  });
});

describe("HomeView earnings checks", () => {
  it("lists the latest checks and links to all of them", () => {
    const out = render();
    expect(out).toContain('href="/en/p/earnings-aapl-2026-10-29"');
    expect(out).toContain("Apple Q4: revenue up 8%");
    expect(out).toContain("Services carried the quarter.");
    expect(out).toContain('href="/en/archive?type=earnings"');
    expect(out).toContain("All earnings checks →");
  });

  it("says so when there are none (ZH)", () => {
    const out = render({ locale: "zh", earnings: [] });
    expect(out).toContain("暂无财报速评。");
    expect(out).toContain(">财报速评</label>");
    expect(out).toContain("全部财报速评 →");
    expect(out).toContain('href="/zh/archive?type=earnings"');
  });
});

describe("HomeView shell", () => {
  const day: StripDay = {
    date: "2026-11-02",
    bmo: [{ ticker: "KO", date: "2026-11-02", slot: "bmo", status: "confirmed", postSlug: null }],
    amc: [],
    unknown: [],
  };

  it("puts the earnings strip first, then the portal grid with the sidebar", () => {
    const out = render({ strip: day });
    expect(out.startsWith('<section class="estrip"')).toBe(true);
    expect(out).toContain("Earnings · Mon, Nov 2");
    expect(out).toContain("grid-portal");
    expect(out).toContain('href="https://www.termpocket.com"');
    for (const host of ["moomoo.com", "okx.com", "crypto.com", "wealthsimple.com"]) expect(out).not.toContain(host);
  });

  it("renders recent posts passed as children before the sidebar", () => {
    const out = renderToStaticMarkup(
      <HomeView {...props()}>
        <section id="recent-slot">recent</section>
      </HomeView>,
    );
    expect(out.indexOf("recent-slot")).toBeGreaterThan(out.indexOf("All earnings checks"));
    expect(out.indexOf("recent-slot")).toBeLessThan(out.indexOf('class="sidebar"'));
  });

  it("says nothing is published when there is no brief", () => {
    const out = render({ tab: null, tabs: [], post: null, facts: null });
    expect(out).toContain("Nothing has been published yet.");
    expect(out).not.toContain("tab=");
  });
});
