//! Synchronous combinator extensions to futures::TryStream

use futures::{TryFuture, TryStream, TryStreamExt};

use super::automatic_width;
use crate::Result;

/// Concurrency extensions to augment futures::TryStreamExt. wide_ combinators
/// produce in-order results
pub trait TryWidebandExt<T, E>
where
	Self: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
{
	fn widen_and_then<U, F, Fut, N>(
		self,
		n: N,
		f: F,
	) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Self::Ok) -> Fut + Send,
		Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
		U: Send;

	fn wide_and_then<U, F, Fut>(
		self,
		f: F,
	) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
	where
		F: Fn(Self::Ok) -> Fut + Send,
		Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
		U: Send,
	{
		self.widen_and_then(None, f)
	}
}

impl<T, E, S> TryWidebandExt<T, E> for S
where
	S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
	E: Send,
{
	fn widen_and_then<U, F, Fut, N>(
		self,
		n: N,
		f: F,
	) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Self::Ok) -> Fut + Send,
		Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
		U: Send,
	{
		self.map_ok(f)
			.try_buffered(n.into().unwrap_or_else(automatic_width))
	}
}
