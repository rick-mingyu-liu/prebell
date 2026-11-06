import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import CalendarWeek, { CARDS_PER_SLOT, EAGER_PER_SLOT } from "../src/components/CalendarWeek";
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
})/** The <li> chunks of rendered markup, so an assertion cannot match across sibling entries. */
const items = (html: string) => html.split("<li ").slice(1).map((li) => `<li ${li.split("</li>")[0]}</li>`);

/** One day's column, from its opening tag to the next column's. */
const column = (html: string, date: string) => {
  const from = html.indexOf(`<section id="d-${date}"`);
  const to = html.indexOf("<section ", from + 1);
  return html.slice(from, to === -1 ? undefined : to);
};

/** The markup of one slot list inside a column. */
const half = (html: string, slot: "bmo" | "amc" | "unknown") => {
  const from = html.indexOf(`<div class="cal-half cal-half-${slot}">`);
  const next = html.indexOf('<div class="cal-half ', from + 1);
  return from === -1 ? "" : html.slice(from, next === -1 ? undefined : next);
};

/** The hrefs of the cards in a chunk, in page order, as tickers or post slugs. */
const order = (html: string) => [...html.matchAll(/class="cal-card-link"[^>]*href="\/\w+\/(?:t\/([^"]+)|p\/([^"]+))"/g)].map((m) => m[1] ?? m[2]);

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
  const week = ["2026-11-02", "2026-11-03", "2026-11-04", "2026-11-05", "2026-11-06"];
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
    ...week.slice(1).map((date) => ({ date, entries: [] })),
  ];

  it("lays the week out as five labelled day columns with the weekday and date, Monday first", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-04" />);
    expect(out.match(/<section /g)).toHaveLength(5);
    expect(out).toContain('<div class="cal-board">');
    for (const d of week) expect(out).toContain(`aria-labelledby="cal-${d}"`);
    expect(out).not.toMatch(/<section[^>]*aria-label=/);
    expect(out).toContain('<h2 class="cal-date" id="cal-2026-11-02"><time dateTime="2026-11-02">Mon · 11/2</time>');
    expect(out).toContain('<time dateTime="2026-11-06">Fri · 11/6</time>');
    expect(out.indexOf("d-2026-11-02")).toBeLessThan(out.indexOf("d-2026-11-06"));
  });

  it("highlights today's column in words, marks earlier days as past and later ones as ahead", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-04" />);
    expect(out).toContain('<section id="d-2026-11-04" class="cal-col is-today" aria-labelledby="cal-2026-11-04">');
    expect(column(out, "2026-11-04")).toContain('<span class="cal-today">Today</span></h2>');
    expect(out.match(/cal-today"/g)).toHaveLength(1);
    expect(out).toContain('<section id="d-2026-11-02" class="cal-col is-past"');
    expect(column(out, "2026-11-03")).toContain('<span class="sr-only">Past</span></h2>');
    expect(out).toContain('<section id="d-2026-11-05" class="cal-col is-ahead"');
    expect(column(out, "2026-11-05")).not.toContain("Past");
    // A week that is not this one has no today and nothing for the start-at-today script to do.
    const other = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-20" />);
    expect(other).not.toContain("is-today");
    expect(other.match(/is-past/g)).toHaveLength(5);
  });

  it("keys the marks in one line above the chips and the board, the words carrying the meaning", () => {
    const key = (html: string) => html.slice(html.indexOf('<p class="cal-key">'), html.indexOf("</p>", html.indexOf('<p class="cal-key">')) + 4);
    const words = (html: string) => key(html).replace(/<[^>]+>/g, "");
    const en = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-04" />);
    expect(en.indexOf('<p class="cal-key">')).toBeLessThan(en.indexOf("cal-chips"));
    expect(en.indexOf("cal-chips")).toBeLessThan(en.indexOf("cal-board"));
    expect(words(en)).toBe("Before open·After close·✓Reported·dashed card: date not confirmed");
    // The sun and moon are the sub-headings' icons; the tick is the cards' tick; the swatch is a dashed card.
    expect(key(en)).toMatch(/<span class="cal-key-item"><svg class="cal-slot-icon"[^>]*aria-hidden="true"[^>]*>.*?<\/svg>Before open<\/span>/);
    expect(key(en)).toContain('<span class="cal-key-tick" aria-hidden="true">✓</span>Reported</span>');
    expect(key(en)).toContain('<span class="cal-key-dash" aria-hidden="true"></span>dashed card: date not confirmed</span>');
    expect(key(en).match(/<span class="cal-key-sep" aria-hidden="true">·<\/span>/g)).toHaveLength(3);
    const zh = renderToStaticMarkup(<CalendarWeek days={days} locale="zh" today="2026-11-04" />);
    expect(words(zh)).toBe("盘前·盘后·✓已发布·虚线框：日期未确认");
    expect(key(zh)).toContain('<span class="cal-key-dash" aria-hidden="true"></span>虚线框：日期未确认</span>');
  });

  it("puts day chips above the board, linking to each column and marking today", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-04" />);
    expect(out.indexOf('<nav class="cal-chips" aria-label="Days of the week">')).toBeLessThan(out.indexOf("cal-board"));
    expect(out).toContain('<a class="cal-chip-day" href="#d-2026-11-02"><span>Mon</span> <span class="num">2</span></a>');
    expect(out).toContain('<a class="cal-chip-day" href="#d-2026-11-04" aria-current="date"><span>Wed</span> <span class="num">4</span></a>');
    expect(out.match(/aria-current/g)).toHaveLength(1);
  });

  it("splits a day into before open and after close side by side, time n/a under them only when it has reports", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-02" />);
    const mon = column(out, "2026-11-02");
    const halves = mon.slice(mon.indexOf('<div class="cal-halves">'));
    expect(halves.indexOf("cal-half-bmo")).toBeLessThan(halves.indexOf("cal-half-amc"));
    expect(halves.indexOf("cal-half-amc")).toBeLessThan(halves.indexOf("cal-half-unknown"));
    expect(mon).toMatch(/<h3 class="cal-slot"><svg [^>]*aria-hidden="true"[^>]*>.*?<\/svg>Before open <span class="num">4<\/span><\/h3>/);
    expect(mon).toMatch(/<h3 class="cal-slot"><svg .*?<\/svg>After close <span class="num">1<\/span><\/h3>/);
    expect(mon).toMatch(/<h3 class="cal-slot"><svg .*?<\/svg>Time n\/a <span class="num">1<\/span><\/h3>/);
    expect(half(mon, "bmo").match(/<li class="cal-card /g)).toHaveLength(4);
    expect(half(mon, "amc").match(/<li class="cal-card /g)).toHaveLength(1);
    expect(order(half(mon, "unknown"))).toEqual(["UBER"]);
    // Time n/a is left out on a day where every report has a time; an empty timed list says so.
    const timed = renderToStaticMarkup(
      <CalendarWeek days={[{ date: "2026-11-02", entries: [entry("AMD", "amc", "confirmed")] }]} locale="en" today="2026-11-02" />,
    );
    expect(timed).not.toContain("cal-half-unknown");
    expect(half(timed, "bmo")).toContain('<p class="cal-empty"><span aria-hidden="true">—</span><span class="sr-only">None</span></p>');
    // A day with nothing says so in place of the lists.
    expect(column(out, "2026-11-03")).toContain('<p class="muted cal-none">No earnings reports.</p>');
    expect(column(out, "2026-11-03")).not.toContain("cal-halves");
  });

  it("leads each list with the write-up companies and links a card to its write-up, else the ticker page", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-02" />);
    expect(order(half(out, "bmo"))).toEqual(["UNH", "intc-q3", "AVGO", "A"]);
    expect(out).toContain('href="/en/p/intc-q3"');
    expect(out).toContain('href="/en/t/A"');
    const brk = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: [entry("BRK.B", "bmo", "confirmed")] }]} locale="en" />);
    expect(brk).toContain('href="/en/t/BRK.B"');
    expect(brk).not.toContain("/p/");
  });

  it("sets every company as a card: a square logo beside the ticker and name, or the ticker and name in type", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-02" />);
    const li = items(out);
    const card = (t: string) => li.find((x) => x.includes(`>${t}</span>`) && x.includes("cal-card "))!;
    // Today's manifest is square tiles: the icon is decoration beside the name, which names the link.
    expect(card("AMD")).toContain('<li class="cal-card cal-card-icon cal-confirmed">');
    expect(card("AMD")).toMatch(/<img class="cal-logo-icon" src="\/logos\/companies\/AMD\.svg" alt="" width="48" height="48" loading="(eager|lazy)"/);
    expect(card("AMD")).toContain('<span class="cal-card-sym" aria-hidden="true">AMD</span><span class="cal-card-name">Advanced Micro Devices</span>');
    expect(card("AMD")).toContain('title="Advanced Micro Devices"');
    // No logo: the ticker and name set in type, the name read out, no image.
    expect(card("AVGO")).toContain('<li class="cal-card cal-card-text cal-confirmed">');
    expect(card("AVGO")).toContain('<span class="cal-card-sym" aria-hidden="true">AVGO</span><span class="cal-card-name">Broadcom</span>');
    expect(card("AVGO")).not.toContain("<img");
    // No name: the ticker is the name.
    expect(card("UBER")).toContain('<span class="cal-card-sym">UBER</span></span>');
    expect(card("UBER")).not.toContain("cal-card-name");
    expect(out).not.toMatch(/badge|cal-tile/);
  });

  it("writes the status in text: a corner tick for reported, muted unconfirmed, and the write-up", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" today="2026-11-02" />);
    const li = items(out);
    const card = (t: string) => li.find((x) => x.includes(`>${t}</span>`) && x.includes("cal-card "))!;
    expect(out.match(/cal-card-tick/g)).toHaveLength(2);
    expect(card("INTC")).toContain('cal-reported">');
    expect(card("INTC")).toContain('<span class="cal-card-tick" aria-hidden="true">✓</span><span class="sr-only">Reported</span><span class="sr-only">Read the check</span>');
    expect(li.find((x) => x.includes('href="/en/t/A"'))).toContain('<span class="sr-only">Reported</span>');
    expect(card("UNH")).toContain('cal-unconfirmed">');
    expect(card("UNH")).toContain('<span class="sr-only">Unconfirmed</span>');
    expect(card("UNH")).not.toContain("Read the check");
    expect(card("AVGO")).not.toMatch(/sr-only/);
  });

  it("folds a long list into a native details element, keeping the write-up companies in view", () => {
    const rows = Array.from({ length: CARDS_PER_SLOT + 1 }, (_, i) => entry(`X${String(i).padStart(2, "0")}`, "bmo", "unconfirmed", { writeUps: false }));
    const ups = [entry("W1", "bmo", "unconfirmed"), entry("W2", "bmo", "unconfirmed")];
    const amc = [entry("Y1", "amc", "confirmed")];
    const out = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: [...rows, ...ups, ...amc] }]} locale="en" />);
    const bmo = half(out, "bmo");
    expect(bmo).toContain(`Before open <span class="num">${CARDS_PER_SLOT + 3}</span>`);
    const [shown, folded] = bmo.split('<details class="cal-more">');
    expect(shown.match(/<li class="cal-card /g)).toHaveLength(CARDS_PER_SLOT);
    expect(order(shown).slice(0, 2)).toEqual(["W1", "W2"]);
    expect(folded).toContain("<summary>Show 3 more</summary>");
    expect(folded.match(/<li class="cal-card /g)).toHaveLength(3);
    expect(order(folded).at(-1)).toBe(`X${String(CARDS_PER_SLOT).padStart(2, "0")}`);
    // The other list folds on its own.
    expect(half(out, "amc")).not.toContain("<details");
    const zh = renderToStaticMarkup(<CalendarWeek days={[{ date: "2026-11-02", entries: [...rows, ...ups] }]} locale="zh" />);
    expect(zh).toContain("<summary>展开其余3家</summary>");
  });

  it("loads the top logos of the day the board opens on eagerly and the rest lazily", () => {
    const tickers = ["AAPL", "ABBV", "ABT", "ACN", "AMD", "BA"];
    const week: CalendarDay[] = [
      { date: "2026-11-02", entries: [...tickers.map((t) => entry(t, "bmo", "confirmed")), entry("CAT", "amc", "confirmed")] },
      { date: "2026-11-03", entries: [entry("AVGO", "bmo", "confirmed"), entry("F", "bmo", "confirmed"), entry("GE", "amc", "confirmed")] },
    ];
    const eagerOf = (html: string) =>
      html
        .match(/<img [^>]*>/g)!
        .filter((i) => i.includes('loading="eager"'))
        .map((i) => i.match(/companies\/(\w+)\./)![1]);
    // Today is Tuesday: its before-open top (AVGO has no logo, so F) and after-close top.
    const tue = renderToStaticMarkup(<CalendarWeek days={week} locale="en" today="2026-11-03" />);
    expect(tue.match(/<img [^>]*>/g)).toHaveLength(tickers.length + 3);
    expect(eagerOf(tue)).toEqual(["F", "GE"]);
    expect(eagerOf(tue)).toHaveLength(2 * EAGER_PER_SLOT);
    // No today in the week: the board opens on Monday.
    expect(eagerOf(renderToStaticMarkup(<CalendarWeek days={week} locale="en" today="2026-11-20" />))).toEqual(["AAPL", "CAT"]);
  });

  it("uses Chinese labels on ZH pages", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="zh" today="2026-11-03" />);
    expect(out).toContain('<time dateTime="2026-11-02">周一 · 11/2</time>');
    expect(out).toContain('<span class="cal-today">今天</span>');
    expect(out).toContain('<span class="sr-only">已过</span>');
    expect(out).toContain('aria-label="本周各日"');
    expect(out).toContain("<span>周三</span>");
    expect(out).toMatch(/<\/svg>盘前 <span class="num">4<\/span>/);
    expect(out).toMatch(/<\/svg>盘后 <span class="num">1<\/span>/);
    expect(out).toMatch(/<\/svg>时间暂缺 <span class="num">1<\/span>/);
    expect(out).toContain(">未确认<");
    expect(out).toContain('href="/zh/t/UNH"');
    expect(out).toContain('href="/zh/p/intc-q3"');
    expect(out).toContain(">已发布<");
    expect(out).toContain(">阅读速评<");
    expect(out).toContain("无财报。");
  });
});
