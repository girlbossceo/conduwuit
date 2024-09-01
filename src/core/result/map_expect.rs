use std::fmt::Debug;

use super::Result;

pub trait MapExpect<T> {
	/// Calls expect(msg) on the mapped Result value. This is similar to
	/// map(Result::unwrap) but composes an expect call and message without
	/// requiring a closure.
	fn map_expect(self, msg: &str) -> Option<T>;
}

impl<T, E: Debug> MapExpect<T> for Option<Result<T, E>> {
	#[inline]
	fn map_expect(self, msg: &str) -> Option<T> { self.map(|result| result.expect(msg)) }
}
