mod broadband;
mod cloned;
mod expect;
mod ignore;
mod iter_stream;
mod ready;
mod tools;
mod try_broadband;
mod try_ready;
mod wideband;

pub use broadband::BroadbandExt;
pub use cloned::Cloned;
pub use expect::TryExpect;
pub use ignore::TryIgnore;
pub use iter_stream::IterStream;
pub use ready::ReadyExt;
pub use tools::Tools;
pub use try_broadband::TryBroadbandExt;
pub use try_ready::TryReadyExt;
pub use wideband::WidebandExt;

/// Stream concurrency factor; this is a live value.
static WIDTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(32);

/// Practicable limits on the stream width
pub const WIDTH_LIMIT: (usize, usize) = (1, 1024);

/// Sets the live concurrency factor. The first return value is the previous
/// width which was replaced. The second return value is the value which was set
/// after any applied limits.
pub fn set_width(width: usize) -> (usize, usize) {
	use std::sync::atomic::Ordering;

	let width = width.clamp(WIDTH_LIMIT.0, WIDTH_LIMIT.1);
	(WIDTH.swap(width, Ordering::Relaxed), width)
}

/// Used by stream operations where the concurrency factor hasn't been manually
/// supplied by the caller (most uses). Instead we provide a default value which
/// is adjusted at startup for the specific system and also dynamically.
#[inline]
pub fn automatic_width() -> usize {
	use std::sync::atomic::Ordering;

	let width = WIDTH.load(Ordering::Relaxed);
	debug_assert!(width >= WIDTH_LIMIT.0, "WIDTH should not be zero");
	debug_assert!(width <= WIDTH_LIMIT.1, "WIDTH is probably too large");
	width
}
