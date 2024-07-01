use std::sync::Arc;

use crate::Engine;

pub struct Cork {
	db: Arc<Engine>,
	flush: bool,
	sync: bool,
}

impl Cork {
	pub(super) fn new(db: &Arc<Engine>, flush: bool, sync: bool) -> Self {
		db.cork();
		Self {
			db: db.clone(),
			flush,
			sync,
		}
	}
}

impl Drop for Cork {
	fn drop(&mut self) {
		self.db.uncork();
		if self.flush {
			self.db.flush().ok();
		}
		if self.sync {
			self.db.sync().ok();
		}
	}
}
