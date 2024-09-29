use std::{convert::AsRef, fmt::Debug};

use conduit::implement;
use rocksdb::WriteBatchWithTransaction;

use crate::util::or_else;

#[implement(super::Map)]
#[tracing::instrument(skip(self, value), fields(%self), level = "trace")]
pub fn insert<K, V>(&self, key: &K, value: &V)
where
	K: AsRef<[u8]> + ?Sized + Debug,
	V: AsRef<[u8]> + ?Sized,
{
	let write_options = &self.write_options;
	self.db
		.db
		.put_cf_opt(&self.cf(), key, value, write_options)
		.or_else(or_else)
		.expect("database insert error");

	if !self.db.corked() {
		self.db.flush().expect("database flush error");
	}

	self.watchers.wake(key.as_ref());
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, iter), fields(%self), level = "trace")]
pub fn insert_batch<'a, I, K, V>(&'a self, iter: I)
where
	I: Iterator<Item = &'a (K, V)> + Send + Debug,
	K: AsRef<[u8]> + Sized + Debug + 'a,
	V: AsRef<[u8]> + Sized + 'a,
{
	let mut batch = WriteBatchWithTransaction::<false>::default();
	for (key, val) in iter {
		batch.put_cf(&self.cf(), key.as_ref(), val.as_ref());
	}

	let write_options = &self.write_options;
	self.db
		.db
		.write_opt(batch, write_options)
		.or_else(or_else)
		.expect("database insert batch error");

	if !self.db.corked() {
		self.db.flush().expect("database flush error");
	}
}
