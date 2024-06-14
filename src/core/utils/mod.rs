pub mod content_disposition;
pub mod debug;
pub mod defer;
pub mod hash;
pub mod html;
pub mod json;
pub mod mutex_map;
pub mod sys;

use std::{
	cmp::{self, Ordering},
	time::{SystemTime, UNIX_EPOCH},
};

pub use debug::slice_truncated as debug_slice_truncated;
pub use html::Escape as HtmlEscape;
pub use json::{deserialize_from_str, to_canonical_object};
pub use mutex_map::MutexMap;
use rand::prelude::*;
use ring::digest;
use ruma::OwnedUserId;
pub use sys::available_parallelism;

use crate::{Error, Result};

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

/// Boilerplate for wraps which are typed to never error.
///
/// * <https://doc.rust-lang.org/std/convert/enum.Infallible.html>
#[must_use]
#[inline(always)]
pub fn unwrap_infallible<T>(result: Result<T, std::convert::Infallible>) -> T {
	match result {
		Ok(val) => val,
		Err(err) => match err {},
	}
}
