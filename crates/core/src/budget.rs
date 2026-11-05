//! Subrequest budget for optional work.
//!
//! The Cloudflare Free plan allows 50 fetches per invocation. Mandatory work
//! (quotes, drafts, the 401-refresh path) always runs; optional work, such as
//! an extra news search, runs only while it fits under `BUDGET_SOFT`.

/// Fetches the facts stage may reach with optional work before it stops
/// adding any. Leaves room for 2 to 4 draft calls and the +3 refresh path.
pub const BUDGET_SOFT: usize = 40;

/// Whether `cost` more fetches fit after `used` without passing `BUDGET_SOFT`.
pub fn allows(used: usize, cost: usize) -> bool {
    used + cost <= BUDGET_SOFT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_soft_budget() {
        assert!(allows(40, 0));
        assert!(allows(38, 2));
        assert!(!allows(39, 2));
    }
}
