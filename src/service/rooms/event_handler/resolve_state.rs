use std::{
	borrow::Borrow,
	collections::{HashMap, HashSet},
	sync::Arc,
};

use conduit::{debug, err, implement, utils::IterStream, Result};
use futures::{FutureExt, StreamExt, TryFutureExt};
use ruma::{
	state_res::{self, StateMap},
	EventId, RoomId, RoomVersionId,
};

use crate::rooms::state_compressor::CompressedStateEvent;

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "resolve")]
pub async fn resolve_state(
	&self, room_id: &RoomId, room_version_id: &RoomVersionId, incoming_state: HashMap<u64, Arc<EventId>>,
) -> Result<Arc<HashSet<CompressedStateEvent>>> {
	debug!("Loading current room state ids");
	let current_sstatehash = self
		.services
		.state
		.get_room_shortstatehash(room_id)
		.await
		.map_err(|e| err!(Database(error!("No state for {room_id:?}: {e:?}"))))?;

	let current_state_ids = self
		.services
		.state_accessor
		.state_full_ids(current_sstatehash)
		.await?;

	let fork_states = [current_state_ids, incoming_state];
	let mut auth_chain_sets = Vec::with_capacity(fork_states.len());
	for state in &fork_states {
		let starting_events: Vec<&EventId> = state.values().map(Borrow::borrow).collect();

		let auth_chain: HashSet<Arc<EventId>> = self
			.services
			.auth_chain
			.get_event_ids(room_id, &starting_events)
			.await?
			.into_iter()
			.collect();

		auth_chain_sets.push(auth_chain);
	}

	debug!("Loading fork states");
	let fork_states: Vec<StateMap<Arc<EventId>>> = fork_states
		.into_iter()
		.stream()
		.then(|fork_state| {
			fork_state
				.into_iter()
				.stream()
				.filter_map(|(k, id)| {
					self.services
						.short
						.get_statekey_from_short(k)
						.map_ok_or_else(|_| None, move |(ty, st_key)| Some(((ty, st_key), id)))
				})
				.collect()
		})
		.collect()
		.boxed()
		.await;

	debug!("Resolving state");
	let lock = self.services.globals.stateres_mutex.lock();

	let event_fetch = |event_id| self.event_fetch(event_id);
	let event_exists = |event_id| self.event_exists(event_id);
	let state = state_res::resolve(room_version_id, &fork_states, &auth_chain_sets, &event_fetch, &event_exists)
		.await
		.map_err(|e| err!(Database(error!("State resolution failed: {e:?}"))))?;

	drop(lock);

	debug!("State resolution done. Compressing state");
	let mut new_room_state = HashSet::new();
	for ((event_type, state_key), event_id) in state {
		let shortstatekey = self
			.services
			.short
			.get_or_create_shortstatekey(&event_type.to_string().into(), &state_key)
			.await;

		let compressed = self
			.services
			.state_compressor
			.compress_state_event(shortstatekey, &event_id)
			.await;

		new_room_state.insert(compressed);
	}

	Ok(Arc::new(new_room_state))
}
