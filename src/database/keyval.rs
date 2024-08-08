use conduit::Result;
use serde::Deserialize;

use crate::de;

pub(crate) type OwnedKeyVal = (Vec<u8>, Vec<u8>);
pub(crate) type OwnedKey = Vec<u8>;
pub(crate) type OwnedVal = Vec<u8>;

pub type KeyVal<'a, K = &'a Slice, V = &'a Slice> = (Key<'a, K>, Val<'a, V>);
pub type Key<'a, T = &'a Slice> = T;
pub type Val<'a, T = &'a Slice> = T;

pub type Slice = [u8];

#[inline]
pub(crate) fn _expect_deserialize<'a, K, V>(kv: Result<KeyVal<'a>>) -> KeyVal<'a, K, V>
where
	K: Deserialize<'a>,
	V: Deserialize<'a>,
{
	result_deserialize(kv).expect("failed to deserialize result key/val")
}

#[inline]
pub(crate) fn _expect_deserialize_key<'a, K>(key: Result<Key<'a>>) -> Key<'a, K>
where
	K: Deserialize<'a>,
{
	result_deserialize_key(key).expect("failed to deserialize result key")
}

#[inline]
pub(crate) fn result_deserialize<'a, K, V>(kv: Result<KeyVal<'a>>) -> Result<KeyVal<'a, K, V>>
where
	K: Deserialize<'a>,
	V: Deserialize<'a>,
{
	deserialize(kv?)
}

#[inline]
pub(crate) fn result_deserialize_key<'a, K>(key: Result<Key<'a>>) -> Result<Key<'a, K>>
where
	K: Deserialize<'a>,
{
	deserialize_key(key?)
}

#[inline]
pub(crate) fn deserialize<'a, K, V>(kv: KeyVal<'a>) -> Result<KeyVal<'a, K, V>>
where
	K: Deserialize<'a>,
	V: Deserialize<'a>,
{
	Ok((deserialize_key::<K>(kv.0)?, deserialize_val::<V>(kv.1)?))
}

#[inline]
pub(crate) fn deserialize_key<'a, K>(key: Key<'a>) -> Result<Key<'a, K>>
where
	K: Deserialize<'a>,
{
	de::from_slice::<K>(key)
}

#[inline]
pub(crate) fn deserialize_val<'a, V>(val: Val<'a>) -> Result<Val<'a, V>>
where
	V: Deserialize<'a>,
{
	de::from_slice::<V>(val)
}

#[inline]
#[must_use]
pub fn to_owned(kv: KeyVal<'_>) -> OwnedKeyVal { (kv.0.to_owned(), kv.1.to_owned()) }

#[inline]
pub fn key<K, V>(kv: KeyVal<'_, K, V>) -> Key<'_, K> { kv.0 }

#[inline]
pub fn val<K, V>(kv: KeyVal<'_, K, V>) -> Val<'_, V> { kv.1 }
