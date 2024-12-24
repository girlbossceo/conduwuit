use std::{
	borrow::Borrow,
	collections::{HashMap, HashSet},
	sync::Arc,
};

use conduwuit::{
	debug, err, implement,
	result::LogErr,
	utils::stream::{BroadbandExt, IterStream},
	PduEvent, Result,
};
use futures::{FutureExt, StreamExt};
use ruma::{state_res::StateMap, EventId, RoomId, RoomVersionId};

// TODO: if we know the prev_events of the incoming event we can avoid the
#[implement(super::Service)]
// request and build the state from a known point and resolve if > 1 prev_event
#[tracing::instrument(skip_all, name = "state")]
pub(super) async fn state_at_incoming_degree_one(
	&self,
	incoming_pdu: &Arc<PduEvent>,
) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
	let prev_event = &*incoming_pdu.prev_events[0];
	let Ok(prev_event_sstatehash) = self
		.services
		.state_accessor
		.pdu_shortstatehash(prev_event)
		.await
	else {
		return Ok(None);
	};

	let Ok(mut state) = self
		.services
		.state_accessor
		.state_full_ids(prev_event_sstatehash)
		.await
		.log_err()
	else {
		return Ok(None);
	};

	debug!("Using cached state");
	let prev_pdu = self
		.services
		.timeline
		.get_pdu(prev_event)
		.await
		.map_err(|e| err!(Database("Could not find prev event, but we know the state: {e:?}")))?;

	if let Some(state_key) = &prev_pdu.state_key {
		let shortstatekey = self
			.services
			.short
			.get_or_create_shortstatekey(&prev_pdu.kind.to_string().into(), state_key)
			.await;

		state.insert(shortstatekey, Arc::from(prev_event));
		// Now it's the state after the pdu
	}

	debug_assert!(!state.is_empty(), "should be returning None for empty HashMap result");

	Ok(Some(state))
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "state")]
pub(super) async fn state_at_incoming_resolved(
	&self,
	incoming_pdu: &Arc<PduEvent>,
	room_id: &RoomId,
	room_version_id: &RoomVersionId,
) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
	debug!("Calculating state at event using state res");
	let mut extremity_sstatehashes = HashMap::with_capacity(incoming_pdu.prev_events.len());

	let mut okay = true;
	for prev_eventid in &incoming_pdu.prev_events {
		let Ok(prev_event) = self.services.timeline.get_pdu(prev_eventid).await else {
			okay = false;
			break;
		};

		let Ok(sstatehash) = self
			.services
			.state_accessor
			.pdu_shortstatehash(prev_eventid)
			.await
		else {
			okay = false;
			break;
		};

		extremity_sstatehashes.insert(sstatehash, prev_event);
	}

	if !okay {
		return Ok(None);
	}

	let mut fork_states = Vec::with_capacity(extremity_sstatehashes.len());
	let mut auth_chain_sets = Vec::with_capacity(extremity_sstatehashes.len());
	for (sstatehash, prev_event) in extremity_sstatehashes {
		let Ok(mut leaf_state) = self
			.services
			.state_accessor
			.state_full_ids(sstatehash)
			.await
		else {
			continue;
		};

		if let Some(state_key) = &prev_event.state_key {
			let shortstatekey = self
				.services
				.short
				.get_or_create_shortstatekey(&prev_event.kind.to_string().into(), state_key)
				.await;

			let event_id = &prev_event.event_id;
			leaf_state.insert(shortstatekey, event_id.clone());
			// Now it's the state after the pdu
		}

		let mut state = StateMap::with_capacity(leaf_state.len());
		let mut starting_events = Vec::with_capacity(leaf_state.len());
		for (k, id) in &leaf_state {
			if let Ok((ty, st_key)) = self
				.services
				.short
				.get_statekey_from_short(*k)
				.await
				.log_err()
			{
				// FIXME: Undo .to_string().into() when StateMap
				//        is updated to use StateEventType
				state.insert((ty.to_string().into(), st_key), id.clone());
			}

			starting_events.push(id.borrow());
		}

		let auth_chain: HashSet<Arc<EventId>> = self
			.services
			.auth_chain
			.get_event_ids(room_id, starting_events.into_iter())
			.await?
			.into_iter()
			.collect();

		auth_chain_sets.push(auth_chain);
		fork_states.push(state);
	}

	let Ok(new_state) = self
		.state_resolution(room_version_id, &fork_states, &auth_chain_sets)
		.boxed()
		.await
	else {
		return Ok(None);
	};

	new_state
		.iter()
		.stream()
		.broad_then(|((event_type, state_key), event_id)| {
			self.services
				.short
				.get_or_create_shortstatekey(event_type, state_key)
				.map(move |shortstatekey| (shortstatekey, event_id.clone()))
		})
		.collect()
		.map(Some)
		.map(Ok)
		.await
}
