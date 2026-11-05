import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import CalendarWeek from "../src/components/CalendarWeek";
import PostList from "../src/components/PostList";
import type { CalendarDay } from "../src/lib/earnings";
import type { PostListItem } from "../src/lib/posts";

const post: PostListItem = {
  slug: "2026-10-29-recap",
  contentType: "recap",
  tradingDate: "2026-10-29",
  title: "Stocks slip into the close",
  summary: "Tech led the drop.",
  publishedAt: null,
  coverKey: null,
  coverLines: null,
  ticker: null,
};

describe("PostList", () => {
  const withCover: PostListItem = {
    ...post,
    slug: "aapl-2026-10-29-earnings",
    contentType: "earnings",
    title: "Apple Q4: revenue up 8%",
    coverKey: "covers/aapl-2026-10-29-earnings.png",
    coverLines: ["Rev +8%", "EPS $1.85"],
    ticker: "AAPL",
  };

  it("archive rows show a small cover (flat when the post has none), kicker, title and date", () => {
    const out = renderToStaticMarkup(<PostList locale="en" items={[post, withCover]} />);
    const [flat, scene] = out.split('<li class="post-row">').slice(1);
    // Every row: small cover, then the kicker, the serif title link, the summary and the date.
    expect(flat).toContain('class="cover cover-small"');
    expect(flat).toContain('<p class="kicker">After-close recap</p>');
    expect(flat).toMatch(/<h3 class="story-title"><a href="\/en\/p\/2026-10-29-recap">Stocks slip into the close<\/a><\/h3>/);
    expect(flat).toContain("Tech led the drop.");
    expect(flat).toContain('<time class="story-meta" dateTime="2026-10-29">Oct 29, 2026</time>');
    expect(flat.indexOf("cover-small")).toBeLessThan(flat.indexOf("kicker"));
    expect(flat.indexOf("kicker")).toBeLessThan(flat.indexOf("story-title"));
    expect(flat.indexOf("story-title")).toBeLessThan(flat.indexOf("story-meta"));
    // No scene and no lines: the flat near-black cover with only its tag.
    expect(flat).not.toContain("cover-bg");
    expect(flat).not.toContain("cover-stat");
    expect(flat).toContain('<span class="cover-tag">Oct 29 · Recap</span>');
    // A post with a scene and lines shows both, and the company logo as its tag.
    expect(scene).toContain('src="/covers/aapl-2026-10-29-earnings.png"');
    expect(scene).toContain("Rev +8%");
    expect(scene).toContain('alt="AAPL"');
    expect(scene).toContain('<p class="kicker">Earnings check</p>');
    // The type is the kicker's job: no tag pill.
    expect(out).not.toContain('class="tag"');
    expect(out).not.toContain('class="row');
  });

  it("labels the kicker and the cover tag in Chinese", () => {
    const out = renderToStaticMarkup(<PostList locale="zh" items={[post]} />);
    expect(out).toContain('<p class="kicker">收盘回顾</p>');
    expect(out).toContain("10月29日 · 收盘");
  });
});

describe("CalendarWeek", () => {
  const days: CalendarDay[] = [
    {
      date: "2026-11-02",
      entries: [
        { ticker: "AMD", date: "2026-11-02", slot: "amc", status: "confirmed", postSlug: null },
        { ticker: "UNH", date: "2026-11-02", slot: "bmo", status: "unconfirmed", postSlug: null },
        { ticker: "INTC", date: "2026-11-02", slot: "bmo", status: "reported", postSlug: "intc-q3" },
        { ticker: "UBER", date: "2026-11-02", slot: "unknown", status: "unconfirmed", postSlug: null },
      ],
    },
    { date: "2026-11-03", entries: [] },
  ];

  it("calendar week renders ruled day rows without badges", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" />);
    // Ruled day rows, no chips, badges or logos.
    expect(out).toContain('class="cal-day"');
    expect(out).not.toMatch(/badge|cal-chip|tlogo|<img/);
    // Each entry: a ticker link to the company page and its slot label.
    expect(out).toContain('<a class="cal-ticker" href="/en/t/UNH">UNH</a>');
    expect(out).toContain('<a class="cal-ticker" href="/en/t/AMD">AMD</a>');
    expect(out).toContain('<a class="cal-ticker" href="/en/t/INTC">INTC</a>');
    expect(out).toContain('<span class="cal-slot">Before open</span>');
    expect(out).toContain('<span class="cal-slot">After close</span>');
    expect(out).toContain('<span class="cal-slot">Time n/a</span>');
    // Slots stay in order: before open, after close, time not set.
    expect(out.indexOf(">UNH<")).toBeLessThan(out.indexOf(">AMD<"));
    expect(out.indexOf(">AMD<")).toBeLessThan(out.indexOf(">UBER<"));
    // The one coloured mark is the tick on a reported entry; the status is also in text for screen readers.
    expect(out.match(/num up/g)).toHaveLength(1);
    expect(out).toMatch(/<li class="cal-entry cal-reported">.*?<span class="num up" aria-hidden="true">✓<\/span><span class="sr-only">Reported<\/span>/);
    // Unconfirmed entries are muted and say so in text.
    expect(out).toMatch(/<li class="cal-entry cal-unconfirmed">.*?<span class="sr-only">Unconfirmed<\/span>/);
    // A reported entry with a post also links to the check.
    expect(out).toContain('href="/en/p/intc-q3"');
    expect(out).toContain("No reports.");
  });

  it("names each day's region by its heading, once", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" />);
    expect(out).toContain('<section class="cal-day" aria-labelledby="cal-2026-11-02"><h2 class="cal-date" id="cal-2026-11-02">');
    expect(out).toContain('aria-labelledby="cal-2026-11-03"');
    expect(out).not.toContain("aria-label=");
  });

  it("uses Chinese labels on ZH pages", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="zh" />);
    expect(out).toContain("盘前");
    expect(out).toContain(">未确认<");
    expect(out).toContain("无财报。");
    expect(out).toContain("时间暂缺");
  });
});
