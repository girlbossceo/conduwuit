use std::{future::Future, pin::Pin};

use crate::Result;

pub(crate) trait KvTree: Send + Sync {
	fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

	#[allow(dead_code)]
	fn multi_get(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
		let mut ret: Vec<Option<Vec<u8>>> = Vec::with_capacity(keys.len());
		for key in keys {
			ret.push(self.get(key)?);
		}

		Ok(ret)
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
