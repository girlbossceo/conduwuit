//! StreamTools for futures::Stream

use std::{collections::HashMap, hash::Hash};

use futures::{Future, Stream, StreamExt};

use super::ReadyExt;
use crate::expected;

/// StreamTools
///
/// This interface is not necessarily complete; feel free to add as-needed.
pub trait Tools<Item>
where
	Self: Stream<Item = Item> + Send + Sized,
	<Self as Stream>::Item: Send,
{
	fn counts(self) -> impl Future<Output = HashMap<Item, usize>> + Send
	where
		<Self as Stream>::Item: Eq + Hash;

	fn counts_by<K, F>(self, f: F) -> impl Future<Output = HashMap<K, usize>> + Send
	where
		F: Fn(Item) -> K + Send,
		K: Eq + Hash + Send;

	fn counts_by_with_cap<const CAP: usize, K, F>(self, f: F) -> impl Future<Output = HashMap<K, usize>> + Send
	where
		F: Fn(Item) -> K + Send,
		K: Eq + Hash + Send;

	fn counts_with_cap<const CAP: usize>(self) -> impl Future<Output = HashMap<Item, usize>> + Send
	where
		<Self as Stream>::Item: Eq + Hash;
}

impl<Item, S> Tools<Item> for S
where
	S: Stream<Item = Item> + Send + Sized,
	<Self as Stream>::Item: Send,
{
	#[inline]
	fn counts(self) -> impl Future<Output = HashMap<Item, usize>> + Send
	where
		<Self as Stream>::Item: Eq + Hash,
	{
		self.counts_with_cap::<0>()
	}

	#[inline]
	fn counts_by<K, F>(self, f: F) -> impl Future<Output = HashMap<K, usize>> + Send
	where
		F: Fn(Item) -> K + Send,
		K: Eq + Hash + Send,
	{
		self.counts_by_with_cap::<0, K, F>(f)
	}

	#[inline]
	fn counts_by_with_cap<const CAP: usize, K, F>(self, f: F) -> impl Future<Output = HashMap<K, usize>> + Send
	where
		F: Fn(Item) -> K + Send,
		K: Eq + Hash + Send,
	{
		self.map(f).counts_with_cap::<CAP>()
	}

	#[inline]
	fn counts_with_cap<const CAP: usize>(self) -> impl Future<Output = HashMap<Item, usize>> + Send
	where
		<Self as Stream>::Item: Eq + Hash,
	{
		self.ready_fold(HashMap::with_capacity(CAP), |mut counts, item| {
			let entry = counts.entry(item).or_default();
			let value = *entry;
			*entry = expected!(value + 1);
			counts
		})
	}
}
