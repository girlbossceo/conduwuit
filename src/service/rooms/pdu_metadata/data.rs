use std::{mem::size_of, sync::Arc};

use conduit::{
	result::LogErr,
	utils,
	utils::{stream::TryIgnore, ReadyExt},
	PduCount, PduEvent,
};
use database::Map;
use futures::{Stream, StreamExt};
use ruma::{EventId, RoomId, UserId};

use crate::{rooms, Dep};

pub(super) struct Data {
	tofrom_relation: Arc<Map>,
	referencedevents: Arc<Map>,
	softfailedeventids: Arc<Map>,
	services: Services,
}

struct Services {
	timeline: Dep<rooms::timeline::Service>,
}

pub(super) type PdusIterItem = (PduCount, PduEvent);

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			tofrom_relation: db["tofrom_relation"].clone(),
			referencedevents: db["referencedevents"].clone(),
			softfailedeventids: db["softfailedeventids"].clone(),
			services: Services {
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
		}
	}

	pub(super) fn add_relation(&self, from: u64, to: u64) {
		const BUFSIZE: usize = size_of::<u64>() * 2;

		let key: &[u64] = &[to, from];
		self.tofrom_relation.aput_raw::<BUFSIZE, _, _>(key, []);
	}

	pub(super) fn relations_until<'a>(
		&'a self, user_id: &'a UserId, shortroomid: u64, target: u64, until: PduCount,
	) -> impl Stream<Item = PdusIterItem> + Send + 'a + '_ {
		let prefix = target.to_be_bytes().to_vec();
		let mut current = prefix.clone();
		let count_raw = match until {
			PduCount::Normal(x) => x.saturating_sub(1),
			PduCount::Backfilled(x) => {
				current.extend_from_slice(&0_u64.to_be_bytes());
				u64::MAX.saturating_sub(x).saturating_sub(1)
			},
		};
		current.extend_from_slice(&count_raw.to_be_bytes());

		self.tofrom_relation
			.rev_raw_keys_from(&current)
			.ignore_err()
			.ready_take_while(move |key| key.starts_with(&prefix))
			.map(|to_from| utils::u64_from_u8(&to_from[(size_of::<u64>())..]))
			.filter_map(move |from| async move {
				let mut pduid = shortroomid.to_be_bytes().to_vec();
				pduid.extend_from_slice(&from.to_be_bytes());
				let mut pdu = self.services.timeline.get_pdu_from_id(&pduid).await.ok()?;

				if pdu.sender != user_id {
					pdu.remove_transaction_id().log_err().ok();
				}

				Some((PduCount::Normal(from), pdu))
			})
	}

	pub(super) fn mark_as_referenced(&self, room_id: &RoomId, event_ids: &[Arc<EventId>]) {
		for prev in event_ids {
			let key = (room_id, prev);
			self.referencedevents.put_raw(key, []);
		}
	}

	pub(super) async fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> bool {
		let key = (room_id, event_id);
		self.referencedevents.qry(&key).await.is_ok()
	}

	pub(super) fn mark_event_soft_failed(&self, event_id: &EventId) { self.softfailedeventids.insert(event_id, []); }

	pub(super) async fn is_event_soft_failed(&self, event_id: &EventId) -> bool {
		self.softfailedeventids.get(event_id).await.is_ok()
	}
}
