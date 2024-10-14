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
macro_rules! at {
	($idx:tt) => {
		|t| t.$idx
	};
}
