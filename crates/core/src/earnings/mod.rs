//! Earnings health checks: detect 8-K Item 2.02 filings on EDGAR for the
//! watchlist, extract quote-anchored figures from the press release, add GAAP
//! history from XBRL, chart it, and publish EN + ZH posts. Plus the earnings
//! calendar.

pub mod watchlist;

pub use watchlist::{by_cik, cik10, find, in_window, poll_batch, watchlist, Filer, WatchEntry};
