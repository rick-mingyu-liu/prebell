import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import CalendarWeek, { EAGER_LOGOS, ROWS_PER_SLOT } from "../src/components/CalendarWeek";
import PostList from "../src/components/PostList";
import type { CalendarDay, CalendarEntry, Slot } from "../src/lib/earnings";
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
    const [flat, image] = out.split('<li class="post-row">').slice(1);
    // Every row: small cover, then the kicker, the serif title link, the summary and the date.
    expect(flat).toContain('class="cover cover-small cover-flat"');
    expect(flat).toContain('<p class="kicker">After-close recap</p>');
    expect(flat).toMatch(/<h3 class="story-title"><a href="\/en\/p\/2026-10-29-recap">Stocks slip into the close<\/a><\/h3>/);
    expect(flat).toContain("Tech led the drop.");
    expect(flat).toContain('<time class="story-meta" dateTime="2026-10-29">Oct 29, 2026</time>');
    expect(flat.indexOf("cover-small")).toBeLessThan(flat.indexOf("kicker"));
    expect(flat.indexOf("kicker")).toBeLessThan(flat.indexOf("story-title"));
    expect(flat.indexOf("story-title")).toBeLessThan(flat.indexOf("story-meta"));
    // No image and no lines: the flat light cover with only its tag.
    expect(flat).not.toContain("cover-bg");
    expect(flat).not.toContain("cover-stat");
    expect(flat).toContain('<span class="cover-tag">Oct 29 · Recap</span>');
    // A post with an image and lines renders both, and the company logo as its tag. The lines stay in the
    // markup; the small-cover CSS hides them so only the tag and the chart show (see layout.test.tsx).
    expect(image).toContain('src="/covers/aapl-2026-10-29-earnings.png"');
    expect(image).toContain("Rev +8%");
    expect(image).toContain('alt="AAPL"');
    expect(image).toContain('<p class="kicker">Earnings check</p>');
    // The type is the kicker's job: no tag pill.
    expect(out).not.toContain('class="tag"');
    expect(out).not.toContain('class="row');
  });

  it("row titles use the heading level the page asks for", () => {
    expect(renderToStaticMarkup(<PostList locale="en" items={[post]} />)).toContain('<h3 class="story-title">');
    const h2 = renderToStaticMarkup(<PostList locale="en" items={[post]} headingLevel="h2" />);
    expect(h2).toContain('<h2 class="story-title">');
    expect(h2).not.toContain("<h3");
  });

  it("labels the kicker and the cover tag in Chinese", () => {
    const out = renderToStaticMarkup(<PostList locale="zh" items={[post]} />);
    expect(out).toContain('<p class="kicker">收盘回顾</p>');
    expect(out).toContain("10月29日 · 收盘");
  });
});

/** The <li> chunks of rendered markup, so an assertion cannot match across sibling entries. */
const items = (html: string) => html.split("<li ").slice(1).map((li) => `<li ${li.split("</li>")[0]}</li>`);

describe("CalendarWeek", () => {
  const SEC = "https://www.sec.gov/Archives/edgar/data/1090872/000109087226000041/0001090872-26-000041-index.htm";
  const entry = (ticker: string, slot: Slot, status: CalendarEntry["status"], more: Partial<CalendarEntry> = {}): CalendarEntry => ({
    ticker,
    name: null,
    writeUps: true,
    date: "2026-11-02",
    slot,
    status,
    postSlug: null,
    filingUrl: null,
    ...more,
  });
  const days: CalendarDay[] = [
    {
      date: "2026-11-02",
      entries: [
        entry("AMD", "amc", "confirmed", { name: "Advanced Micro Devices" }),
        entry("UNH", "bmo", "unconfirmed", { name: "UnitedHealth Group" }),
        entry("INTC", "bmo", "reported", { name: "Intel", postSlug: "intc-q3" }),
        entry("A", "bmo", "reported", { name: "Agilent Technologies", writeUps: false, filingUrl: SEC }),
        entry("UBER", "unknown", "unconfirmed"),
        entry("AVGO", "bmo", "confirmed", { name: "Broadcom" }),
      ],
    },
    { date: "2026-11-03", entries: [] },
  ];

  it("calendar week renders ruled day rows, a logo grid for write-up companies and text rows for the rest", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" />);
    expect(out).toContain('class="cal-day"');
    expect(out).not.toMatch(/badge/);
    // Each slot is a sub-heading with its count, in order: before open, after close, time not set.
    expect(out).toContain('<h3 class="cal-slot">Before open <span class="num">4</span></h3>');
    expect(out).toContain('<h3 class="cal-slot">After close <span class="num">1</span></h3>');
    expect(out).toContain('<h3 class="cal-slot">Time n/a <span class="num">1</span></h3>');
    // The tier split: write-up companies are tiles, in one grid ahead of the text rows; calendar-only ones are rows.
    const bmo = out.slice(out.indexOf("Before open"), out.indexOf("After close"));
    const [grid, rows] = bmo.split('<ul class="cal-list">');
    expect(grid.match(/class="cal-tile /g)).toHaveLength(3);
    expect(grid).toContain('href="/en/t/UNH"');
    expect(grid).toContain('href="/en/p/intc-q3"');
    expect(rows).toContain('<a class="cal-ticker" href="/en/t/A">A</a><span class="cal-name">Agilent Technologies</span>');
    expect(rows).not.toContain("cal-tile");
    expect(grid).not.toContain("cal-ticker");
    // A tile shows the logo with the company name as its alt text and the ticker beneath.
    expect(out).toMatch(/<img src="\/logos\/companies\/AMD\.svg" alt="Advanced Micro Devices" width="48" height="48" loading="(eager|lazy)"/);
    expect(out).toContain('<span class="cal-sym-t">AMD</span>');
    // No logo: the ticker as a text tile in the same grid, named for screen readers; no image.
    expect(out).toContain('<span class="cal-chip"><span class="sr-only">Broadcom</span>AVGO</span>');
    expect(out).not.toContain("logos/companies/AVGO");
    // A tile with no company name falls back to the ticker as its alt.
    expect(out).toContain('alt="UBER"');
    // The one coloured mark is the tick on a reported entry (tile or row); the status is also text for screen readers.
    expect(out.match(/num up/g)).toHaveLength(2);
    const li = items(out);
    const tileOf = (t: string) => li.find((x) => x.includes(`>${t}</span>`) && x.includes("cal-tile "))!;
    expect(tileOf("INTC")).toContain('<li class="cal-tile cal-reported">');
    expect(tileOf("INTC")).toContain('<span class="num up" aria-hidden="true">✓</span></span><span class="sr-only">Reported</span>');
    expect(li.find((x) => x.includes('href="/en/t/A"'))).toContain('<span class="num up" aria-hidden="true">✓</span><span class="sr-only">Reported</span>');
    // Unconfirmed entries are muted and say so in text.
    expect(tileOf("UNH")).toContain('<li class="cal-tile cal-unconfirmed">');
    expect(tileOf("UNH")).toContain('<span class="sr-only">Unconfirmed</span>');
    // A tile with a post says so for screen readers; a calendar-only reported row links to its SEC filing.
    expect(tileOf("INTC")).toContain('<span class="sr-only">Read the check</span>');
    expect(tileOf("UNH")).not.toContain("Read the check");
    expect(out).toContain(
      `<a class="cal-read" href="${SEC}" rel="noopener noreferrer" aria-label="SEC filing: A">SEC filing ↗</a>`,
    );
    expect(out).toContain("No reports.");
    expect(out).not.toContain("<details");
  });

  it("links a tile to the ticker page when there is no write-up", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: [entry("BRK.B", "bmo", "confirmed")] }]} locale="en" />);
    expect(out).toContain('href="/en/t/BRK.B"');
    expect(out).not.toContain("/p/");
  });

  it("shows only text rows for a slot of calendar-only companies", () => {
    const out = renderToStaticMarkup(
      <CalendarWeek days={[{ date: "2026-11-02", entries: [entry("A", "bmo", "confirmed", { writeUps: false })] }]} locale="en" />,
    );
    expect(out).toContain('class="cal-list"');
    expect(out).not.toContain("cal-tiles");
    expect(out).not.toContain("<img");
  });

  it("folds a long slot into a native details element", () => {
    const many = Array.from({ length: ROWS_PER_SLOT + 3 }, (_, i) =>
      entry(`T${String(i).padStart(2, "0")}`, "bmo", "unconfirmed", { writeUps: false }),
    );
    const out = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: many }]} locale="en" />);
    expect(out).toContain(`<h3 class="cal-slot">Before open <span class="num">${ROWS_PER_SLOT + 3}</span></h3>`);
    const [shown, folded] = out.split('<details class="cal-more">');
    expect(shown.match(/class="cal-ticker"/g)).toHaveLength(ROWS_PER_SLOT);
    expect(folded).toContain("<summary>Show 3 more</summary>");
    expect(folded.match(/class="cal-ticker"/g)).toHaveLength(3);
    const zh = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: many }]} locale="zh" />);
    expect(zh).toContain("<summary>展开其余3家</summary>");
  });

  it("folds only the text rows; the logo tiles all stay in view", () => {
    const tiles = Array.from({ length: ROWS_PER_SLOT + 5 }, (_, i) => entry(`W${String(i).padStart(2, "0")}`, "bmo", "unconfirmed"));
    const rows = [entry("X1", "bmo", "unconfirmed", { writeUps: false })];
    const out = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: [...tiles, ...rows] }]} locale="en" />);
    expect(out.match(/class="cal-tile /g)).toHaveLength(ROWS_PER_SLOT + 5);
    expect(out).toContain(`<span class="num">${ROWS_PER_SLOT + 6}</span>`);
    expect(out).not.toContain("<details");
  });

  it("loads the first logos on the page eagerly and the rest lazily", () => {
    const tickers = ["AAPL", "ABBV", "ABT", "ACN", "ADBE", "AMD", "AMGN", "AMZN", "BA", "BAC"];
    const out = renderToStaticMarkup(
      <CalendarWeek days={[{ date: "2026-11-02", entries: tickers.map((t) => entry(t, "bmo", "unconfirmed")) }]} locale="en" />,
    );
    const imgs = out.match(/<img [^>]*>/g)!;
    expect(imgs).toHaveLength(tickers.length);
    expect(imgs.filter((i) => i.includes('loading="eager"'))).toHaveLength(EAGER_LOGOS);
    expect(imgs.slice(0, EAGER_LOGOS).every((i) => i.includes('loading="eager"'))).toBe(true);
    expect(imgs.slice(EAGER_LOGOS).every((i) => i.includes('loading="lazy"'))).toBe(true);
  });

  it("names each day's region by its heading, once", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" />);
    expect(out).toContain('<section class="cal-day" aria-labelledby="cal-2026-11-02"><h2 class="cal-date" id="cal-2026-11-02">');
    expect(out).toContain('aria-labelledby="cal-2026-11-03"');
    expect(out).not.toMatch(/<section[^>]*aria-label=/);
  });

  it("uses Chinese labels on ZH pages", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="zh" />);
    expect(out).toContain("盘前");
    expect(out).toContain(">未确认<");
    expect(out).toContain('href="/zh/t/UNH"');
    expect(out).toContain('href="/zh/p/intc-q3"');
    expect(out).toContain(">已发布<");
    expect(out).toContain(">阅读速评<");
    expect(out).toContain("无财报。");
    expect(out).toContain("时间暂缺");
  });
});
