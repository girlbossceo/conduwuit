use std::{fmt::Debug, future::Future};

use conduit::implement;
use futures::stream::StreamExt;
use serde::Serialize;

use crate::de::Ignore;

/// Count the total number of entries in the map.
#[implement(super::Map)]
#[inline]
pub fn count(&self) -> impl Future<Output = usize> + Send + '_ { self.keys::<Ignore>().count() }

/// Count the number of entries in the map starting from a lower-bound.
///
/// - From is a structured key
#[implement(super::Map)]
#[inline]
pub fn count_from<'a, P>(&'a self, from: &P) -> impl Future<Output = usize> + Send + 'a
where
	P: Serialize + ?Sized + Debug + 'a,
{
	self.keys_from::<Ignore, P>(from).count()
}

/// Count the number of entries in the map matching a prefix.
///
/// - Prefix is structured key
#[implement(super::Map)]
#[inline]
pub fn count_prefix<'a, P>(&'a self, prefix: &P) -> impl Future<Output = usize> + Send + 'a
where
	P: Serialize + ?Sized + Debug + 'a,
{
	self.keys_prefix::<Ignore, P>(prefix).count()
}
