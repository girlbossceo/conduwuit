mod data;

use std::sync::Arc;

use conduit::Result;
use data::Data;
use ruma::RoomId;

pub struct Service {
	db: Data,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn index_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) {
		self.db.index_pdu(shortroomid, pdu_id, message_body);
	}

	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn deindex_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) {
		self.db.deindex_pdu(shortroomid, pdu_id, message_body);
	}

	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn search_pdus(&self, room_id: &RoomId, search_string: &str) -> Option<(Vec<Vec<u8>>, Vec<String>)> {
		self.db.search_pdus(room_id, search_string).await
	}
}
