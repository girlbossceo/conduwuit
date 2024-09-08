use std::fmt;

use tracing::Level;

use super::{DebugInspect, Result};
use crate::error;

pub trait LogDebugErr<T, E>
where
	E: fmt::Debug,
{
	#[must_use]
	fn err_debug_log(self, level: Level) -> Self;

	#[inline]
	#[must_use]
	fn log_debug_err(self) -> Self
	where
		Self: Sized,
	{
		self.err_debug_log(Level::ERROR)
	}
}

impl<T, E> LogDebugErr<T, E> for Result<T, E>
where
	E: fmt::Debug,
{
	#[inline]
	fn err_debug_log(self, level: Level) -> Self
	where
		Self: Sized,
	{
		self.debug_inspect_err(|error| error::inspect_debug_log_level(&error, level))
	}
}
