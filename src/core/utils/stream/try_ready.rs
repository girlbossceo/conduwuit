//! Synchronous combinator extensions to futures::TryStream
#![allow(clippy::type_complexity)]

use futures::{
	future::{ready, Ready},
	stream::{AndThen, TryFilterMap, TryFold, TryForEach, TryStream, TryStreamExt},
};

use crate::Result;

/// Synchronous combinators to augment futures::TryStreamExt.
///
/// This interface is not necessarily complete; feel free to add as-needed.
pub trait TryReadyExt<T, E, S>
where
	S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + ?Sized,
	Self: TryStream + Send + Sized,
{
	fn ready_and_then<U, F>(self, f: F) -> AndThen<Self, Ready<Result<U, E>>, impl FnMut(S::Ok) -> Ready<Result<U, E>>>
	where
		F: Fn(S::Ok) -> Result<U, E>;

	fn ready_try_filter_map<F, U>(
		self, f: F,
	) -> TryFilterMap<Self, Ready<Result<Option<U>, E>>, impl FnMut(S::Ok) -> Ready<Result<Option<U>, E>>>
	where
		F: Fn(S::Ok) -> Result<Option<U>, E>;

	fn ready_try_fold<U, F>(
		self, init: U, f: F,
	) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
	where
		F: Fn(U, S::Ok) -> Result<U, E>;

	fn ready_try_fold_default<U, F>(
		self, f: F,
	) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
	where
		F: Fn(U, S::Ok) -> Result<U, E>,
		U: Default;

	fn ready_try_for_each<F>(
		self, f: F,
	) -> TryForEach<Self, Ready<Result<(), E>>, impl FnMut(S::Ok) -> Ready<Result<(), E>>>
	where
		F: FnMut(S::Ok) -> Result<(), E>;
}

impl<T, E, S> TryReadyExt<T, E, S> for S
where
	S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + ?Sized,
	Self: TryStream + Send + Sized,
{
	#[inline]
	fn ready_and_then<U, F>(self, f: F) -> AndThen<Self, Ready<Result<U, E>>, impl FnMut(S::Ok) -> Ready<Result<U, E>>>
	where
		F: Fn(S::Ok) -> Result<U, E>,
	{
		self.and_then(move |t| ready(f(t)))
	}

	fn ready_try_filter_map<F, U>(
		self, f: F,
	) -> TryFilterMap<Self, Ready<Result<Option<U>, E>>, impl FnMut(S::Ok) -> Ready<Result<Option<U>, E>>>
	where
		F: Fn(S::Ok) -> Result<Option<U>, E>,
	{
		self.try_filter_map(move |t| ready(f(t)))
	}

	#[inline]
	fn ready_try_fold<U, F>(
		self, init: U, f: F,
	) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
	where
		F: Fn(U, S::Ok) -> Result<U, E>,
	{
		self.try_fold(init, move |a, t| ready(f(a, t)))
	}

	#[inline]
	fn ready_try_fold_default<U, F>(
		self, f: F,
	) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
	where
		F: Fn(U, S::Ok) -> Result<U, E>,
		U: Default,
	{
		self.ready_try_fold(U::default(), f)
	}

	#[inline]
	fn ready_try_for_each<F>(
		self, mut f: F,
	) -> TryForEach<Self, Ready<Result<(), E>>, impl FnMut(S::Ok) -> Ready<Result<(), E>>>
	where
		F: FnMut(S::Ok) -> Result<(), E>,
	{
		self.try_for_each(move |t| ready(f(t)))
	}
}
