use std::{cmp, time::Duration};

pub use checked_ops::checked_ops;

/// Checked arithmetic expression. Returns a Result<R, Error::Arithmetic>
#[macro_export]
macro_rules! checked {
	($($input:tt)*) => {
		$crate::utils::math::checked_ops!($($input)*)
			.ok_or_else(|| $crate::Error::Arithmetic("operation overflowed or result invalid"))
	}
}

/// in release-mode. Use for performance when the expression is obviously safe.
/// The check remains in debug-mode for regression analysis.
#[cfg(not(debug_assertions))]
#[macro_export]
macro_rules! validated {
	($($input:tt)*) => { Ok($($input)*) }
}

#[cfg(debug_assertions)]
#[macro_export]
macro_rules! validated {
	($($input:tt)*) => { $crate::checked!($($input)*) }
}

/// Returns false if the exponential backoff has expired based on the inputs
#[inline]
#[must_use]
pub fn continue_exponential_backoff_secs(min: u64, max: u64, elapsed: Duration, tries: u32) -> bool {
	let min = Duration::from_secs(min);
	let max = Duration::from_secs(max);
	continue_exponential_backoff(min, max, elapsed, tries)
}

/// Returns false if the exponential backoff has expired based on the inputs
#[inline]
#[must_use]
pub fn continue_exponential_backoff(min: Duration, max: Duration, elapsed: Duration, tries: u32) -> bool {
	let min = min.saturating_mul(tries).saturating_mul(tries);
	let min = cmp::min(min, max);
	elapsed < min
}
