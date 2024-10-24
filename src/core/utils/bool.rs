//! Trait BoolExt

/// Boolean extensions and chain.starters
pub trait BoolExt {
	fn map<T, F: FnOnce(Self) -> T>(self, f: F) -> T
	where
		Self: Sized;

	fn map_ok_or<T, E, F: FnOnce() -> T>(self, err: E, f: F) -> Result<T, E>;

	fn map_or<T, F: FnOnce() -> T>(self, err: T, f: F) -> T;

	fn map_or_else<T, F: FnOnce() -> T>(self, err: F, f: F) -> T;

	fn ok_or<E>(self, err: E) -> Result<(), E>;

	fn ok_or_else<E, F: FnOnce() -> E>(self, err: F) -> Result<(), E>;

	fn or<T, F: FnOnce() -> T>(self, f: F) -> Option<T>;

	fn or_some<T>(self, t: T) -> Option<T>;
}

impl BoolExt for bool {
	#[inline]
	fn map<T, F: FnOnce(Self) -> T>(self, f: F) -> T
	where
		Self: Sized,
	{
		f(self)
	}

	#[inline]
	fn map_ok_or<T, E, F: FnOnce() -> T>(self, err: E, f: F) -> Result<T, E> { self.ok_or(err).map(|()| f()) }

	#[inline]
	fn map_or<T, F: FnOnce() -> T>(self, err: T, f: F) -> T { self.then(f).unwrap_or(err) }

	#[inline]
	fn map_or_else<T, F: FnOnce() -> T>(self, err: F, f: F) -> T { self.then(f).unwrap_or_else(err) }

	#[inline]
	fn ok_or<E>(self, err: E) -> Result<(), E> { self.then_some(()).ok_or(err) }

	#[inline]
	fn ok_or_else<E, F: FnOnce() -> E>(self, err: F) -> Result<(), E> { self.then_some(()).ok_or_else(err) }

	#[inline]
	fn or<T, F: FnOnce() -> T>(self, f: F) -> Option<T> { (!self).then(f) }

	#[inline]
	fn or_some<T>(self, t: T) -> Option<T> { (!self).then_some(t) }
}
