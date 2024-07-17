//! Default allocator with no special features

/// Always returns None
#[must_use]
pub fn memory_stats() -> Option<String> { None }

/// Always returns None
#[must_use]
pub fn memory_usage() -> Option<String> { None }
