//! Competitor watch (internal only): a daily read of AlphaLab's public RSS
//! feed, compared against what we published. Nothing here may feed article
//! generation; a test below enforces that.

pub mod feed;
