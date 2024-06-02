//! Default allocator with no special features

/// Always returns the empty string
#[must_use]
pub fn memory_stats() -> String { String::default() }

/// Always returns the empty string
#[must_use]
pub fn memory_usage() -> String { String::default() }
