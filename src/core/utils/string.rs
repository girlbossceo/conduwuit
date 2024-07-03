use crate::Result;

pub const EMPTY: &str = "";

#[inline]
#[must_use]
pub fn split_once_infallible<'a>(input: &'a str, delim: &'_ str) -> (&'a str, &'a str) {
	input.split_once(delim).unwrap_or((input, EMPTY))
}

/// Parses the bytes into a string.
#[inline]
pub fn string_from_bytes(bytes: &[u8]) -> Result<String> {
	let str: &str = str_from_bytes(bytes)?;
	Ok(str.to_owned())
}

/// Parses the bytes into a string.
#[inline]
pub fn str_from_bytes(bytes: &[u8]) -> Result<&str> { Ok(std::str::from_utf8(bytes)?) }
