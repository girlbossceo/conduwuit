use std::fmt;

use tracing::Level;

use super::Result;
use crate::error;

pub trait ErrLog<T, E>
where
	E: fmt::Display,
{
	fn log_err(self, level: Level) -> Self;

	fn err_log(self) -> Self
	where
		Self: Sized,
	{
		self.log_err(Level::ERROR)
	}
}

pub trait ErrDebugLog<T, E>
where
	E: fmt::Debug,
{
	fn log_err_debug(self, level: Level) -> Self;

	fn err_debug_log(self) -> Self
	where
		Self: Sized,
	{
		self.log_err_debug(Level::ERROR)
	}
}

impl<T, E> ErrLog<T, E> for Result<T, E>
where
	E: fmt::Display,
{
	#[inline]
	fn log_err(self, level: Level) -> Self
	where
		Self: Sized,
	{
		self.inspect_err(|error| error::inspect_log_level(&error, level))
	}
}

impl<T, E> ErrDebugLog<T, E> for Result<T, E>
where
	E: fmt::Debug,
{
	#[inline]
	fn log_err_debug(self, level: Level) -> Self
	where
		Self: Sized,
	{
		self.inspect_err(|error| error::inspect_debug_log_level(&error, level))
	}
}
