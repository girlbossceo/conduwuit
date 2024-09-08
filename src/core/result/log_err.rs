use std::fmt;

use tracing::Level;

use super::Result;
use crate::error;

pub trait LogErr<T, E>
where
	E: fmt::Display,
{
	#[must_use]
	fn err_log(self, level: Level) -> Self;

	#[inline]
	#[must_use]
	fn log_err(self) -> Self
	where
		Self: Sized,
	{
		self.err_log(Level::ERROR)
	}
}

impl<T, E> LogErr<T, E> for Result<T, E>
where
	E: fmt::Display,
{
	#[inline]
	fn err_log(self, level: Level) -> Self
	where
		Self: Sized,
	{
		self.inspect_err(|error| error::inspect_log_level(&error, level))
	}
}
