//! Default allocator with no special features

/// Always returns the empty string
pub(crate) fn memory_stats() -> String { Default::default() }

/// Always returns the empty string
pub(crate) fn memory_usage() -> String { Default::default() }
