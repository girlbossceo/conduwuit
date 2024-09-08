pub mod algorithm;
pub mod bytes;
pub mod content_disposition;
pub mod debug;
pub mod defer;
pub mod hash;
pub mod html;
pub mod json;
pub mod math;
pub mod mutex_map;
pub mod rand;
pub mod string;
pub mod sys;
mod tests;
pub mod time;

use std::cmp;

pub use ::ctor::{ctor, dtor};
pub use algorithm::common_elements;
pub use bytes::{increment, u64_from_bytes, u64_from_u8, u64_from_u8x8};
pub use conduit_macros::implement;
pub use debug::slice_truncated as debug_slice_truncated;
pub use hash::calculate_hash;
pub use html::Escape as HtmlEscape;
pub use json::{deserialize_from_str, to_canonical_object};
pub use mutex_map::{Guard as MutexMapGuard, MutexMap};
pub use rand::string as random_string;
pub use string::{str_from_bytes, string_from_bytes};
pub use sys::available_parallelism;
pub use time::now_millis as millis_since_unix_epoch;

#[inline]
pub fn clamp<T: Ord>(val: T, min: T, max: T) -> T { cmp::min(cmp::max(val, min), max) }

#[inline]
pub fn exchange<T: Clone>(state: &mut T, source: T) -> T {
	let ret = state.clone();
	*state = source;
	ret
}

#[must_use]
pub fn generate_keypair() -> Vec<u8> {
	let mut value = rand::string(8).as_bytes().to_vec();
	value.push(0xFF);
	value.extend_from_slice(
		&ruma::signatures::Ed25519KeyPair::generate().expect("Ed25519KeyPair generation always works (?)"),
	);
	value
}
