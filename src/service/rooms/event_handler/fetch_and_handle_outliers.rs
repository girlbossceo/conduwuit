use std::{
	collections::{hash_map, BTreeMap, HashSet},
	sync::Arc,
	time::Instant,
};

use conduwuit::{
	debug, debug_error, implement, info, pdu, trace,
	utils::math::continue_exponential_backoff_secs, warn, PduEvent,
};
use futures::TryFutureExt;
use ruma::{
	api::federation::event::get_event, CanonicalJsonValue, OwnedEventId, RoomId, RoomVersionId,
	ServerName,
};

/// Find the event and auth it. Once the event is validated (steps 1 - 8)
/// it is appended to the outliers Tree.
///
/// Returns pdu and if we fetched it over federation the raw json.
///
/// a. Look in the main timeline (pduid_pdu tree)
/// b. Look at outlier pdu tree
/// c. Ask origin server over federation
/// d. TODO: Ask other servers over federation?
#[implement(super::Service)]
pub(super) async fn fetch_and_handle_outliers<'a>(
	&self,
	origin: &'a ServerName,
	events: &'a [OwnedEventId],
	create_event: &'a PduEvent,
	room_id: &'a RoomId,
	room_version_id: &'a RoomVersionId,
) -> Vec<(Arc<PduEvent>, Option<BTreeMap<String, CanonicalJsonValue>>)> {
	let back_off = |id| match self
		.services
		.globals
		.bad_event_ratelimiter
		.write()
		.expect("locked")
		.entry(id)
	{
		| hash_map::Entry::Vacant(e) => {
			e.insert((Instant::now(), 1));
		},
		| hash_map::Entry::Occupied(mut e) => {
			*e.get_mut() = (Instant::now(), e.get().1.saturating_add(1));
		},
	};

	let mut events_with_auth_events = Vec::with_capacity(events.len());
	for id in events {
		// a. Look in the main timeline (pduid_pdu tree)
		// b. Look at outlier pdu tree
		// (get_pdu_json checks both)
		if let Ok(local_pdu) = self.services.timeline.get_pdu(id).map_ok(Arc::new).await {
			trace!("Found {id} in db");
			events_with_auth_events.push((id, Some(local_pdu), vec![]));
			continue;
		}

		// c. Ask origin server over federation
		// We also handle its auth chain here so we don't get a stack overflow in
		// handle_outlier_pdu.
		let mut todo_auth_events = vec![id.clone()];
		let mut events_in_reverse_order = Vec::with_capacity(todo_auth_events.len());
		let mut events_all = HashSet::with_capacity(todo_auth_events.len());
		while let Some(next_id) = todo_auth_events.pop() {
			if let Some((time, tries)) = self
				.services
				.globals
				.bad_event_ratelimiter
				.read()
				.expect("locked")
				.get(&*next_id)
			{
				// Exponential backoff
				const MIN_DURATION: u64 = 5 * 60;
				const MAX_DURATION: u64 = 60 * 60 * 24;
				if continue_exponential_backoff_secs(
					MIN_DURATION,
					MAX_DURATION,
					time.elapsed(),
					*tries,
				) {
					info!("Backing off from {next_id}");
					continue;
				}
			}

			if events_all.contains(&next_id) {
				continue;
			}

			if self.services.timeline.pdu_exists(&next_id).await {
				trace!("Found {next_id} in db");
				continue;
			}

			debug!("Fetching {next_id} over federation.");
			match self
				.services
				.sending
				.send_federation_request(origin, get_event::v1::Request {
					event_id: (*next_id).to_owned(),
					include_unredacted_content: None,
				})
				.await
			{
				| Ok(res) => {
					debug!("Got {next_id} over federation");
					let Ok((calculated_event_id, value)) =
						pdu::gen_event_id_canonical_json(&res.pdu, room_version_id)
					else {
						back_off((*next_id).to_owned());
						continue;
					};

					if calculated_event_id != *next_id {
						warn!(
							"Server didn't return event id we requested: requested: {next_id}, \
							 we got {calculated_event_id}. Event: {:?}",
							&res.pdu
						);
					}

					if let Some(auth_events) = value
						.get("auth_events")
						.and_then(CanonicalJsonValue::as_array)
					{
						for auth_event in auth_events {
							if let Ok(auth_event) =
								serde_json::from_value::<OwnedEventId>(auth_event.clone().into())
							{
								todo_auth_events.push(auth_event);
							} else {
								warn!("Auth event id is not valid");
							}
						}
					} else {
						warn!("Auth event list invalid");
					}

					events_in_reverse_order.push((next_id.clone(), value));
					events_all.insert(next_id);
				},
				| Err(e) => {
					debug_error!("Failed to fetch event {next_id}: {e}");
					back_off((*next_id).to_owned());
				},
			}
		}
		events_with_auth_events.push((id, None, events_in_reverse_order));
	}

	let mut pdus = Vec::with_capacity(events_with_auth_events.len());
	for (id, local_pdu, events_in_reverse_order) in events_with_auth_events {
		// a. Look in the main timeline (pduid_pdu tree)
		// b. Look at outlier pdu tree
		// (get_pdu_json checks both)
		if let Some(local_pdu) = local_pdu {
			trace!("Found {id} in db");
			pdus.push((local_pdu.clone(), None));
		}

		for (next_id, value) in events_in_reverse_order.into_iter().rev() {
			if let Some((time, tries)) = self
				.services
				.globals
				.bad_event_ratelimiter
				.read()
				.expect("locked")
				.get(&*next_id)
			{
				// Exponential backoff
				const MIN_DURATION: u64 = 5 * 60;
				const MAX_DURATION: u64 = 60 * 60 * 24;
				if continue_exponential_backoff_secs(
					MIN_DURATION,
					MAX_DURATION,
					time.elapsed(),
					*tries,
				) {
					debug!("Backing off from {next_id}");
					continue;
				}
			}

			match Box::pin(self.handle_outlier_pdu(
				origin,
				create_event,
				&next_id,
				room_id,
				value.clone(),
				true,
			))
			.await
			{
				| Ok((pdu, json)) =>
					if next_id == *id {
						pdus.push((pdu, Some(json)));
					},
				| Err(e) => {
					warn!("Authentication of event {next_id} failed: {e:?}");
					back_off(next_id);
				},
			}
		}
	}
	pdus
}
