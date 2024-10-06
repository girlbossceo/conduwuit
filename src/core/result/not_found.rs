use super::Result;
use crate::{error, Error};

pub trait NotFound<T> {
	#[must_use]
	fn is_not_found(&self) -> bool;
}

impl<T> NotFound<T> for Result<T, Error> {
	#[inline]
	fn is_not_found(&self) -> bool { self.as_ref().is_err_and(error::is_not_found) }
}
