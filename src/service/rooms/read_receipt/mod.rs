mod data;

use std::{collections::BTreeMap, sync::Arc};

use conduit::{debug, Result};
use futures::Stream;
use ruma::{
	events::{
		receipt::{ReceiptEvent, ReceiptEventContent},
		SyncEphemeralRoomEvent,
	},
	serde::Raw,
	RoomId, UserId,
};

use self::data::{Data, ReceiptItem};
use crate::{sending, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	sending: Dep<sending::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				sending: args.depend::<sending::Service>("sending"),
			},
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Replaces the previous read receipt.
	pub async fn readreceipt_update(&self, user_id: &UserId, room_id: &RoomId, event: &ReceiptEvent) {
		self.db.readreceipt_update(user_id, room_id, event).await;
		self.services
			.sending
			.flush_room(room_id)
			.await
			.expect("room flush failed");
	}

	/// Returns an iterator over the most recent read_receipts in a room that
	/// happened after the event with id `since`.
	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn readreceipts_since<'a>(
		&'a self, room_id: &'a RoomId, since: u64,
	) -> impl Stream<Item = ReceiptItem> + Send + 'a {
		self.db.readreceipts_since(room_id, since)
	}

	/// Sets a private read marker at `count`.
	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn private_read_set(&self, room_id: &RoomId, user_id: &UserId, count: u64) {
		self.db.private_read_set(room_id, user_id, count);
	}

	/// Returns the private read marker.
	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn private_read_get(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
		self.db.private_read_get(room_id, user_id).await
	}

	/// Returns the count of the last typing update in this room.
	#[inline]
	pub async fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		self.db.last_privateread_update(user_id, room_id).await
	}
}

#[must_use]
pub fn pack_receipts<I>(receipts: I) -> Raw<SyncEphemeralRoomEvent<ReceiptEventContent>>
where
	I: Iterator<Item = ReceiptItem>,
{
	let mut json = BTreeMap::new();
	for (_, _, value) in receipts {
		let receipt = serde_json::from_str::<SyncEphemeralRoomEvent<ReceiptEventContent>>(value.json().get());
		if let Ok(value) = receipt {
			for (event, receipt) in value.content {
				json.insert(event, receipt);
			}
		} else {
			debug!("failed to parse receipt: {:?}", receipt);
		}
	}
	let content = ReceiptEventContent::from_iter(json);

	Raw::from_json(
		serde_json::value::to_raw_value(&SyncEphemeralRoomEvent {
			content,
		})
		.expect("received valid json"),
	)
}
