use std::{error::Error, sync::Arc};

use super::{Config, KvTree};
use crate::Result;

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
