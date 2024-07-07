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
