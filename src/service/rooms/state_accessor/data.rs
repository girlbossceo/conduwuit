use std::{collections::HashMap, sync::Arc};

use conduit::{utils, Error, PduEvent, Result};
use database::Map;
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
			.load_shortstatehash_info(shortstatehash)?
			.pop()
			.expect("there is always one layer")
			.1;
		let mut result = HashMap::new();
		let mut i: u8 = 0;
		for compressed in full_state.iter() {
			let parsed = self
				.services
				.state_compressor
				.parse_compressed_state_event(compressed)?;
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
			.load_shortstatehash_info(shortstatehash)?
			.pop()
			.expect("there is always one layer")
			.1;

		let mut result = HashMap::new();
		let mut i: u8 = 0;
		for compressed in full_state.iter() {
			let (_, eventid) = self
				.services
				.state_compressor
				.parse_compressed_state_event(compressed)?;
			if let Some(pdu) = self.services.timeline.get_pdu(&eventid)? {
				result.insert(
					(
						pdu.kind.to_string().into(),
						pdu.state_key
							.as_ref()
							.ok_or_else(|| Error::bad_database("State event has no state key."))?
							.clone(),
					),
					pdu,
				);
			}

			i = i.wrapping_add(1);
			if i % 100 == 0 {
				tokio::task::yield_now().await;
			}
		}

		Ok(result)
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,
	/// `state_key`).
	#[allow(clippy::unused_self)]
	pub(super) fn state_get_id(
		&self, shortstatehash: u64, event_type: &StateEventType, state_key: &str,
	) -> Result<Option<Arc<EventId>>> {
		let Some(shortstatekey) = self
			.services
			.short
			.get_shortstatekey(event_type, state_key)?
		else {
			return Ok(None);
		};
		let full_state = self
			.services
			.state_compressor
			.load_shortstatehash_info(shortstatehash)?
			.pop()
			.expect("there is always one layer")
			.1;
		Ok(full_state
			.iter()
			.find(|bytes| bytes.starts_with(&shortstatekey.to_be_bytes()))
			.and_then(|compressed| {
				self.services
					.state_compressor
					.parse_compressed_state_event(compressed)
					.ok()
					.map(|(_, id)| id)
			}))
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,
	/// `state_key`).
	pub(super) fn state_get(
		&self, shortstatehash: u64, event_type: &StateEventType, state_key: &str,
	) -> Result<Option<Arc<PduEvent>>> {
		self.state_get_id(shortstatehash, event_type, state_key)?
			.map_or(Ok(None), |event_id| self.services.timeline.get_pdu(&event_id))
	}

	/// Returns the state hash for this pdu.
	pub(super) fn pdu_shortstatehash(&self, event_id: &EventId) -> Result<Option<u64>> {
		self.eventid_shorteventid
			.get(event_id.as_bytes())?
			.map_or(Ok(None), |shorteventid| {
				self.shorteventid_shortstatehash
					.get(&shorteventid)?
					.map(|bytes| {
						utils::u64_from_bytes(&bytes).map_err(|_| {
							Error::bad_database("Invalid shortstatehash bytes in shorteventid_shortstatehash")
						})
					})
					.transpose()
			})
	}

	/// Returns the full room state.
	#[allow(unused_qualifications)] // async traits
	pub(super) async fn room_state_full(
		&self, room_id: &RoomId,
	) -> Result<HashMap<(StateEventType, String), Arc<PduEvent>>> {
		if let Some(current_shortstatehash) = self.services.state.get_room_shortstatehash(room_id)? {
			self.state_full(current_shortstatehash).await
		} else {
			Ok(HashMap::new())
		}
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,
	/// `state_key`).
	pub(super) fn room_state_get_id(
		&self, room_id: &RoomId, event_type: &StateEventType, state_key: &str,
	) -> Result<Option<Arc<EventId>>> {
		if let Some(current_shortstatehash) = self.services.state.get_room_shortstatehash(room_id)? {
			self.state_get_id(current_shortstatehash, event_type, state_key)
		} else {
			Ok(None)
		}
	}

	/// Returns a single PDU from `room_id` with key (`event_type`,
	/// `state_key`).
	pub(super) fn room_state_get(
		&self, room_id: &RoomId, event_type: &StateEventType, state_key: &str,
	) -> Result<Option<Arc<PduEvent>>> {
		if let Some(current_shortstatehash) = self.services.state.get_room_shortstatehash(room_id)? {
			self.state_get(current_shortstatehash, event_type, state_key)
		} else {
			Ok(None)
		}
	}
}
