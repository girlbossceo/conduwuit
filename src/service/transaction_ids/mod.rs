mod data;

pub(crate) use data::Data;
use ruma::{DeviceId, TransactionId, UserId};

use crate::Result;

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
}

impl Service {
	pub(crate) fn add_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId, data: &[u8],
	) -> Result<()> {
		self.db.add_txnid(user_id, device_id, txn_id, data)
	}

	pub(crate) fn existing_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId,
	) -> Result<Option<Vec<u8>>> {
		self.db.existing_txnid(user_id, device_id, txn_id)
	}
}
