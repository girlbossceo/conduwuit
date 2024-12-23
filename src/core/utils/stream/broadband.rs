//! Broadband stream combinator extensions to futures::Stream

use std::convert::identity;

use futures::{
	stream::{Stream, StreamExt},
	Future,
};

use super::{automatic_width, ReadyExt};

/// Concurrency extensions to augment futures::StreamExt. broad_ combinators
/// produce out-of-order
pub trait BroadbandExt<Item>
where
	Self: Stream<Item = Item> + Send + Sized,
{
	fn broadn_all<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = bool> + Send;

	fn broadn_any<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = bool> + Send;

	/// Concurrent filter_map(); unordered results
	fn broadn_filter_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = Option<U>> + Send,
		U: Send;

	fn broadn_then<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = U> + Send,
		U: Send;

	#[inline]
	fn broad_all<F, Fut>(self, f: F) -> impl Future<Output = bool> + Send
	where
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = bool> + Send,
	{
		self.broadn_all(None, f)
	}

	#[inline]
	fn broad_any<F, Fut>(self, f: F) -> impl Future<Output = bool> + Send
	where
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = bool> + Send,
	{
		self.broadn_any(None, f)
	}

	#[inline]
	fn broad_filter_map<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
	where
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = Option<U>> + Send,
		U: Send,
	{
		self.broadn_filter_map(None, f)
	}

	#[inline]
	fn broad_then<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
	where
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = U> + Send,
		U: Send,
	{
		self.broadn_then(None, f)
	}
}

impl<Item, S> BroadbandExt<Item> for S
where
	S: Stream<Item = Item> + Send + Sized,
{
	#[inline]
	fn broadn_all<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = bool> + Send,
	{
		self.map(f)
			.buffer_unordered(n.into().unwrap_or_else(automatic_width))
			.ready_all(identity)
	}

	#[inline]
	fn broadn_any<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = bool> + Send,
	{
		self.map(f)
			.buffer_unordered(n.into().unwrap_or_else(automatic_width))
			.ready_any(identity)
	}

	#[inline]
	fn broadn_filter_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = Option<U>> + Send,
		U: Send,
	{
		self.map(f)
			.buffer_unordered(n.into().unwrap_or_else(automatic_width))
			.ready_filter_map(identity)
	}

	#[inline]
	fn broadn_then<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
	where
		N: Into<Option<usize>>,
		F: Fn(Item) -> Fut + Send,
		Fut: Future<Output = U> + Send,
		U: Send,
	{
		self.map(f)
			.buffer_unordered(n.into().unwrap_or_else(automatic_width))
	}
}
