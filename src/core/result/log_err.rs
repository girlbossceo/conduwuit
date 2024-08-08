use std::fmt::Display;

use tracing::Level;

use super::Result;
use crate::error;

pub trait LogErr<T, E: Display> {
	#[must_use]
	fn err_log(self, level: Level) -> Self;

	#[must_use]
	fn log_err(self) -> Self
	where
		Self: Sized,
	{
		self.err_log(Level::ERROR)
	}
}

impl<T, E: Display> LogErr<T, E> for Result<T, E> {
	#[inline]
	fn err_log(self, level: Level) -> Self { self.inspect_err(|error| error::inspect_log_level(&error, level)) }
}
