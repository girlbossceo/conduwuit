//! Trait BoolExt

/// Boolean extensions and chain.starters
pub trait BoolExt {
	fn or<T, F: FnOnce() -> T>(self, f: F) -> Option<T>;

	fn or_some<T>(self, t: T) -> Option<T>;
}

impl BoolExt for bool {
	#[inline]
	fn or<T, F: FnOnce() -> T>(self, f: F) -> Option<T> { (!self).then(f) }

	#[inline]
	fn or_some<T>(self, t: T) -> Option<T> { (!self).then_some(t) }
}
