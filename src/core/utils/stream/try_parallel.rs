//! Parallelism stream combinator extensions to futures::Stream

use futures::{stream::TryStream, TryFutureExt};
use tokio::{runtime, task::JoinError};

use super::TryBroadbandExt;
use crate::{utils::sys::available_parallelism, Error, Result};

/// Parallelism extensions to augment futures::StreamExt. These combinators are
/// for computation-oriented workloads, unlike -band combinators for I/O
/// workloads; these default to the available compute parallelism for the
/// system. Threads are currently drawn from the tokio-spawn pool. Results are
/// unordered.
pub trait TryParallelExt<T, E>
where
	Self: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
	E: From<JoinError> + From<Error> + Send + 'static,
	T: Send + 'static,
{
	fn paralleln_and_then<U, F, N, H>(
		self,
		h: H,
		n: N,
		f: F,
	) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
	where
		N: Into<Option<usize>>,
		H: Into<Option<runtime::Handle>>,
		F: Fn(Self::Ok) -> Result<U, E> + Clone + Send + 'static,
		U: Send + 'static;

	fn parallel_and_then<U, F, H>(
		self,
		h: H,
		f: F,
	) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
	where
		H: Into<Option<runtime::Handle>>,
		F: Fn(Self::Ok) -> Result<U, E> + Clone + Send + 'static,
		U: Send + 'static,
	{
		self.paralleln_and_then(h, None, f)
	}
}

impl<T, E, S> TryParallelExt<T, E> for S
where
	S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
	E: From<JoinError> + From<Error> + Send + 'static,
	T: Send + 'static,
{
	fn paralleln_and_then<U, F, N, H>(
		self,
		h: H,
		n: N,
		f: F,
	) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
	where
		N: Into<Option<usize>>,
		H: Into<Option<runtime::Handle>>,
		F: Fn(Self::Ok) -> Result<U, E> + Clone + Send + 'static,
		U: Send + 'static,
	{
		let n = n.into().unwrap_or_else(available_parallelism);
		let h = h.into().unwrap_or_else(runtime::Handle::current);
		self.broadn_and_then(n, move |val| {
			let (h, f) = (h.clone(), f.clone());
			async move { h.spawn_blocking(move || f(val)).map_err(E::from).await? }
		})
	}
}
