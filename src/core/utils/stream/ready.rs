//! Synchronous combinator extensions to futures::Stream

use futures::{
	future::{ready, Ready},
	stream::{Any, Filter, FilterMap, Fold, ForEach, SkipWhile, Stream, StreamExt, TakeWhile},
};

/// Synchronous combinators to augment futures::StreamExt. Most Stream
/// combinators take asynchronous arguments, but often only simple predicates
/// are required to steer a Stream like an Iterator. This suite provides a
/// convenience to reduce boilerplate by de-cluttering non-async predicates.
///
/// This interface is not necessarily complete; feel free to add as-needed.
pub trait ReadyExt<Item, S>
where
	S: Stream<Item = Item> + Send + ?Sized,
	Self: Stream + Send + Sized,
{
	fn ready_any<F>(self, f: F) -> Any<Self, Ready<bool>, impl FnMut(S::Item) -> Ready<bool>>
	where
		F: Fn(S::Item) -> bool;

	fn ready_filter<'a, F>(self, f: F) -> Filter<Self, Ready<bool>, impl FnMut(&S::Item) -> Ready<bool> + 'a>
	where
		F: Fn(&S::Item) -> bool + 'a;

	fn ready_filter_map<F, U>(self, f: F) -> FilterMap<Self, Ready<Option<U>>, impl FnMut(S::Item) -> Ready<Option<U>>>
	where
		F: Fn(S::Item) -> Option<U>;

	fn ready_fold<T, F>(self, init: T, f: F) -> Fold<Self, Ready<T>, T, impl FnMut(T, S::Item) -> Ready<T>>
	where
		F: Fn(T, S::Item) -> T;

	fn ready_for_each<F>(self, f: F) -> ForEach<Self, Ready<()>, impl FnMut(S::Item) -> Ready<()>>
	where
		F: FnMut(S::Item);

	fn ready_take_while<'a, F>(self, f: F) -> TakeWhile<Self, Ready<bool>, impl FnMut(&S::Item) -> Ready<bool> + 'a>
	where
		F: Fn(&S::Item) -> bool + 'a;

	fn ready_skip_while<'a, F>(self, f: F) -> SkipWhile<Self, Ready<bool>, impl FnMut(&S::Item) -> Ready<bool> + 'a>
	where
		F: Fn(&S::Item) -> bool + 'a;
}

impl<Item, S> ReadyExt<Item, S> for S
where
	S: Stream<Item = Item> + Send + ?Sized,
	Self: Stream + Send + Sized,
{
	#[inline]
	fn ready_any<F>(self, f: F) -> Any<Self, Ready<bool>, impl FnMut(S::Item) -> Ready<bool>>
	where
		F: Fn(S::Item) -> bool,
	{
		self.any(move |t| ready(f(t)))
	}

	#[inline]
	fn ready_filter<'a, F>(self, f: F) -> Filter<Self, Ready<bool>, impl FnMut(&S::Item) -> Ready<bool> + 'a>
	where
		F: Fn(&S::Item) -> bool + 'a,
	{
		self.filter(move |t| ready(f(t)))
	}

	#[inline]
	fn ready_filter_map<F, U>(self, f: F) -> FilterMap<Self, Ready<Option<U>>, impl FnMut(S::Item) -> Ready<Option<U>>>
	where
		F: Fn(S::Item) -> Option<U>,
	{
		self.filter_map(move |t| ready(f(t)))
	}

	#[inline]
	fn ready_fold<T, F>(self, init: T, f: F) -> Fold<Self, Ready<T>, T, impl FnMut(T, S::Item) -> Ready<T>>
	where
		F: Fn(T, S::Item) -> T,
	{
		self.fold(init, move |a, t| ready(f(a, t)))
	}

	#[inline]
	#[allow(clippy::unit_arg)]
	fn ready_for_each<F>(self, mut f: F) -> ForEach<Self, Ready<()>, impl FnMut(S::Item) -> Ready<()>>
	where
		F: FnMut(S::Item),
	{
		self.for_each(move |t| ready(f(t)))
	}

	#[inline]
	fn ready_take_while<'a, F>(self, f: F) -> TakeWhile<Self, Ready<bool>, impl FnMut(&S::Item) -> Ready<bool> + 'a>
	where
		F: Fn(&S::Item) -> bool + 'a,
	{
		self.take_while(move |t| ready(f(t)))
	}

	#[inline]
	fn ready_skip_while<'a, F>(self, f: F) -> SkipWhile<Self, Ready<bool>, impl FnMut(&S::Item) -> Ready<bool> + 'a>
	where
		F: Fn(&S::Item) -> bool + 'a,
	{
		self.skip_while(move |t| ready(f(t)))
	}
}
