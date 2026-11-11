//! Earnings health checks: detect 8-K Item 2.02 filings on EDGAR for the
//! watchlist, extract quote-anchored figures from the press release, add GAAP
//! history from XBRL, chart it, and publish EN + ZH posts. Plus the earnings
//! calendar.

pub mod calendar;
pub mod charts;
pub mod detect;
pub mod draft;
pub mod extract;
pub mod facts;
pub mod foreign;
pub mod run;
pub mod share;
pub mod store;
pub mod watchlist;
pub mod xbrl;

pub use watchlist::{by_cik, cik10, find, in_window, watchlist, Filer, Tier, WatchEntry};
