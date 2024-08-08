use std::collections::HashSet;

use axum::extract::State;
use conduit::{err, error, Err};
use futures::StreamExt;
use ruma::{
	api::client::{context::get_context, filter::LazyLoadOptions},
	events::StateEventType,
};

use crate::{Result, Ruma};

/// # `GET /_matrix/client/r0/rooms/{roomId}/context`
///
/// Allows loading room history around an event.
///
/// - Only works if the user is joined (TODO: always allow, but only show events
///   if the user was joined, depending on history_visibility)
pub(crate) async fn get_context_route(
	State(services): State<crate::State>, body: Ruma<get_context::v3::Request>,
) -> Result<get_context::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let sender_device = body.sender_device.as_ref().expect("user is authenticated");

	// some clients, at least element, seem to require knowledge of redundant
	// members for "inline" profiles on the timeline to work properly
	let (lazy_load_enabled, lazy_load_send_redundant) = match &body.filter.lazy_load_options {
		LazyLoadOptions::Enabled {
			include_redundant_members,
		} => (true, *include_redundant_members),
		LazyLoadOptions::Disabled => (false, cfg!(feature = "element_hacks")),
	};

	let mut lazy_loaded = HashSet::new();

	let base_token = services
		.rooms
		.timeline
		.get_pdu_count(&body.event_id)
		.await
		.map_err(|_| err!(Request(NotFound("Base event id not found."))))?;

	let base_event = services
		.rooms
		.timeline
		.get_pdu(&body.event_id)
		.await
		.map_err(|_| err!(Request(NotFound("Base event not found."))))?;

	let room_id = &base_event.room_id;

	if !services
		.rooms
		.state_accessor
		.user_can_see_event(sender_user, room_id, &body.event_id)
		.await
	{
		return Err!(Request(Forbidden("You don't have permission to view this event.")));
	}

	if !services
		.rooms
		.lazy_loading
		.lazy_load_was_sent_before(sender_user, sender_device, room_id, &base_event.sender)
		.await || lazy_load_send_redundant
	{
		lazy_loaded.insert(base_event.sender.as_str().to_owned());
	}

	// Use limit or else 10, with maximum 100
	let limit = usize::try_from(body.limit).unwrap_or(10).min(100);

	let base_event = base_event.to_room_event();

	let events_before: Vec<_> = services
		.rooms
		.timeline
		.pdus_until(sender_user, room_id, base_token)
		.await?
		.take(limit / 2)
		.filter_map(|(count, pdu)| async move {
			services
				.rooms
				.state_accessor
				.user_can_see_event(sender_user, room_id, &pdu.event_id)
				.await
				.then_some((count, pdu))
		})
		.collect()
		.await;

	for (_, event) in &events_before {
		if !services
			.rooms
			.lazy_loading
			.lazy_load_was_sent_before(sender_user, sender_device, room_id, &event.sender)
			.await || lazy_load_send_redundant
		{
			lazy_loaded.insert(event.sender.as_str().to_owned());
		}
	}

	let start_token = events_before
		.last()
		.map_or_else(|| base_token.stringify(), |(count, _)| count.stringify());

	let events_before: Vec<_> = events_before
		.into_iter()
		.map(|(_, pdu)| pdu.to_room_event())
		.collect();

	let events_after: Vec<_> = services
		.rooms
		.timeline
		.pdus_after(sender_user, room_id, base_token)
		.await?
		.take(limit / 2)
		.filter_map(|(count, pdu)| async move {
			services
				.rooms
				.state_accessor
				.user_can_see_event(sender_user, room_id, &pdu.event_id)
				.await
				.then_some((count, pdu))
		})
		.collect()
		.await;

	for (_, event) in &events_after {
		if !services
			.rooms
			.lazy_loading
			.lazy_load_was_sent_before(sender_user, sender_device, room_id, &event.sender)
			.await || lazy_load_send_redundant
		{
			lazy_loaded.insert(event.sender.as_str().to_owned());
		}
	}

	let shortstatehash = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(
			events_after
				.last()
				.map_or(&*body.event_id, |(_, e)| &*e.event_id),
		)
		.await
		.map_or(
			services
				.rooms
				.state
				.get_room_shortstatehash(room_id)
				.await
				.expect("All rooms have state"),
			|hash| hash,
		);

	let state_ids = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await
		.map_err(|e| err!(Database("State not found: {e}")))?;

	let end_token = events_after
		.last()
		.map_or_else(|| base_token.stringify(), |(count, _)| count.stringify());

	let events_after: Vec<_> = events_after
		.into_iter()
		.map(|(_, pdu)| pdu.to_room_event())
		.collect();

	let mut state = Vec::with_capacity(state_ids.len());

	for (shortstatekey, id) in state_ids {
		let (event_type, state_key) = services
			.rooms
			.short
			.get_statekey_from_short(shortstatekey)
			.await?;

		if event_type != StateEventType::RoomMember {
			let Ok(pdu) = services.rooms.timeline.get_pdu(&id).await else {
				error!("Pdu in state not found: {id}");
				continue;
			};

			state.push(pdu.to_state_event());
		} else if !lazy_load_enabled || lazy_loaded.contains(&state_key) {
			let Ok(pdu) = services.rooms.timeline.get_pdu(&id).await else {
				error!("Pdu in state not found: {id}");
				continue;
			};

			state.push(pdu.to_state_event());
		}
	}

	Ok(get_context::v3::Response {
		start: Some(start_token),
		end: Some(end_token),
		events_before,
		event: Some(base_event),
		events_after,
		state,
	})
}
