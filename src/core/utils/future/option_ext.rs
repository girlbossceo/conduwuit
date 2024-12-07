#![allow(clippy::wrong_self_convention)]

use futures::{future::OptionFuture, Future, FutureExt};

pub trait OptionExt<T> {
	fn is_none_or(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send;

	fn is_some_and(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send;
}

impl<T, Fut> OptionExt<T> for OptionFuture<Fut>
where
	Fut: Future<Output = T> + Send,
{
	fn is_none_or(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send {
		self.map(|o| o.as_ref().is_none_or(f))
	}

	fn is_some_and(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send {
		self.map(|o| o.as_ref().is_some_and(f))
	}
}
