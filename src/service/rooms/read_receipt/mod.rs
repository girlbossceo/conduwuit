mod data;

use std::{collections::BTreeMap, sync::Arc};

use conduit::{debug, Result};
use data::Data;
use ruma::{
	events::{
		receipt::{ReceiptEvent, ReceiptEventContent},
		AnySyncEphemeralRoomEvent, SyncEphemeralRoomEvent,
	},
	serde::Raw,
	OwnedUserId, RoomId, UserId,
};

use crate::{sending, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	sending: Dep<sending::Service>,
}

type AnySyncEphemeralRoomEventIter<'a> =
	Box<dyn Iterator<Item = Result<(OwnedUserId, u64, Raw<AnySyncEphemeralRoomEvent>)>> + 'a>;

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
	pub fn readreceipt_update(&self, user_id: &UserId, room_id: &RoomId, event: &ReceiptEvent) -> Result<()> {
		self.db.readreceipt_update(user_id, room_id, event)?;
		self.services.sending.flush_room(room_id)?;

		Ok(())
	}

	/// Returns an iterator over the most recent read_receipts in a room that
	/// happened after the event with id `since`.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn readreceipts_since<'a>(
		&'a self, room_id: &RoomId, since: u64,
	) -> impl Iterator<Item = Result<(OwnedUserId, u64, Raw<AnySyncEphemeralRoomEvent>)>> + 'a {
		self.db.readreceipts_since(room_id, since)
	}

	/// Sets a private read marker at `count`.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn private_read_set(&self, room_id: &RoomId, user_id: &UserId, count: u64) -> Result<()> {
		self.db.private_read_set(room_id, user_id, count)
	}

	/// Returns the private read marker.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn private_read_get(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<u64>> {
		self.db.private_read_get(room_id, user_id)
	}

	/// Returns the count of the last typing update in this room.
	pub fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> Result<u64> {
		self.db.last_privateread_update(user_id, room_id)
	}
}

#[must_use]
pub fn pack_receipts(receipts: AnySyncEphemeralRoomEventIter<'_>) -> Raw<SyncEphemeralRoomEvent<ReceiptEventContent>> {
	let mut json = BTreeMap::new();
	for (_user, _count, value) in receipts.flatten() {
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
