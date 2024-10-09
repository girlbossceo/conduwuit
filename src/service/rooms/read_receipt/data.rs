use std::{mem::size_of, sync::Arc};

use conduit::{
	utils,
	utils::{stream::TryIgnore, ReadyExt},
	Error, Result,
};
use database::{Deserialized, Map};
use futures::{Stream, StreamExt};
use ruma::{
	events::{receipt::ReceiptEvent, AnySyncEphemeralRoomEvent},
	serde::Raw,
	CanonicalJsonObject, OwnedUserId, RoomId, UserId,
};

use crate::{globals, Dep};

pub(super) struct Data {
	roomuserid_privateread: Arc<Map>,
	roomuserid_lastprivatereadupdate: Arc<Map>,
	services: Services,
	readreceiptid_readreceipt: Arc<Map>,
}

struct Services {
	globals: Dep<globals::Service>,
}

pub(super) type ReceiptItem = (OwnedUserId, u64, Raw<AnySyncEphemeralRoomEvent>);

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			roomuserid_privateread: db["roomuserid_privateread"].clone(),
			roomuserid_lastprivatereadupdate: db["roomuserid_lastprivatereadupdate"].clone(),
			readreceiptid_readreceipt: db["readreceiptid_readreceipt"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}
	}

	pub(super) async fn readreceipt_update(&self, user_id: &UserId, room_id: &RoomId, event: &ReceiptEvent) {
		type KeyVal<'a> = (&'a RoomId, u64, &'a UserId);

		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		let mut last_possible_key = prefix.clone();
		last_possible_key.extend_from_slice(&u64::MAX.to_be_bytes());

		// Remove old entry
		self.readreceiptid_readreceipt
			.rev_keys_from_raw(&last_possible_key)
			.ignore_err()
			.ready_take_while(|(r, ..): &KeyVal<'_>| *r == room_id)
			.ready_filter_map(|(r, c, u): KeyVal<'_>| (u == user_id).then_some((r, c, u)))
			.ready_for_each(|old: KeyVal<'_>| {
				// This is the old room_latest
				self.readreceiptid_readreceipt.del(&old);
			})
			.await;

		let mut room_latest_id = prefix;
		room_latest_id.extend_from_slice(&self.services.globals.next_count().unwrap().to_be_bytes());
		room_latest_id.push(0xFF);
		room_latest_id.extend_from_slice(user_id.as_bytes());

		self.readreceiptid_readreceipt.insert(
			&room_latest_id,
			&serde_json::to_vec(event).expect("EduEvent::to_string always works"),
		);
	}

	pub(super) fn readreceipts_since<'a>(
		&'a self, room_id: &'a RoomId, since: u64,
	) -> impl Stream<Item = ReceiptItem> + Send + 'a {
		let after_since = since.saturating_add(1); // +1 so we don't send the event at since
		let first_possible_edu = (room_id, after_since);

		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);
		let prefix2 = prefix.clone();

		self.readreceiptid_readreceipt
			.stream_from_raw(&first_possible_edu)
			.ignore_err()
			.ready_take_while(move |(k, _)| k.starts_with(&prefix2))
			.map(move |(k, v)| {
				let count_offset = prefix.len().saturating_add(size_of::<u64>());
				let user_id_offset = count_offset.saturating_add(1);

				let count = utils::u64_from_bytes(&k[prefix.len()..count_offset])
					.map_err(|_| Error::bad_database("Invalid readreceiptid count in db."))?;

				let user_id_str = utils::string_from_bytes(&k[user_id_offset..])
					.map_err(|_| Error::bad_database("Invalid readreceiptid userid bytes in db."))?;

				let user_id = UserId::parse(user_id_str)
					.map_err(|_| Error::bad_database("Invalid readreceiptid userid in db."))?;

				let mut json = serde_json::from_slice::<CanonicalJsonObject>(v)
					.map_err(|_| Error::bad_database("Read receipt in roomlatestid_roomlatest is invalid json."))?;

				json.remove("room_id");

				let event = Raw::from_json(serde_json::value::to_raw_value(&json)?);

				Ok((user_id, count, event))
			})
			.ignore_err()
	}

	pub(super) fn private_read_set(&self, room_id: &RoomId, user_id: &UserId, count: u64) {
		let mut key = room_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(user_id.as_bytes());

		self.roomuserid_privateread
			.insert(&key, &count.to_be_bytes());

		self.roomuserid_lastprivatereadupdate
			.insert(&key, &self.services.globals.next_count().unwrap().to_be_bytes());
	}

	pub(super) async fn private_read_get(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
		let key = (room_id, user_id);
		self.roomuserid_privateread.qry(&key).await.deserialized()
	}

	pub(super) async fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		let key = (room_id, user_id);
		self.roomuserid_lastprivatereadupdate
			.qry(&key)
			.await
			.deserialized()
			.unwrap_or(0)
	}
}
