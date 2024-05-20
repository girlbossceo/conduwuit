use std::{
	cmp,
	cmp::Ordering,
	fmt,
	str::FromStr,
	time::{SystemTime, UNIX_EPOCH},
};

use rand::prelude::*;
use ring::digest;
use ruma::{canonical_json::try_from_json_map, CanonicalJsonError, CanonicalJsonObject, OwnedUserId};
use tracing::debug;

use crate::{Error, Result};

pub mod content_disposition;
pub mod defer;

pub fn clamp<T: Ord>(val: T, min: T, max: T) -> T { cmp::min(cmp::max(val, min), max) }

#[must_use]
#[allow(clippy::as_conversions)]
pub fn millis_since_unix_epoch() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.expect("time is valid")
		.as_millis() as u64
}

pub fn increment(old: Option<&[u8]>) -> Vec<u8> {
	let number = match old.map(TryInto::try_into) {
		Some(Ok(bytes)) => {
			let number = u64::from_be_bytes(bytes);
			number + 1
		},
		_ => 1, // Start at one. since 0 should return the first event in the db
	};

	number.to_be_bytes().to_vec()
}

#[must_use]
pub fn generate_keypair() -> Vec<u8> {
	let mut value = random_string(8).as_bytes().to_vec();
	value.push(0xFF);
	value.extend_from_slice(
		&ruma::signatures::Ed25519KeyPair::generate().expect("Ed25519KeyPair generation always works (?)"),
	);
	value
}

/// Parses the bytes into an u64.
pub fn u64_from_bytes(bytes: &[u8]) -> Result<u64, std::array::TryFromSliceError> {
	let array: [u8; 8] = bytes.try_into()?;
	Ok(u64::from_be_bytes(array))
}

/// Parses the bytes into a string.
pub fn string_from_bytes(bytes: &[u8]) -> Result<String, std::string::FromUtf8Error> {
	String::from_utf8(bytes.to_vec())
}

/// Parses a `OwnedUserId` from bytes.
pub fn user_id_from_bytes(bytes: &[u8]) -> Result<OwnedUserId> {
	OwnedUserId::try_from(
		string_from_bytes(bytes).map_err(|_| Error::bad_database("Failed to parse string from bytes"))?,
	)
	.map_err(|_| Error::bad_database("Failed to parse user id from bytes"))
}

pub fn random_string(length: usize) -> String {
	thread_rng()
		.sample_iter(&rand::distributions::Alphanumeric)
		.take(length)
		.map(char::from)
		.collect()
}

#[tracing::instrument(skip(keys))]
pub fn calculate_hash(keys: &[&[u8]]) -> Vec<u8> {
	// We only hash the pdu's event ids, not the whole pdu
	let bytes = keys.join(&0xFF);
	let hash = digest::digest(&digest::SHA256, &bytes);
	hash.as_ref().to_owned()
}

#[allow(clippy::impl_trait_in_params)]
pub fn common_elements(
	mut iterators: impl Iterator<Item = impl Iterator<Item = Vec<u8>>>, check_order: impl Fn(&[u8], &[u8]) -> Ordering,
) -> Option<impl Iterator<Item = Vec<u8>>> {
	let first_iterator = iterators.next()?;
	let mut other_iterators = iterators.map(Iterator::peekable).collect::<Vec<_>>();

	Some(first_iterator.filter(move |target| {
		other_iterators.iter_mut().all(|it| {
			while let Some(element) = it.peek() {
				match check_order(element, target) {
					Ordering::Greater => return false, // We went too far
					Ordering::Equal => return true,    // Element is in both iters
					Ordering::Less => {
						// Keep searching
						it.next();
					},
				}
			}
			false
		})
	}))
}

/// Fallible conversion from any value that implements `Serialize` to a
/// `CanonicalJsonObject`.
///
/// `value` must serialize to an `serde_json::Value::Object`.
pub fn to_canonical_object<T: serde::Serialize>(value: T) -> Result<CanonicalJsonObject, CanonicalJsonError> {
	use serde::ser::Error;

	match serde_json::to_value(value).map_err(CanonicalJsonError::SerDe)? {
		serde_json::Value::Object(map) => try_from_json_map(map),
		_ => Err(CanonicalJsonError::SerDe(serde_json::Error::custom("Value must be an object"))),
	}
}

pub fn deserialize_from_str<'de, D: serde::de::Deserializer<'de>, T: FromStr<Err = E>, E: fmt::Display>(
	deserializer: D,
) -> Result<T, D::Error> {
	struct Visitor<T: FromStr<Err = E>, E>(std::marker::PhantomData<T>);
	impl<T: FromStr<Err = Err>, Err: fmt::Display> serde::de::Visitor<'_> for Visitor<T, Err> {
		type Value = T;

		fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
			write!(formatter, "a parsable string")
		}

		fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
		where
			E: serde::de::Error,
		{
			v.parse().map_err(serde::de::Error::custom)
		}
	}
	deserializer.deserialize_str(Visitor(std::marker::PhantomData))
}

// Copied from librustdoc:
// https://github.com/rust-lang/rust/blob/cbaeec14f90b59a91a6b0f17fc046c66fa811892/src/librustdoc/html/escape.rs

/// Wrapper struct which will emit the HTML-escaped version of the contained
/// string when passed to a format string.
pub struct HtmlEscape<'a>(pub &'a str);

impl fmt::Display for HtmlEscape<'_> {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Because the internet is always right, turns out there's not that many
		// characters to escape: http://stackoverflow.com/questions/7381974
		let HtmlEscape(s) = *self;
		let pile_o_bits = s;
		let mut last = 0;
		for (i, ch) in s.char_indices() {
			let s = match ch {
				'>' => "&gt;",
				'<' => "&lt;",
				'&' => "&amp;",
				'\'' => "&#39;",
				'"' => "&quot;",
				_ => continue,
			};
			fmt.write_str(&pile_o_bits[last..i])?;
			fmt.write_str(s)?;
			// NOTE: we only expect single byte characters here - which is fine as long as
			// we only match single byte characters
			last = i + 1;
		}

		if last < s.len() {
			fmt.write_str(&pile_o_bits[last..])?;
		}
		Ok(())
	}
}

/// one true function for returning the conduwuit version with the necessary
/// CONDUWUIT_VERSION_EXTRA env variables used if specified
///
/// Set the environment variable `CONDUWUIT_VERSION_EXTRA` to any UTF-8 string
/// to include it in parenthesis after the SemVer version. A common value are
/// git commit hashes.
#[must_use]
pub fn conduwuit_version() -> String {
	match option_env!("CONDUWUIT_VERSION_EXTRA") {
		Some(extra) => {
			if extra.is_empty() {
				env!("CARGO_PKG_VERSION").to_owned()
			} else {
				format!("{} ({})", env!("CARGO_PKG_VERSION"), extra)
			}
		},
		None => match option_env!("CONDUIT_VERSION_EXTRA") {
			Some(extra) => {
				if extra.is_empty() {
					env!("CARGO_PKG_VERSION").to_owned()
				} else {
					format!("{} ({})", env!("CARGO_PKG_VERSION"), extra)
				}
			},
			None => env!("CARGO_PKG_VERSION").to_owned(),
		},
	}
}

/// Debug-formats the given slice, but only up to the first `max_len` elements.
/// Any further elements are replaced by an ellipsis.
///
/// See also [`debug_slice_truncated()`],
pub struct TruncatedDebugSlice<'a, T> {
	inner: &'a [T],
	max_len: usize,
}

impl<T: fmt::Debug> fmt::Debug for TruncatedDebugSlice<'_, T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.inner.len() <= self.max_len {
			write!(f, "{:?}", self.inner)
		} else {
			f.debug_list()
				.entries(&self.inner[..self.max_len])
				.entry(&"...")
				.finish()
		}
	}
}

/// See [`TruncatedDebugSlice`]. Useful for `#[instrument]`:
///
/// ```
/// use conduit_core::utils::debug_slice_truncated;
///
/// #[tracing::instrument(fields(foos = debug_slice_truncated(foos, 42)))]
/// fn bar(foos: &[&str]);
/// ```
pub fn debug_slice_truncated<T: fmt::Debug>(
	slice: &[T], max_len: usize,
) -> tracing::field::DebugValue<TruncatedDebugSlice<'_, T>> {
	tracing::field::debug(TruncatedDebugSlice {
		inner: slice,
		max_len,
	})
}

/// This is needed for opening lots of file descriptors, which tends to
/// happen more often when using RocksDB and making lots of federation
/// connections at startup. The soft limit is usually 1024, and the hard
/// limit is usually 512000; I've personally seen it hit >2000.
///
/// * <https://www.freedesktop.org/software/systemd/man/systemd.exec.html#id-1.12.2.1.17.6>
/// * <https://github.com/systemd/systemd/commit/0abf94923b4a95a7d89bc526efc84e7ca2b71741>
#[cfg(unix)]
pub fn maximize_fd_limit() -> Result<(), nix::errno::Errno> {
	use nix::sys::resource::{getrlimit, setrlimit, Resource::RLIMIT_NOFILE as NOFILE};

	let (soft_limit, hard_limit) = getrlimit(NOFILE)?;
	if soft_limit < hard_limit {
		setrlimit(NOFILE, hard_limit, hard_limit)?;
		assert_eq!((hard_limit, hard_limit), getrlimit(NOFILE)?, "getrlimit != setrlimit");
		debug!(to = hard_limit, from = soft_limit, "Raised RLIMIT_NOFILE",);
	}

	Ok(())
}
