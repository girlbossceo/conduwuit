mod data;

use std::sync::Arc;

use conduit::{Result, Server};
use data::Data;
use database::Database;
use ruma::{DeviceId, TransactionId, UserId};

pub struct Service {
	pub db: Data,
}

impl Service {
	pub fn build(_server: &Arc<Server>, db: &Arc<Database>) -> Result<Self> {
		Ok(Self {
			db: Data::new(db),
		})
	}

	pub fn add_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId, data: &[u8],
	) -> Result<()> {
		self.db.add_txnid(user_id, device_id, txn_id, data)
	}

	pub fn existing_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId,
	) -> Result<Option<database::Handle<'_>>> {
		self.db.existing_txnid(user_id, device_id, txn_id)
	}
}
