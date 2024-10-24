//! Synchronous combinator extensions to futures::TryStream

use futures::{
	future::{ready, Ready},
	stream::{AndThen, TryStream, TryStreamExt},
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
}
