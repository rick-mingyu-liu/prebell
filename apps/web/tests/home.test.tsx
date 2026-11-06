import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import HomeView from "../src/components/HomeView";
import type { Locale } from "../src/i18n/routing";
import type { StripDay } from "../src/lib/earnings";
import { parseFacts } from "../src/lib/facts";
import type { HomeData } from "../src/lib/home";
import type { PostHead, PostListItem } from "../src/lib/posts";

const fixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const premarketFacts = () => parseFacts("premarket", fixture("premarket-2026-10-30.json"))!;

const lead = (over: Partial<PostHead> = {}): PostHead => ({
  slug: "premarket-2026-10-30",
  contentType: "premarket",
  tradingDate: "2026-10-30",
  publishedAt: "2026-10-30T12:40:00Z",
  title: "Futures slide as Intel sinks",
  summary: "Stock futures are lower before the open.",
  status: "published",
  hasOtherLang: true,
  factsKey: "premarket/2026-10-30.json",
  revisedAt: null,
  coverKey: "covers/premarket-2026-10-30.png",
  coverLines: ["S&P futures −1.14%", "Intel −26%"],
  ticker: null,
  ...over,
});

const item = (over: Partial<PostListItem> = {}): PostListItem => ({
  slug: "aapl-2026-10-29-earnings",
  contentType: "earnings",
  tradingDate: "2026-10-29",
  publishedAt: "2026-10-29T21:00:00Z",
  title: "Apple Q4: revenue up 8%",
  summary: "Services carried the quarter.",
  coverKey: "covers/aapl-2026-10-29-earnings.png",
  coverLines: ["Rev +8%", "EPS $1.85"],
  ticker: "AAPL",
  ...over,
});

const recap = item({
  slug: "recap-2026-10-29",
  contentType: "recap",
  publishedAt: "2026-10-29T20:45:00Z",
  title: "Stocks close mixed",
  summary: "The S&P 500 ended flat.",
  coverKey: null,
  coverLines: null,
  ticker: null,
});

const reporting: StripDay = {
  date: "2026-11-02",
  bmo: [{ ticker: "KO", date: "2026-11-02", slot: "bmo", status: "confirmed", postSlug: null, name: null, writeUps: true, filingUrl: null }],
  amc: [
    { ticker: "AAPL", date: "2026-11-02", slot: "amc", status: "confirmed", postSlug: null, name: null, writeUps: true, filingUrl: null },
    { ticker: "BRK.B", date: "2026-11-02", slot: "amc", status: "unconfirmed", postSlug: null, name: null, writeUps: true, filingUrl: null },
  ],
  unknown: [],
};

const data = (over: Partial<HomeData> = {}): HomeData => ({
  lead: lead(),
  leadFacts: premarketFacts(),
  earnings: [item()],
  recent: [recap],
  reporting,
  ...over,
});

const render = (over: Partial<HomeData> = {}, locale: Locale = "en", signup = true) =>
  renderToStaticMarkup(<HomeView data={data(over)} locale={locale} signup={signup} />);

/** Where each section opens in the markup, in document order. */
const at = (out: string, cls: string) => {
  const i = out.indexOf(`class="${cls}"`);
  expect(i, cls).toBeGreaterThan(-1);
  return i;
};

describe("HomeView front page", () => {
  it("renders lead, earnings row, recent list, movers column and reporting block in order", () => {
    const out = render();
    // The lead graphic's preload link comes first in a fragment; in a full document React moves it to <head>.
    expect(out).toMatch(/^(<link rel="preload" as="image" [^>]*\/>)?<div class="front">/);
    const order = ["front-lead", "front-earnings", "front-recent", "front-side", "front-movers", "front-reporting"];
    const offsets = order.map((c) => at(out, c));
    expect([...offsets].sort((a, b) => a - b)).toEqual(offsets);

    // Lead: edition kicker, wide cover with the date-and-type label, serif h1, summary, read link.
    expect(out).toContain("Before the bell");
    expect(out).toContain('class="cover cover-lead"');
    expect(out).toContain('src="/covers/premarket-2026-10-30.png"');
    expect(out).toContain(">Oct 30 · Premarket</span>");
    expect(out).toContain("S&amp;P futures −1.14%");
    expect(out).toMatch(/<h1[^>]*><a href="\/en\/p\/premarket-2026-10-30">Futures slide as Intel sinks<\/a><\/h1>/);
    expect(out).toContain("Stock futures are lower before the open.");
    expect(out).toContain("Read the full brief →");
    expect(out).not.toContain("recap →");

    // Earnings checks: a small cover with the company logo, a serif headline linking to the check.
    expect(out).toContain('<h2 class="section-title">Earnings checks</h2>');
    expect(out).toContain('class="cover cover-small"');
    expect(out).toMatch(/<img src="\/logos\/companies\/AAPL\.(svg|png)" alt="AAPL" loading="lazy"\/>/);
    expect(out).toContain('href="/en/p/aapl-2026-10-29-earnings"');
    expect(out).toContain("Apple Q4: revenue up 8%");

    // Recent: kicker, title, date.
    expect(out).toContain('<h2 class="section-title">Recent</h2>');
    expect(out).toContain(">After-close recap</p>");
    expect(out).toContain('href="/en/p/recap-2026-10-29"');
    expect(out).toContain("Oct 29, 2026");

    // Movers: three per side; name, grey ticker, signed percent in the number colours.
    expect(out).toContain('<h2 class="section-title">Top movers</h2>');
    expect(out).toContain('<ol class="movers-col">');
    for (const s of ["MDLZ", "REGN", "CL", "INTC", "HPQ", "DELL"]) expect(out).toContain(`>${s}</span>`);
    for (const s of ["AZN", "EFX", "SYK", "ARM"]) expect(out).not.toContain(`>${s}</span>`);
    expect(out).toContain("Mondelez International");
    expect(out).toContain('class="num up">+7.08%');
    expect(out).toContain('class="num down">−26.00%');
    expect(out).toContain("Shares surged in premarket trading");

    // Reporting: the found date, the slots, tickers linking to their pages.
    expect(out).toContain("Reporting Nov 2, 2026");
    expect(out).toContain("Before the open");
    expect(out).toContain("After the close");
    expect(out).toContain('href="/en/t/KO"');
    expect(out).toContain('href="/en/t/BRK.B"');
    expect(out.indexOf("/en/t/KO")).toBeLessThan(out.indexOf("/en/t/AAPL"));

    // The weekly-digest signup and the partner strip close the page.
    expect(out.indexOf('class="signup"')).toBeGreaterThan(offsets[offsets.length - 1]);
    expect(out.indexOf('class="partners-strip"')).toBeGreaterThan(out.indexOf('class="signup"'));
  });

  describe("themes block", () => {
    const themed = () => {
      const facts = parseFacts("premarket", {
        ...(fixture("premarket-2026-10-30.json") as Record<string, unknown>),
        themes: [
          {
            group: "Memory & storage",
            group_zh: "存储芯片",
            direction: "Down",
            members: [
              { ticker: "MU", name: "Micron", change_pct: -4.12 },
              { ticker: "SNDK", name: "SanDisk", change_pct: -6.01 },
              { ticker: "WDC", name: "Western Digital", change_pct: -3.5 },
              { ticker: "STX", name: "Seagate", change_pct: -3.2 },
            ],
            more: 2,
            median_pct: -4.1,
            reasons: [],
          },
          {
            group: "Banks",
            group_zh: "银行",
            direction: "Up",
            members: [{ ticker: "JPM", name: "JPMorgan", change_pct: 1.5 }],
            more: 0,
            median_pct: 1.5,
            reasons: [],
          },
        ],
      })!;
      return { leadFacts: facts };
    };

    it("renders under the movers and before reporting, with the English labels", () => {
      const out = render(themed());
      expect(out).toContain('<h2 class="section-title">Themes</h2>');
      expect(out).toContain("Memory &amp; storage");
      expect(out).toContain("Banks");
      expect(out.match(/class="theme-row"/g)).toHaveLength(2);
      expect(at(out, "front-movers")).toBeLessThan(at(out, "front-themes"));
      expect(at(out, "front-themes")).toBeLessThan(at(out, "front-reporting"));
      expect(out).toContain('class="num down"><span aria-hidden="true">▼</span><span class="sr-only">down </span>−4.10%');
      expect(out).toContain('class="num up"><span aria-hidden="true">▲</span><span class="sr-only">up </span>+1.50%');
      // First three tickers with their moves; the fourth listed one and the generator's "more" 2 are counted.
      for (const s of ["MU", "SNDK", "WDC"]) expect(out).toContain(`${s} <span class="num">`);
      expect(out).not.toContain("STX");
      expect(out).toContain('class="theme-more">+3<');
      expect(out).toContain("−6.01%");
      // The whole row links to the lead post.
      expect(out).toContain('<a class="theme-link" href="/en/p/premarket-2026-10-30">');
    });

    it("shows the Chinese labels on the ZH page", () => {
      const out = render(themed(), "zh");
      expect(out).toContain('<h2 class="section-title">板块动向</h2>');
      expect(out).toContain("存储芯片");
      expect(out).toContain("银行");
      expect(out).not.toContain("Memory &amp; storage");
      expect(out).toContain('href="/zh/p/premarket-2026-10-30"');
    });

    it("shows at most three themes", () => {
      const facts = themed().leadFacts;
      const four = { ...facts, themes: [...facts.themes, ...facts.themes.map((t) => ({ ...t, group: t.group + "2" }))] };
      expect(render({ leadFacts: four }).match(/class="theme-row"/g)).toHaveLength(3);
    });

    it("is absent when there are no themes", () => {
      const out = render();
      expect(out).not.toContain("front-themes");
      expect(out).not.toContain("Themes");
    });
  });

  it("does not carry a not-advice line; that statement lives on the legal page", () => {
    for (const locale of ["en", "zh"] as const) {
      const out = render({}, locale);
      expect(out).not.toMatch(/investment advice/i);
      expect(out).not.toContain("投资建议");
      expect(out).not.toContain("disclaimer");
    }
  });

  it("leaves the signup out until the Buttondown key is set, keeping the partner strip", () => {
    const out = render({}, "en", false);
    expect(out).not.toContain('class="signup"');
    expect(out).not.toContain("Get the weekly digest");
    expect(out).toContain('class="partners-strip"');
    expect(render({}, "en", true)).toContain('id="signup-email-home"');
  });

  it('renders "No clear news" quietly in the movers column', () => {
    const out = render();
    expect(out).toContain('<p class="mover-why muted"><i>No clear news</i></p>');
    // No pills, tiles, logos or letter avatars in the rows.
    for (const cls of ["pill", "tlogo", "tiles", "big"]) expect(out).not.toMatch(new RegExp(`class="[^"]*\\b${cls}\\b`));
  });

  it("links a reason to its source", () => {
    const out = render();
    expect(out).toMatch(
      /<a href="https:\/\/finance\.yahoo\.com\/[^"]*" rel="nofollow noopener" target="_blank">Shares surged in premarket trading/,
    );
  });

  it("shows the empty earnings message", () => {
    const out = render({ earnings: [] });
    expect(out).toContain('<h2 class="section-title">Earnings checks</h2>');
    expect(out).toContain("No earnings checks yet.");
    expect(out).not.toContain("cover-small");
    expect(render({ earnings: [] }, "zh")).toContain("暂无财报速评。");
  });

  it("labels a check without a ticker as an earnings check", () => {
    const out = render({ earnings: [item({ ticker: null, title: "Amgen: revenue up 8%" })] });
    expect(out).toContain('<span class="cover-tag">Earnings check</span>');
    const zh = render({ earnings: [item({ ticker: null, title: "安进：营收增长8%" })] }, "zh");
    expect(zh).toContain('<span class="cover-tag">财报速评</span>');
  });

  it("names the recap edition in the kicker, the cover label and the link", () => {
    const out = render({
      lead: lead({ slug: "recap-2026-10-30", contentType: "recap", title: "Stocks rally" }),
      leadFacts: null,
    });
    expect(out).toContain("After the close");
    expect(out).toContain(">Oct 30 · Recap</span>");
    expect(out).toContain("Read the full recap →");
    expect(render({ lead: lead({ contentType: "recap" }) }, "zh")).toContain(">10月30日 · 收盘回顾</span>");
    expect(out).toContain('href="/en/p/recap-2026-10-30"');
  });

  it("points to the brief for movers when the facts are missing, and says so when there are none", () => {
    const missing = render({ leadFacts: null });
    expect(missing).not.toContain("movers-col");
    expect(missing).toContain("Movers are in the full brief →");

    const none = render({ leadFacts: { ...premarketFacts(), gainers: [], losers: [] } });
    expect(none).toContain("No movers for this brief.");
    expect(none).not.toContain("Movers are in the full brief");
  });

  it("drops the reporting block when nothing is on the calendar", () => {
    const out = render({ reporting: null });
    expect(out).not.toContain("front-reporting");
    expect(out).not.toContain("Reporting");
  });

  it("drops the recent section when there is nothing else", () => {
    expect(render({ recent: [] })).not.toContain("front-recent");
  });

  it("says nothing is published when there is no lead", () => {
    const out = render({ lead: null, leadFacts: null, earnings: [], recent: [], reporting: null });
    expect(out).toContain("Nothing has been published yet.");
    expect(out).not.toContain("front-movers");
    expect(out).toContain("No earnings checks yet.");
  });

  it("never renders the facts diagnostics, the old tabs or a script", () => {
    const out = render();
    expect(out).not.toContain("diagnostics");
    expect(out).not.toContain("?tab=");
    expect(out).not.toContain('type="radio"');
    expect(out).not.toContain("estrip");
    expect(out).not.toContain('class="sidebar"');
    expect(out).not.toContain("<script");
  });
});

describe("HomeView ZH", () => {
  it("omits the English reasons and uses Chinese labels", () => {
    const out = render({}, "zh");
    expect(out).toContain("盘前");
    expect(out).toContain(">10月30日 · 盘前</span>");
    expect(out).toContain("<h2 class=\"section-title\">涨跌榜</h2>");
    expect(out).toContain(">INTC</span>");
    expect(out).not.toContain("Shares surged");
    expect(out).not.toContain("mover-why");
    expect(out).toContain("2026年11月2日财报");
    expect(out).toContain('href="/zh/t/KO"');
  });

  it("keeps the cover and movers of a pending lead and shows the notice instead of the story", () => {
    const out = render(
      // A pending row may carry stale text; none of it may show.
      { lead: lead({ status: "pending", title: "旧标题", summary: "旧摘要", coverLines: ["旧行", ""] }) },
      "zh",
    );
    expect(out).not.toContain("旧标题");
    expect(out).not.toContain("旧摘要");
    expect(out).not.toContain("旧行");
    expect(out).toContain('class="cover cover-lead"');
    expect(out).toContain(">INTC</span>");
    expect(out).toContain("中文版稍后推出");
    expect(out).toContain('href="/en/p/premarket-2026-10-30"');
    expect(out).toMatch(/<h1[^>]*>盘前简报 2026年10月30日<\/h1>/);
    expect(out).not.toContain("阅读完整简报");
  });
});
