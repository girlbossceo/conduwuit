use std::{error::Error, future::Future, pin::Pin, sync::Arc};

use super::Config;
use crate::Result;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "rocksdb")]
pub(crate) mod rocksdb;

#[cfg(any(feature = "sqlite", feature = "rocksdb"))]
pub(crate) mod watchers;

pub(crate) trait KeyValueDatabaseEngine: Send + Sync {
	fn open(config: &Config) -> Result<Self>
	where
		Self: Sized;
	fn open_tree(&self, name: &'static str) -> Result<Arc<dyn KvTree>>;
	fn flush(&self) -> Result<()>;
	#[allow(dead_code)]
	fn sync(&self) -> Result<()> { Ok(()) }
	fn cork(&self) -> Result<()> { Ok(()) }
	fn uncork(&self) -> Result<()> { Ok(()) }
	fn corked(&self) -> bool { false }
	fn cleanup(&self) -> Result<()> { Ok(()) }
	fn memory_usage(&self) -> Result<String> {
		Ok("Current database engine does not support memory usage reporting.".to_owned())
	}

	#[allow(dead_code)]
	fn clear_caches(&self) {}

	fn backup(&self) -> Result<(), Box<dyn Error>> { unimplemented!() }

	fn backup_list(&self) -> Result<String> { Ok(String::new()) }

	fn file_list(&self) -> Result<String> { Ok(String::new()) }
}

pub(crate) trait KvTree: Send + Sync {
	fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

	#[allow(dead_code)]
	#[cfg(feature = "rocksdb")]
	fn multi_get(
		&self, _iter: Vec<(&Arc<rust_rocksdb::BoundColumnFamily<'_>>, Vec<u8>)>,
	) -> Vec<Result<Option<Vec<u8>>, rust_rocksdb::Error>> {
		unimplemented!()
	}

	fn insert(&self, key: &[u8], value: &[u8]) -> Result<()>;
	fn insert_batch(&self, iter: &mut dyn Iterator<Item = (Vec<u8>, Vec<u8>)>) -> Result<()> {
		for (key, value) in iter {
			self.insert(&key, &value)?;
		}

		Ok(())
	}

	fn remove(&self, key: &[u8]) -> Result<()>;

	#[allow(dead_code)]
	fn remove_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		for key in iter {
			self.remove(&key)?;
		}

		Ok(())
	}

	fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a>;

	fn iter_from<'a>(&'a self, from: &[u8], backwards: bool) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a>;

	fn increment(&self, key: &[u8]) -> Result<Vec<u8>>;
	fn increment_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		for key in iter {
			self.increment(&key)?;
		}

		Ok(())
	}

	fn scan_prefix<'a>(&'a self, prefix: Vec<u8>) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a>;

	fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

	fn clear(&self) -> Result<()> {
		for (key, _) in self.iter() {
			self.remove(&key)?;
		}

		Ok(())
	}
}

pub struct Cork {
	db: Arc<dyn KeyValueDatabaseEngine>,
	flush: bool,
	sync: bool,
}

impl Cork {
	pub(crate) fn new(db: &Arc<dyn KeyValueDatabaseEngine>, flush: bool, sync: bool) -> Self {
		db.cork().unwrap();
		Cork {
			db: db.clone(),
			flush,
			sync,
		}
	}
}

impl Drop for Cork {
	fn drop(&mut self) {
		self.db.uncork().ok();
		if self.flush {
			self.db.flush().ok();
		}
		if self.sync {
			self.db.sync().ok();
		}
	}
}
