use std::{convert::Infallible, fmt};

use super::Error;
use crate::{debug_error, error};

#[inline]
pub fn else_log<T, E>(error: E) -> Result<T, Infallible>
where
	T: Default,
	Error: From<E>,
{
	Ok(default_log(error))
}

#[inline]
pub fn else_debug_log<T, E>(error: E) -> Result<T, Infallible>
where
	T: Default,
	Error: From<E>,
{
	Ok(default_debug_log(error))
}

#[inline]
pub fn default_log<T, E>(error: E) -> T
where
	T: Default,
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_log(&error);
	T::default()
}

#[inline]
pub fn default_debug_log<T, E>(error: E) -> T
where
	T: Default,
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_debug_log(&error);
	T::default()
}

#[inline]
pub fn map_log<E>(error: E) -> Error
where
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_log(&error);
	error
}

#[inline]
pub fn map_debug_log<E>(error: E) -> Error
where
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_debug_log(&error);
	error
}

#[inline]
pub fn inspect_log<E: fmt::Display>(error: &E) {
	error!("{error}");
}

#[inline]
pub fn inspect_debug_log<E: fmt::Debug>(error: &E) {
	debug_error!("{error:?}");
}
