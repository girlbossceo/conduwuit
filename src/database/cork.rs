use std::sync::Arc;

use super::KeyValueDatabaseEngine;

pub(crate) struct Cork {
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
