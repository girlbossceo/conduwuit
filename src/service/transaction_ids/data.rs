use ruma::{DeviceId, TransactionId, UserId};

use crate::Result;

pub(crate) trait Data: Send + Sync {
	fn add_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId, data: &[u8],
	) -> Result<()>;

	fn existing_txnid(
		&self, user_id: &UserId, device_id: Option<&DeviceId>, txn_id: &TransactionId,
	) -> Result<Option<Vec<u8>>>;
}
