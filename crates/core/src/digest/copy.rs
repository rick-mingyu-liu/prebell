//! Every word of the weekly digest email, so the wording can be redrafted
//! here without touching the layout in `render`. A `{name}` placeholder is
//! filled in by `render`; keep it when rewording. Labels the site already
//! shows use the site's own strings (`apps/web/messages/en.json`), so the
//! email and the calendar page read the same.
//!
//! One English email (Rick, 2026-11-10: "just english"). The wording below
//! is a first draft for Rick to rewrite. It claims nothing the site does not
//! do: the picks are earnings checks already on the site, the dates are the
//! calendar's, and nothing is compared with analyst estimates.

/// Subject line. `{range}`: "Nov 9–13".
pub const SUBJECT: &str = "Week ahead: {range}";

/// The first line, when last week has earnings checks to pick from.
/// `{picks}`: one of `PICK_COUNTS`.
pub const OPENING: &str =
    "Hi, it's Rick from PreBell. First, {picks} from last week; then the dates to know for the week of {range}.";
/// `{picks}` for one, two and three picks (`super::PICKS`).
pub const PICK_COUNTS: [&str; super::PICKS] = [
    "one earnings check",
    "two earnings checks",
    "three earnings checks",
];
/// The first line when last week has no earnings check at all.
pub const OPENING_NONE: &str =
    "Hi, it's Rick from PreBell. Here are the dates to know for the week of {range}.";

/// The picks' heading.
pub const PICKS_HEADING: &str = "From last week's earnings checks";
/// Under the picks (site: `home.allEarnings`).
pub const ALL_CHECKS: &str = "All earnings checks →";

/// The week's section heading (site: `earnings.calendarTitle`).
pub const CALENDAR_HEADING: &str = "Market calendar";
/// Under the heading.
pub const TIMES_NOTE: &str = "Times are US Eastern.";

/// Report slots (site: `earnings.bmo`, `earnings.amc`, `earnings.timeUnknown`).
pub const BMO: &str = "Before open";
pub const AMC: &str = "After close";
pub const TIME_UNKNOWN: &str = "Time n/a";

/// After a company whose date is an aggregator's (site: `earnings.unconfirmed`).
pub const UNCONFIRMED: &str = " (unconfirmed)";

/// After the listed write-up companies: the slot's other companies (site:
/// `earnings.stripMore`).
pub const MORE: &str = "+{count} more";
/// A slot with no write-up company: only the count.
pub const COMPANIES: &str = "{count} companies";
pub const ONE_COMPANY: &str = "1 company";

/// A trading day with no earnings on the calendar (site: `earnings.none`).
pub const NO_EARNINGS: &str = "No earnings reports.";

/// After an event (site: `events.postponed`, `events.movedFrom`).
pub const POSTPONED: &str = " (postponed)";
pub const MOVED_FROM: &str = " (moved from {date})";

/// Under the week (site: `earnings.stripAll`).
pub const FULL_CALENDAR: &str = "Full calendar →";

/// The one question, the last line before the sign-off. Replies go to the
/// newsletter's reply-to address (Buttondown: your account email).
pub const QUESTION: &str =
    "One question: what would make this email more useful to you? Just hit reply.";

/// The sign-off.
pub const SIGN_OFF: &str = "Rick";

/// Last, under the sign-off.
pub const FOOTER: &str =
    "PreBell is generated automatically from public data and SEC filings, for information only. It is not investment advice.";

/// Day headings: "Mon, Nov 9" (the site's `formatDayShort`).
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
