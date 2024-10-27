use std::iter::once;

use axum::extract::State;
use conduit::{
	err, error,
	utils::{future::TryExtExt, stream::ReadyExt, IterStream},
	Err, Result,
};
use futures::{future::try_join, StreamExt, TryFutureExt};
use ruma::{
	api::client::{context::get_context, filter::LazyLoadOptions},
	events::StateEventType,
	UserId,
};

use crate::{
	client::message::{event_filter, ignored_filter, update_lazy, visibility_filter, LazySet},
	Ruma,
};

const LIMIT_MAX: usize = 100;
const LIMIT_DEFAULT: usize = 10;

/// # `GET /_matrix/client/r0/rooms/{roomId}/context/{eventId}`
///
/// Allows loading room history around an event.
///
/// - Only works if the user is joined (TODO: always allow, but only show events
///   if the user was joined, depending on history_visibility)
pub(crate) async fn get_context_route(
	State(services): State<crate::State>, body: Ruma<get_context::v3::Request>,
) -> Result<get_context::v3::Response> {
	let filter = &body.filter;
	let sender = body.sender();
	let (sender_user, _) = sender;

	// Use limit or else 10, with maximum 100
	let limit: usize = body
		.limit
		.try_into()
		.unwrap_or(LIMIT_DEFAULT)
		.min(LIMIT_MAX);

	// some clients, at least element, seem to require knowledge of redundant
	// members for "inline" profiles on the timeline to work properly
	let lazy_load_enabled = matches!(filter.lazy_load_options, LazyLoadOptions::Enabled { .. });

	let lazy_load_redundant = if let LazyLoadOptions::Enabled {
		include_redundant_members,
	} = filter.lazy_load_options
	{
		include_redundant_members
	} else {
		false
	};

	let base_token = services
		.rooms
		.timeline
		.get_pdu_count(&body.event_id)
		.map_err(|_| err!(Request(NotFound("Event not found."))));

	let base_event = services
		.rooms
		.timeline
		.get_pdu(&body.event_id)
		.map_err(|_| err!(Request(NotFound("Base event not found."))));

	let (base_token, base_event) = try_join(base_token, base_event).await?;

	let room_id = &base_event.room_id;

	if !services
		.rooms
		.state_accessor
		.user_can_see_event(sender_user, room_id, &body.event_id)
		.await
	{
		return Err!(Request(Forbidden("You don't have permission to view this event.")));
	}

	let events_before: Vec<_> = services
		.rooms
		.timeline
		.pdus_until(sender_user, room_id, base_token)
		.await?
		.ready_filter_map(|item| event_filter(item, filter))
		.filter_map(|item| ignored_filter(&services, item, sender_user))
		.filter_map(|item| visibility_filter(&services, item, sender_user))
		.take(limit / 2)
		.collect()
		.await;

	let events_after: Vec<_> = services
		.rooms
		.timeline
		.pdus_after(sender_user, room_id, base_token)
		.await?
		.ready_filter_map(|item| event_filter(item, filter))
		.filter_map(|item| ignored_filter(&services, item, sender_user))
		.filter_map(|item| visibility_filter(&services, item, sender_user))
		.take(limit / 2)
		.collect()
		.await;

	let lazy = once(&(base_token, (*base_event).clone()))
		.chain(events_before.iter())
		.chain(events_after.iter())
		.stream()
		.fold(LazySet::new(), |lazy, item| {
			update_lazy(&services, room_id, sender, lazy, item, lazy_load_redundant)
		})
		.await;

	let state_id = events_after
		.last()
		.map_or(body.event_id.as_ref(), |(_, e)| e.event_id.as_ref());

	let shortstatehash = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(state_id)
		.or_else(|_| services.rooms.state.get_room_shortstatehash(room_id))
		.await
		.map_err(|e| err!(Database("State hash not found: {e}")))?;

	let state_ids = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await
		.map_err(|e| err!(Database("State not found: {e}")))?;

	let lazy = &lazy;
	let state: Vec<_> = state_ids
		.iter()
		.stream()
		.filter_map(|(shortstatekey, event_id)| {
			services
				.rooms
				.short
				.get_statekey_from_short(*shortstatekey)
				.map_ok(move |(event_type, state_key)| (event_type, state_key, event_id))
				.ok()
		})
		.filter_map(|(event_type, state_key, event_id)| async move {
			if lazy_load_enabled && event_type == StateEventType::RoomMember {
				let user_id: &UserId = state_key.as_str().try_into().ok()?;
				if !lazy.contains(user_id) {
					return None;
				}
			}

			services
				.rooms
				.timeline
				.get_pdu(event_id)
				.await
				.inspect_err(|_| error!("Pdu in state not found: {event_id}"))
				.map(|pdu| pdu.to_state_event())
				.ok()
		})
		.collect()
		.await;

	Ok(get_context::v3::Response {
		event: Some(base_event.to_room_event()),

		start: events_before
			.last()
			.map_or_else(|| base_token.stringify(), |(count, _)| count.stringify())
			.into(),

		end: events_after
			.last()
			.map_or_else(|| base_token.stringify(), |(count, _)| count.stringify())
			.into(),

		events_before: events_before
			.into_iter()
			.map(|(_, pdu)| pdu.to_room_event())
			.collect(),

		events_after: events_after
			.into_iter()
			.map(|(_, pdu)| pdu.to_room_event())
			.collect(),

		state,
	})
}
