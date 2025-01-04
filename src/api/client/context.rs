use axum::extract::State;
use conduwuit::{
	at, err, ref_at,
	utils::{
		future::TryExtExt,
		stream::{BroadbandExt, ReadyExt, TryIgnore, WidebandExt},
		IterStream,
	},
	Err, PduEvent, Result,
};
use futures::{join, try_join, FutureExt, StreamExt, TryFutureExt};
use ruma::{
	api::client::{context::get_context, filter::LazyLoadOptions},
	events::StateEventType,
	OwnedEventId, UserId,
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
	State(services): State<crate::State>,
	body: Ruma<get_context::v3::Request>,
) -> Result<get_context::v3::Response> {
	let filter = &body.filter;
	let sender = body.sender();
	let (sender_user, _) = sender;
	let room_id = &body.room_id;

	// Use limit or else 10, with maximum 100
	let limit: usize = body
		.limit
		.try_into()
		.unwrap_or(LIMIT_DEFAULT)
		.min(LIMIT_MAX);

	// some clients, at least element, seem to require knowledge of redundant
	// members for "inline" profiles on the timeline to work properly
	let lazy_load_enabled = matches!(filter.lazy_load_options, LazyLoadOptions::Enabled { .. });

	let lazy_load_redundant = if let LazyLoadOptions::Enabled { include_redundant_members } =
		filter.lazy_load_options
	{
		include_redundant_members
	} else {
		false
	};

	let base_id = services
		.rooms
		.timeline
		.get_pdu_id(&body.event_id)
		.map_err(|_| err!(Request(NotFound("Event not found."))));

	let base_pdu = services
		.rooms
		.timeline
		.get_pdu(&body.event_id)
		.map_err(|_| err!(Request(NotFound("Base event not found."))));

	let visible = services
		.rooms
		.state_accessor
		.user_can_see_event(sender_user, &body.room_id, &body.event_id)
		.map(Ok);

	let (base_id, base_pdu, visible) = try_join!(base_id, base_pdu, visible)?;

	if base_pdu.room_id != body.room_id || base_pdu.event_id != body.event_id {
		return Err!(Request(NotFound("Base event not found.")));
	}

	if !visible {
		return Err!(Request(Forbidden("You don't have permission to view this event.")));
	}

	let base_count = base_id.pdu_count();

	let base_event = ignored_filter(&services, (base_count, base_pdu), sender_user);

	let events_before = services
		.rooms
		.timeline
		.pdus_rev(Some(sender_user), room_id, Some(base_count))
		.ignore_err()
		.ready_filter_map(|item| event_filter(item, filter))
		.wide_filter_map(|item| ignored_filter(&services, item, sender_user))
		.wide_filter_map(|item| visibility_filter(&services, item, sender_user))
		.take(limit / 2)
		.collect();

	let events_after = services
		.rooms
		.timeline
		.pdus(Some(sender_user), room_id, Some(base_count))
		.ignore_err()
		.ready_filter_map(|item| event_filter(item, filter))
		.wide_filter_map(|item| ignored_filter(&services, item, sender_user))
		.wide_filter_map(|item| visibility_filter(&services, item, sender_user))
		.take(limit / 2)
		.collect();

	let (base_event, events_before, events_after): (_, Vec<_>, Vec<_>) =
		join!(base_event, events_before, events_after);

	let state_at = events_after
		.last()
		.map(ref_at!(1))
		.map_or(body.event_id.as_ref(), |e| e.event_id.as_ref());

	let state_ids = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(state_at)
		.or_else(|_| services.rooms.state.get_room_shortstatehash(room_id))
		.and_then(|shortstatehash| services.rooms.state_accessor.state_full_ids(shortstatehash))
		.map_err(|e| err!(Database("State not found: {e}")))
		.await?;

	let lazy = base_event
		.iter()
		.chain(events_before.iter())
		.chain(events_after.iter())
		.stream()
		.fold(LazySet::new(), |lazy, item| {
			update_lazy(&services, room_id, sender, lazy, item, lazy_load_redundant)
		})
		.await;

	let lazy = &lazy;
	let state: Vec<_> = state_ids
		.iter()
		.stream()
		.broad_filter_map(|(shortstatekey, event_id)| {
			services
				.rooms
				.short
				.get_statekey_from_short(*shortstatekey)
				.map_ok(move |(event_type, state_key)| (event_type, state_key, event_id))
				.ok()
		})
		.ready_filter_map(|(event_type, state_key, event_id)| {
			if !lazy_load_enabled || event_type != StateEventType::RoomMember {
				return Some(event_id);
			}

			state_key
				.as_str()
				.try_into()
				.ok()
				.filter(|&user_id: &&UserId| lazy.contains(user_id))
				.map(|_| event_id)
		})
		.broad_filter_map(|event_id: &OwnedEventId| {
			services.rooms.timeline.get_pdu(event_id).ok()
		})
		.map(|pdu| pdu.to_state_event())
		.collect()
		.await;

	Ok(get_context::v3::Response {
		event: base_event.map(at!(1)).as_ref().map(PduEvent::to_room_event),

		start: events_before
			.last()
			.map(at!(0))
			.or(Some(base_count))
			.as_ref()
			.map(ToString::to_string),

		end: events_after
			.last()
			.map(at!(0))
			.or(Some(base_count))
			.as_ref()
			.map(ToString::to_string),

		events_before: events_before
			.into_iter()
			.map(at!(1))
			.map(|pdu| pdu.to_room_event())
			.collect(),

		events_after: events_after
			.into_iter()
			.map(at!(1))
			.map(|pdu| pdu.to_room_event())
			.collect(),

		state,
	})
}
