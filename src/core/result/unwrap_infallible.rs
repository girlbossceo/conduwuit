use std::convert::Infallible;

use super::{DebugInspect, Result};
use crate::error;

pub trait UnwrapInfallible<T> {
	fn unwrap_infallible(self) -> T;
}

impl<T> UnwrapInfallible<T> for Result<T, Infallible> {
	#[inline]
	fn unwrap_infallible(self) -> T {
		// SAFETY: Branchless unwrap for errors that can never happen. In debug
		// mode this is asserted.
		unsafe { self.debug_inspect_err(error::infallible).unwrap_unchecked() }
	}
}
