//! Every word of the weekly digest email, English and Chinese side by side
//! as `(EN, ZH)` pairs, so the wording can be redrafted here without
//! touching the layout in `render`. A `{name}` placeholder is filled in by
//! `render`; keep it when rewording. Labels the site already shows use the
//! site's own strings (`apps/web/messages/*.json`), so the email and the
//! calendar page read the same.
//!
//! v0 (2026-11-06): plain and functional; the owner drafts the final wording.

/// `(EN, ZH)`.
pub type Pair = (&'static str, &'static str);

/// Subject line. `{range}`: "Nov 9–13" / "11月9日–13日".
pub const SUBJECT: Pair = ("Week ahead: {range}", "一周前瞻：{range}");

/// The body's first line.
pub const INTRO: Pair = (
    "Earnings and economic dates for the week of {range}. Times are US Eastern.",
    "{range}当周的财报与经济日程。时间均为美东时间。",
);

/// The week's section heading (site: `earnings.calendarTitle`).
pub const CALENDAR_HEADING: Pair = ("Market calendar", "市场日历");

/// Report slots (site: `earnings.bmo`, `earnings.amc`, `earnings.timeUnknown`).
pub const BMO: Pair = ("Before open", "盘前");
pub const AMC: Pair = ("After close", "盘后");
pub const TIME_UNKNOWN: Pair = ("Time n/a", "时间暂缺");

/// After a company whose date is an aggregator's (site: `earnings.unconfirmed`).
pub const UNCONFIRMED: Pair = (" (unconfirmed)", "（未确认）");

/// After the listed write-up companies: the slot's other companies (site:
/// `earnings.stripMore`).
pub const MORE: Pair = ("+{count} more", "另有{count}家");
/// A slot with no write-up company: only the count.
pub const COMPANIES: Pair = ("{count} companies", "{count}家公司");
pub const ONE_COMPANY: Pair = ("1 company", "1家公司");

/// A trading day with no earnings on the calendar (site: `earnings.none`).
pub const NO_EARNINGS: Pair = ("No earnings reports.", "无财报。");

/// After an event (site: `events.postponed`, `events.movedFrom`).
pub const POSTPONED: Pair = (" (postponed)", "（已推迟）");
pub const MOVED_FROM: Pair = (" (moved from {date})", "（由{date}改期）");

/// Under the week (site: `earnings.stripAll`).
pub const FULL_CALENDAR: Pair = ("Full calendar →", "完整日历 →");

/// Last week's earnings checks (site: `home.earningsChecks`, `home.allEarnings`).
pub const CHECKS_HEADING: Pair = ("Last week's earnings checks", "上周财报速评");
pub const ALL_CHECKS: Pair = ("All earnings checks →", "全部财报速评 →");

/// Day headings: "Mon, Nov 9" / "11月9日 周一" (the site's `formatDayShort`).
pub const WEEKDAYS: [Pair; 7] = [
    ("Mon", "周一"),
    ("Tue", "周二"),
    ("Wed", "周三"),
    ("Thu", "周四"),
    ("Fri", "周五"),
    ("Sat", "周六"),
    ("Sun", "周日"),
];
