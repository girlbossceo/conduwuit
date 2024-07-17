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

use std::cmp::{self, Ordering};

pub use bytes::{increment, u64_from_bytes, u64_from_u8, u64_from_u8x8};
pub use debug::slice_truncated as debug_slice_truncated;
pub use hash::calculate_hash;
pub use html::Escape as HtmlEscape;
pub use json::{deserialize_from_str, to_canonical_object};
pub use mutex_map::{Guard as MutexMapGuard, MutexMap};
pub use rand::string as random_string;
pub use string::{str_from_bytes, string_from_bytes};
pub use sys::available_parallelism;
pub use time::now_millis as millis_since_unix_epoch;

pub fn clamp<T: Ord>(val: T, min: T, max: T) -> T { cmp::min(cmp::max(val, min), max) }

#[must_use]
pub fn generate_keypair() -> Vec<u8> {
	let mut value = rand::string(8).as_bytes().to_vec();
	value.push(0xFF);
	value.extend_from_slice(
		&ruma::signatures::Ed25519KeyPair::generate().expect("Ed25519KeyPair generation always works (?)"),
	);
	value
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
