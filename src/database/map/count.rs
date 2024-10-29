use std::{fmt::Debug, future::Future};

use conduit::implement;
use futures::stream::StreamExt;
use serde::Serialize;

/// Count the total number of entries in the map.
#[implement(super::Map)]
#[inline]
pub fn count(&self) -> impl Future<Output = usize> + Send + '_ { self.raw_keys().count() }

/// Count the number of entries in the map starting from a lower-bound.
///
/// - From is a structured key
#[implement(super::Map)]
#[inline]
pub fn count_from<'a, P>(&'a self, from: &P) -> impl Future<Output = usize> + Send + 'a
where
	P: Serialize + ?Sized + Debug + 'a,
{
	self.keys_from_raw(from).count()
}

/// Count the number of entries in the map starting from a lower-bound.
///
/// - From is a raw
#[implement(super::Map)]
#[inline]
pub fn raw_count_from<'a, P>(&'a self, from: &'a P) -> impl Future<Output = usize> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
	self.raw_keys_from(from).count()
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
	self.keys_prefix_raw(prefix).count()
}

/// Count the number of entries in the map matching a prefix.
///
/// - Prefix is raw
#[implement(super::Map)]
#[inline]
pub fn raw_count_prefix<'a, P>(&'a self, prefix: &'a P) -> impl Future<Output = usize> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
	self.raw_keys_prefix(prefix).count()
}
