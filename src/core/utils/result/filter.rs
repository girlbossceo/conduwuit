use super::Result;

pub trait Filter<T, E> {
	/// Similar to Option::filter
	#[must_use]
	fn filter<P, U>(self, predicate: P) -> Self
	where
		P: FnOnce(&T) -> Result<(), U>,
		E: From<U>;
}

impl<T, E> Filter<T, E> for Result<T, E> {
	#[inline]
	fn filter<P, U>(self, predicate: P) -> Self
	where
		P: FnOnce(&T) -> Result<(), U>,
		E: From<U>,
	{
		self.and_then(move |t| predicate(&t).map(move |()| t).map_err(Into::into))
	}
}
