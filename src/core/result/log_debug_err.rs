use std::fmt::Debug;

use tracing::Level;

use super::{DebugInspect, Result};
use crate::error;

pub trait LogDebugErr<T, E: Debug> {
	#[must_use]
	fn err_debug_log(self, level: Level) -> Self;

	#[must_use]
	fn log_debug_err(self) -> Self
	where
		Self: Sized,
	{
		self.err_debug_log(Level::ERROR)
	}
}

impl<T, E: Debug> LogDebugErr<T, E> for Result<T, E> {
	#[inline]
	fn err_debug_log(self, level: Level) -> Self {
		self.debug_inspect_err(|error| error::inspect_debug_log_level(&error, level))
	}
}
