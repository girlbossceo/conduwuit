//! Extended external extensions to futures::TryFutureExt

use futures::{
	future::{MapOkOrElse, UnwrapOrElse},
	TryFuture, TryFutureExt,
};

/// This interface is not necessarily complete; feel free to add as-needed.
pub trait TryExtExt<T, E>
where
	Self: TryFuture<Ok = T, Error = E> + Send,
{
	fn map_ok_or<U, F>(
		self, default: U, f: F,
	) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> U, impl FnOnce(Self::Error) -> U>
	where
		F: FnOnce(Self::Ok) -> U,
		Self: Send + Sized;

	fn ok(
		self,
	) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> Option<Self::Ok>, impl FnOnce(Self::Error) -> Option<Self::Ok>>
	where
		Self: Sized;

	fn unwrap_or(self, default: Self::Ok) -> UnwrapOrElse<Self, impl FnOnce(Self::Error) -> Self::Ok>
	where
		Self: Sized;
}

impl<T, E, Fut> TryExtExt<T, E> for Fut
where
	Fut: TryFuture<Ok = T, Error = E> + Send,
{
	#[inline]
	fn map_ok_or<U, F>(
		self, default: U, f: F,
	) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> U, impl FnOnce(Self::Error) -> U>
	where
		F: FnOnce(Self::Ok) -> U,
		Self: Send + Sized,
	{
		self.map_ok_or_else(|_| default, f)
	}

	#[inline]
	fn ok(
		self,
	) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> Option<Self::Ok>, impl FnOnce(Self::Error) -> Option<Self::Ok>>
	where
		Self: Sized,
	{
		self.map_ok_or(None, Some)
	}

	#[inline]
	fn unwrap_or(self, default: Self::Ok) -> UnwrapOrElse<Self, impl FnOnce(Self::Error) -> Self::Ok>
	where
		Self: Sized,
	{
		self.unwrap_or_else(move |_| default)
	}
}
