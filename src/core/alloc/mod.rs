//! Integration with allocators

// jemalloc
#[cfg(all(not(target_env = "msvc"), feature = "jemalloc", not(feature = "hardened_malloc")))]
pub mod je;
#[cfg(all(not(target_env = "msvc"), feature = "jemalloc", not(feature = "hardened_malloc")))]
pub use je::{memory_stats, memory_usage};

// hardened_malloc
#[cfg(all(not(target_env = "msvc"), feature = "hardened_malloc", target_os = "linux", not(feature = "jemalloc")))]
pub mod hardened;
#[cfg(all(not(target_env = "msvc"), feature = "hardened_malloc", target_os = "linux", not(feature = "jemalloc")))]
pub use hardened::{memory_stats, memory_usage};

// default, enabled when none or multiple of the above are enabled
#[cfg(any(
	not(any(feature = "jemalloc", feature = "hardened_malloc")),
	all(feature = "jemalloc", feature = "hardened_malloc"),
))]
pub mod default;
#[cfg(any(
	not(any(feature = "jemalloc", feature = "hardened_malloc")),
	all(feature = "jemalloc", feature = "hardened_malloc"),
))]
pub use default::{memory_stats, memory_usage};
