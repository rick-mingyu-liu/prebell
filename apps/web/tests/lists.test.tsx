import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import CalendarWeek, { ROWS_PER_SLOT } from "../src/components/CalendarWeek";
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
      ],
    },
    { date: "2026-11-03", entries: [] },
  ];

  it("calendar week renders ruled day rows without badges", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" />);
    // Ruled day rows, no chips, badges or logos.
    expect(out).toContain('class="cal-day"');
    expect(out).not.toMatch(/badge|cal-chip|tlogo|<img/);
    // Each entry: a ticker link to the company page and the company name.
    expect(out).toContain('<a class="cal-ticker" href="/en/t/UNH">UNH</a><span class="cal-name">UnitedHealth Group</span>');
    expect(out).toContain('<a class="cal-ticker" href="/en/t/AMD">AMD</a>');
    expect(out).toContain('<a class="cal-ticker" href="/en/t/INTC">INTC</a>');
    // Each slot is a sub-heading with its count, in order: before open, after close, time not set.
    expect(out).toContain('<h3 class="cal-slot">Before open <span class="num">3</span></h3>');
    expect(out).toContain('<h3 class="cal-slot">After close <span class="num">1</span></h3>');
    expect(out).toContain('<h3 class="cal-slot">Time n/a <span class="num">1</span></h3>');
    expect(out.indexOf(">UNH<")).toBeLessThan(out.indexOf(">AMD<"));
    expect(out.indexOf(">AMD<")).toBeLessThan(out.indexOf(">UBER<"));
    // The one coloured mark is the tick on a reported entry; the status is also in text for screen readers.
    expect(out.match(/num up/g)).toHaveLength(2);
    expect(out).toMatch(/<li class="cal-entry cal-reported">.*?<span class="num up" aria-hidden="true">✓<\/span><span class="sr-only">Reported<\/span>/);
    // Unconfirmed entries are muted and say so in text.
    expect(out).toMatch(/<li class="cal-entry cal-unconfirmed">.*?<span class="sr-only">Unconfirmed<\/span>/);
    // A reported entry with a post links to the check; a calendar-only one to its SEC filing.
    expect(out).toMatch(/<a class="cal-read" aria-label="Read the check: INTC" href="\/en\/p\/intc-q3">Read the check →<\/a>/);
    expect(out).toContain(
      `<a class="cal-read" href="${SEC}" rel="noopener noreferrer" aria-label="SEC filing: A">SEC filing ↗</a>`,
    );
    expect(out).toContain("No reports.");
    expect(out).not.toContain("<details");
  });

  it("folds a long slot into a native details element", () => {
    const many = Array.from({ length: ROWS_PER_SLOT + 3 }, (_, i) => entry(`T${String(i).padStart(2, "0")}`, "bmo", "unconfirmed"));
    const out = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: many }]} locale="en" />);
    expect(out).toContain(`<h3 class="cal-slot">Before open <span class="num">${ROWS_PER_SLOT + 3}</span></h3>`);
    const [shown, folded] = out.split('<details class="cal-more">');
    expect(shown.match(/class="cal-ticker"/g)).toHaveLength(ROWS_PER_SLOT);
    expect(folded).toContain("<summary>Show 3 more</summary>");
    expect(folded.match(/class="cal-ticker"/g)).toHaveLength(3);
    const zh = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: many }]} locale="zh" />);
    expect(zh).toContain("<summary>展开其余3家</summary>");
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
    expect(out).toContain("无财报。");
    expect(out).toContain("时间暂缺");
  });
});
