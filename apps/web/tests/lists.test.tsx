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
};

describe("PostList", () => {
  it("renders a row with date, title link, summary and the type tag", () => {
    const out = renderToStaticMarkup(<PostList locale="en" items={[post]} />);
    expect(out).toContain('class="row post-row"');
    expect(out).toContain('href="/en/p/2026-10-29-recap"');
    expect(out).toContain("Stocks slip into the close");
    expect(out).toContain("Tech led the drop.");
    expect(out).toMatch(/class="tag">After-close recap</);
  });

  it("labels the type in Chinese", () => {
    expect(renderToStaticMarkup(<PostList locale="zh" items={[post]} />)).toContain(">收盘回顾<");
  });
});

describe("CalendarWeek", () => {
  const days: CalendarDay[] = [
    {
      date: "2026-11-02",
      entries: [
        { ticker: "AMD", date: "2026-11-02", slot: "amc", status: "confirmed", postSlug: null },
        { ticker: "PFE", date: "2026-11-02", slot: "bmo", status: "unconfirmed", postSlug: null },
        { ticker: "INTC", date: "2026-11-02", slot: "bmo", status: "reported", postSlug: "intc-q3" },
      ],
    },
    { date: "2026-11-03", entries: [] },
  ];

  it("groups chips by slot, writes the status on each chip and links reported ones to the post", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="en" />);
    expect(out.indexOf("Before open")).toBeLessThan(out.indexOf("After close"));
    expect(out).toContain('class="cal-chip cal-unconfirmed" href="/en/t/PFE"');
    expect(out).toContain('class="cal-chip cal-reported" href="/en/p/intc-q3"');
    expect(out).toContain(">Unconfirmed<");
    expect(out).toContain(">Reported<");
    expect(out).toContain("No reports.");
  });

  it("uses Chinese labels on ZH pages", () => {
    const out = renderToStaticMarkup(<CalendarWeek days={days} locale="zh" />);
    expect(out).toContain("盘前");
    expect(out).toContain(">未确认<");
    expect(out).toContain("无财报。");
  });
});
