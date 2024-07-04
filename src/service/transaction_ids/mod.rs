mod data;

use std::sync::Arc;

use conduit::Result;
use data::Data;
use ruma::{DeviceId, TransactionId, UserId};

pub struct Service {
	pub db: Data,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data::new(args.db),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
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
