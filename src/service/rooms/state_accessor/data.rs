use std::{collections::HashMap, sync::Arc};

use conduit::{err, PduEvent, Result};
use database::{Deserialized, Map};
use futures::TryFutureExt;
use ruma::{events::StateEventType, EventId, RoomId};

use crate::{rooms, Dep};

pub(super) struct Data {
	eventid_shorteventid: Arc<Map>,
	shorteventid_shortstatehash: Arc<Map>,
	services: Services,
}

struct Services {
	short: Dep<rooms::short::Service>,
	state: Dep<rooms::state::Service>,
	state_compressor: Dep<rooms::state_compressor::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			eventid_shorteventid: db["eventid_shorteventid"].clone(),
			shorteventid_shortstatehash: db["shorteventid_shortstatehash"].clone(),
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state: args.depend::<rooms::state::Service>("rooms::state"),
				state_compressor: args.depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
		}
	}

	#[allow(unused_qualifications)] // async traits
	pub(super) async fn state_full_ids(&self, shortstatehash: u64) -> Result<HashMap<u64, Arc<EventId>>> {
		let full_state = self
			.services
			.state_compressor
			.load_shortstatehash_info(shortstatehash)
			.await
			.map_err(|e| err!(Database("Missing state IDs: {e}")))?
			.pop()
			.expect("there is always one layer")
			.1;

		let mut result = HashMap::new();
		let mut i: u8 = 0;
		for compressed in full_state.iter() {
			let parsed = self
				.services
				.state_compressor
				.parse_compressed_state_event(compressed)
				.await?;

			result.insert(parsed.0, parsed.1);

			i = i.wrapping_add(1);
			if i % 100 == 0 {
				tokio::task::yield_now().await;
			}
		}

		Ok(result)
	}

	#[allow(unused_qualifications)] // async traits
	pub(super) async fn state_full(
		&self, shortstatehash: u64,
	) -> Result<HashMap<(StateEventType, String), Arc<PduEvent>>> {
		let full_state = self
			.services
			.state_compressor
			.load_shortstatehash_info(shortstatehash)
			.await?
			.pop()
			.expect("there is always one layer")
			.1;

		let mut result = HashMap::new();
		let mut i: u8 = 0;
		for compressed in full_state.iter() {
			let (_, eventid) = self
				.services
				.state_compressor
				.parse_compressed_state_event(compressed)
				.await?;

			if let Ok(pdu) = self.services.timeline.get_pdu(&eventid).await {
				if let Some(state_key) = pdu.state_key.as_ref() {
					result.insert((pdu.kind.to_string().into(), state_key.clone()), pdu);
				}
			}

			i = i.wrapping_add(1);
			if i % 100 == 0 {
				tokio::task::yield_now().await;
			}
		}

		Ok(result)
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,`state_key`).
	#[allow(clippy::unused_self)]
	pub(super) async fn state_get_id(
		&self, shortstatehash: u64, event_type: &StateEventType, state_key: &str,
	) -> Result<Arc<EventId>> {
		let shortstatekey = self
			.services
			.short
			.get_shortstatekey(event_type, state_key)
			.await?;

		let full_state = self
			.services
			.state_compressor
			.load_shortstatehash_info(shortstatehash)
			.await
			.map_err(|e| err!(Database(error!(?event_type, ?state_key, "Missing state: {e:?}"))))?
			.pop()
			.expect("there is always one layer")
			.1;

		let compressed = full_state
			.iter()
			.find(|bytes| bytes.starts_with(&shortstatekey.to_be_bytes()))
			.ok_or(err!(Database("No shortstatekey in compressed state")))?;

		self.services
			.state_compressor
			.parse_compressed_state_event(compressed)
			.map_ok(|(_, id)| id)
			.map_err(|e| {
				err!(Database(error!(
					?event_type,
					?state_key,
					?shortstatekey,
					"Failed to parse compressed: {e:?}"
				)))
			})
			.await
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,`state_key`).
	pub(super) async fn state_get(
		&self, shortstatehash: u64, event_type: &StateEventType, state_key: &str,
	) -> Result<Arc<PduEvent>> {
		self.state_get_id(shortstatehash, event_type, state_key)
			.and_then(|event_id| async move { self.services.timeline.get_pdu(&event_id).await })
			.await
	}

	/// Returns the state hash for this pdu.
	pub(super) async fn pdu_shortstatehash(&self, event_id: &EventId) -> Result<u64> {
		self.eventid_shorteventid
			.get(event_id)
			.and_then(|shorteventid| self.shorteventid_shortstatehash.get(&shorteventid))
			.await
			.deserialized()
	}

	/// Returns the full room state.
	#[allow(unused_qualifications)] // async traits
	pub(super) async fn room_state_full(
		&self, room_id: &RoomId,
	) -> Result<HashMap<(StateEventType, String), Arc<PduEvent>>> {
		self.services
			.state
			.get_room_shortstatehash(room_id)
			.and_then(|shortstatehash| self.state_full(shortstatehash))
			.map_err(|e| err!(Database("Missing state for {room_id:?}: {e:?}")))
			.await
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,`state_key`).
	pub(super) async fn room_state_get_id(
		&self, room_id: &RoomId, event_type: &StateEventType, state_key: &str,
	) -> Result<Arc<EventId>> {
		self.services
			.state
			.get_room_shortstatehash(room_id)
			.and_then(|shortstatehash| self.state_get_id(shortstatehash, event_type, state_key))
			.await
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,`state_key`).
	pub(super) async fn room_state_get(
		&self, room_id: &RoomId, event_type: &StateEventType, state_key: &str,
	) -> Result<Arc<PduEvent>> {
		self.services
			.state
			.get_room_shortstatehash(room_id)
			.and_then(|shortstatehash| self.state_get(shortstatehash, event_type, state_key))
			.await
	}
}
