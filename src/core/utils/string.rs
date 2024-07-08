use crate::Result;

pub const EMPTY: &str = "";

/// Find the common prefix from a collection of strings and return a slice
/// ```
/// use conduit_core::utils::string::common_prefix;
/// let input = ["conduwuit", "conduit", "construct"];
/// common_prefix(&input) == "con";
/// ```
#[must_use]
#[allow(clippy::string_slice)]
pub fn common_prefix<'a>(choice: &'a [&str]) -> &'a str {
	choice.first().map_or(EMPTY, move |best| {
		choice.iter().skip(1).fold(*best, |best, choice| {
			&best[0..choice
				.char_indices()
				.zip(best.char_indices())
				.take_while(|&(a, b)| a == b)
				.count()]
		})
	})
}

#[inline]
#[must_use]
pub fn split_once_infallible<'a>(input: &'a str, delim: &'_ str) -> (&'a str, &'a str) {
	input.split_once(delim).unwrap_or((input, EMPTY))
}

/// Parses the bytes into a string.
pub fn string_from_bytes(bytes: &[u8]) -> Result<String> {
	let str: &str = str_from_bytes(bytes)?;
	Ok(str.to_owned())
}

/// Parses the bytes into a string.
#[inline]
pub fn str_from_bytes(bytes: &[u8]) -> Result<&str> { Ok(std::str::from_utf8(bytes)?) }
