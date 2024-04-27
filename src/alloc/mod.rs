pub(crate) mod hardened;
pub(crate) mod je;

#[cfg(all(not(target_env = "msvc"), feature = "hardened_malloc", target_os = "linux", not(feature = "jemalloc")))]
pub(crate) fn memory_usage() -> String { hardened::memory_usage() }

#[cfg(all(not(target_env = "msvc"), feature = "jemalloc", not(feature = "hardened_malloc")))]
pub(crate) fn memory_usage() -> String { je::memory_usage() }

#[cfg(any(target_env = "msvc", all(not(feature = "jemalloc"), not(feature = "hardened_malloc"))))]
pub(crate) fn memory_usage() -> String { String::default() }

#[cfg(all(not(target_env = "msvc"), feature = "hardened_malloc", target_os = "linux", not(feature = "jemalloc")))]
pub(crate) fn memory_stats() -> String { hardened::memory_stats() }

#[cfg(all(not(target_env = "msvc"), feature = "jemalloc", not(feature = "hardened_malloc")))]
pub(crate) fn memory_stats() -> String { je::memory_stats() }

#[cfg(any(target_env = "msvc", all(not(feature = "jemalloc"), not(feature = "hardened_malloc"))))]
pub(crate) fn memory_stats() -> String { String::default() }
