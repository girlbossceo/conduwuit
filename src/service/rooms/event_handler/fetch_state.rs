use std::{
	collections::{hash_map, HashMap},
	sync::Arc,
};

use conduit::{debug, implement, warn, Err, Error, PduEvent, Result};
use futures::FutureExt;
use ruma::{
	api::federation::event::get_room_state_ids, events::StateEventType, EventId, RoomId, RoomVersionId, ServerName,
};

/// Call /state_ids to find out what the state at this pdu is. We trust the
/// server's response to some extend (sic), but we still do a lot of checks
/// on the events
#[implement(super::Service)]
#[tracing::instrument(skip(self, create_event, room_version_id))]
pub(super) async fn fetch_state(
	&self, origin: &ServerName, create_event: &PduEvent, room_id: &RoomId, room_version_id: &RoomVersionId,
	event_id: &EventId,
) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
	debug!("Fetching state ids");
	let res = self
		.services
		.sending
		.send_federation_request(
			origin,
			get_room_state_ids::v1::Request {
				room_id: room_id.to_owned(),
				event_id: (*event_id).to_owned(),
			},
		)
		.await
		.inspect_err(|e| warn!("Fetching state for event failed: {e}"))?;

	debug!("Fetching state events");
	let collect = res
		.pdu_ids
		.iter()
		.map(|x| Arc::from(&**x))
		.collect::<Vec<_>>();

	let state_vec = self
		.fetch_and_handle_outliers(origin, &collect, create_event, room_id, room_version_id)
		.boxed()
		.await;

	let mut state: HashMap<_, Arc<EventId>> = HashMap::with_capacity(state_vec.len());
	for (pdu, _) in state_vec {
		let state_key = pdu
			.state_key
			.clone()
			.ok_or_else(|| Error::bad_database("Found non-state pdu in state events."))?;

		let shortstatekey = self
			.services
			.short
			.get_or_create_shortstatekey(&pdu.kind.to_string().into(), &state_key)
			.await;

		match state.entry(shortstatekey) {
			hash_map::Entry::Vacant(v) => {
				v.insert(Arc::from(&*pdu.event_id));
			},
			hash_map::Entry::Occupied(_) => {
				return Err(Error::bad_database(
					"State event's type and state_key combination exists multiple times.",
				))
			},
		}
	}

	// The original create event must still be in the state
	let create_shortstatekey = self
		.services
		.short
		.get_shortstatekey(&StateEventType::RoomCreate, "")
		.await?;

	if state.get(&create_shortstatekey).map(AsRef::as_ref) != Some(&create_event.event_id) {
		return Err!(Database("Incoming event refers to wrong create event."));
	}

	Ok(Some(state))
}
