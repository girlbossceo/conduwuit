use futures::{future::ready, Stream, StreamExt, TryStream};

use crate::{Error, Result};

pub trait TryIgnore<'a, Item> {
	fn ignore_err(self) -> impl Stream<Item = Item> + Send + 'a;

	fn ignore_ok(self) -> impl Stream<Item = Error> + Send + 'a;
}

impl<'a, T, Item> TryIgnore<'a, Item> for T
where
	T: Stream<Item = Result<Item>> + TryStream + Send + 'a,
	Item: Send + 'a,
{
	#[inline]
	fn ignore_err(self: T) -> impl Stream<Item = Item> + Send + 'a { self.filter_map(|res| ready(res.ok())) }

	#[inline]
	fn ignore_ok(self: T) -> impl Stream<Item = Error> + Send + 'a { self.filter_map(|res| ready(res.err())) }
}
