use std::{
	cmp::{Eq, Ord},
	pin::Pin,
	sync::Arc,
};

use futures::{Stream, StreamExt};

use crate::{is_equal_to, is_less_than};

/// Intersection of sets
///
/// Outputs the set of elements common to all input sets. Inputs do not have to
/// be sorted. If inputs are sorted a more optimized function is available in
/// this suite and should be used.
pub fn intersection<Item, Iter, Iters>(mut input: Iters) -> impl Iterator<Item = Item> + Send
where
	Iters: Iterator<Item = Iter> + Clone + Send,
	Iter: Iterator<Item = Item> + Send,
	Item: Eq + Send,
{
	input.next().into_iter().flat_map(move |first| {
		let input = input.clone();
		first.filter(move |targ| {
			input
				.clone()
				.all(|mut other| other.any(is_equal_to!(*targ)))
		})
	})
}

/// Intersection of sets
///
/// Outputs the set of elements common to all input sets. Inputs must be sorted.
pub fn intersection_sorted<Item, Iter, Iters>(mut input: Iters) -> impl Iterator<Item = Item> + Send
where
	Iters: Iterator<Item = Iter> + Clone + Send,
	Iter: Iterator<Item = Item> + Send,
	Item: Eq + Ord + Send,
{
	input.next().into_iter().flat_map(move |first| {
		let mut input = input.clone().collect::<Vec<_>>();
		first.filter(move |targ| {
			input.iter_mut().all(|it| {
				it.by_ref()
					.skip_while(is_less_than!(targ))
					.peekable()
					.peek()
					.is_some_and(is_equal_to!(targ))
			})
		})
	})
}

/// Intersection of sets
///
/// Outputs the set of elements common to both streams. Streams must be sorted.
pub fn intersection_sorted_stream2<Item, S>(a: S, b: S) -> impl Stream<Item = Item> + Send
where
	S: Stream<Item = Item> + Send + Unpin,
	Item: Eq + PartialOrd + Send + Sync,
{
	use tokio::sync::Mutex;

	let b = Arc::new(Mutex::new(b.peekable()));
	a.map(move |ai| (ai, b.clone()))
		.filter_map(|(ai, b)| async move {
			let mut lock = b.lock().await;
			while let Some(bi) = Pin::new(&mut *lock).next_if(|bi| *bi <= ai).await.as_ref() {
				if ai == *bi {
					return Some(ai);
				}
			}

			None
		})
}
