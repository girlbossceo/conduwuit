pub mod bool;
pub mod bytes;
pub mod content_disposition;
pub mod debug;
pub mod defer;
pub mod future;
pub mod hash;
pub mod html;
pub mod json;
pub mod math;
pub mod mutex_map;
pub mod rand;
pub mod result;
pub mod set;
pub mod stream;
pub mod string;
pub mod sys;
mod tests;
pub mod time;

pub use ::conduit_macros::implement;
pub use ::ctor::{ctor, dtor};

pub use self::{
	bool::BoolExt,
	bytes::{increment, u64_from_bytes, u64_from_u8, u64_from_u8x8},
	debug::slice_truncated as debug_slice_truncated,
	future::TryExtExt as TryFutureExtExt,
	hash::calculate_hash,
	html::Escape as HtmlEscape,
	json::{deserialize_from_str, to_canonical_object},
	math::clamp,
	mutex_map::{Guard as MutexMapGuard, MutexMap},
	rand::{shuffle, string as random_string},
	stream::{IterStream, ReadyExt, Tools as StreamTools, TryReadyExt},
	string::{str_from_bytes, string_from_bytes},
	sys::available_parallelism,
	time::{now_millis as millis_since_unix_epoch, timepoint_ago, timepoint_from_now},
};

#[inline]
pub fn exchange<T>(state: &mut T, source: T) -> T { std::mem::replace(state, source) }

#[macro_export]
macro_rules! extract_variant {
	($e:expr, $variant:path) => {
		match $e {
			$variant(value) => Some(value),
			_ => None,
		}
	};
}

#[macro_export]
macro_rules! at {
	($idx:tt) => {
		|t| t.$idx
	};
}

/// Functor for equality i.e. .is_some_and(is_equal!(2))
#[macro_export]
macro_rules! is_equal_to {
	($val:ident) => {
		|x| x == $val
	};

	($val:expr) => {
		|x| x == $val
	};
}

/// Functor for less i.e. .is_some_and(is_less_than!(2))
#[macro_export]
macro_rules! is_less_than {
	($val:ident) => {
		|x| x < $val
	};

	($val:expr) => {
		|x| x < $val
	};
}

/// Functor for equality to zero
#[macro_export]
macro_rules! is_zero {
	() => {
		$crate::is_matching!(0)
	};
}

/// Functor for matches! i.e. .is_some_and(is_matching!('A'..='Z'))
#[macro_export]
macro_rules! is_matching {
	($val:ident) => {
		|x| matches!(x, $val)
	};

	($val:expr) => {
		|x| matches!(x, $val)
	};
}

/// Functor for !is_empty()
#[macro_export]
macro_rules! is_not_empty {
	() => {
		|x| !x.is_empty()
	};
}

/// Functor for truthy
#[macro_export]
macro_rules! is_true {
	() => {
		|x| !!x
	};
}

/// Functor for falsy
#[macro_export]
macro_rules! is_false {
	() => {
		|x| !x
	};
}
