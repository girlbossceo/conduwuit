use std::sync::Arc;

use conduit::Result;
use database::{Database, Map};
use ruma::{DeviceId, TransactionId, UserId};

pub struct Data {
	userdevicetxnid_response: Arc<Map>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			userdevicetxnid_response: db["userdevicetxnid_response"].clone(),
		}
	}

	pub(super) fn add_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId, data: &[u8],
	) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(device_id.map(DeviceId::as_bytes).unwrap_or_default());
		key.push(0xFF);
		key.extend_from_slice(txn_id.as_bytes());

		self.userdevicetxnid_response.insert(&key, data)?;

		Ok(())
	}

	pub(super) fn existing_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId,
	) -> Result<Option<Vec<u8>>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(device_id.map(DeviceId::as_bytes).unwrap_or_default());
		key.push(0xFF);
		key.extend_from_slice(txn_id.as_bytes());

		// If there's no entry, this is a new transaction
		self.userdevicetxnid_response.get(&key)
	}
}
