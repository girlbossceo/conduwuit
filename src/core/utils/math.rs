use std::{cmp, time::Duration};

pub use checked_ops::checked_ops;

use crate::{Error, Result};

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
	($($input:tt)*) => {
		//#[allow(clippy::arithmetic_side_effects)] {
		//Some($($input)*)
		//	.ok_or_else(|| $crate::Error::Arithmetic("this error should never been seen"))
		//}

		//NOTE: remove me when stmt_expr_attributes is stable
		$crate::checked!($($input)*)
	}
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

#[inline]
#[allow(clippy::as_conversions)]
pub fn usize_from_f64(val: f64) -> Result<usize, Error> {
	if val < 0.0 {
		return Err(Error::Arithmetic("Converting negative float to unsigned integer"));
	}

	//SAFETY: <https://doc.rust-lang.org/std/primitive.f64.html#method.to_int_unchecked>
	Ok(unsafe { val.to_int_unchecked::<usize>() })
}

#[inline]
#[must_use]
pub fn usize_from_ruma(val: ruma::UInt) -> usize {
	usize::try_from(val).expect("failed conversion from ruma::UInt to usize")
}

#[inline]
#[must_use]
pub fn ruma_from_u64(val: u64) -> ruma::UInt {
	ruma::UInt::try_from(val).expect("failed conversion from u64 to ruma::UInt")
}

#[inline]
#[must_use]
pub fn ruma_from_usize(val: usize) -> ruma::UInt {
	ruma::UInt::try_from(val).expect("failed conversion from usize to ruma::UInt")
}

#[inline]
#[must_use]
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub fn usize_from_u64_truncated(val: u64) -> usize { val as usize }
