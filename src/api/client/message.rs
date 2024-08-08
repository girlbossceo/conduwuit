use std::collections::{BTreeMap, HashSet};

use axum::extract::State;
use conduit::{err, utils::ReadyExt, Err, PduCount};
use futures::{FutureExt, StreamExt};
use ruma::{
	api::client::{
		error::ErrorKind,
		filter::{RoomEventFilter, UrlFilter},
		message::{get_message_events, send_message_event},
	},
	events::{MessageLikeEventType, StateEventType},
	UserId,
};
use serde_json::{from_str, Value};
use service::rooms::timeline::PdusIterItem;

use crate::{
	service::{pdu::PduBuilder, Services},
	utils, Error, Result, Ruma,
};

/// # `PUT /_matrix/client/v3/rooms/{roomId}/send/{eventType}/{txnId}`
///
/// Send a message event into the room.
///
/// - Is a NOOP if the txn id was already used before and returns the same event
///   id again
/// - The only requirement for the content is that it has to be valid json
/// - Tries to send the event into the room, auth rules will determine if it is
///   allowed
pub(crate) async fn send_message_event_route(
	State(services): State<crate::State>, body: Ruma<send_message_event::v3::Request>,
) -> Result<send_message_event::v3::Response> {
	let sender_user = body.sender_user.as_deref().expect("user is authenticated");
	let sender_device = body.sender_device.as_deref();
	let appservice_info = body.appservice_info.as_ref();

	// Forbid m.room.encrypted if encryption is disabled
	if MessageLikeEventType::RoomEncrypted == body.event_type && !services.globals.allow_encryption() {
		return Err!(Request(Forbidden("Encryption has been disabled")));
	}

	let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

	if body.event_type == MessageLikeEventType::CallInvite
		&& services.rooms.directory.is_public_room(&body.room_id).await
	{
		return Err!(Request(Forbidden("Room call invites are not allowed in public rooms")));
	}

	// Check if this is a new transaction id
	if let Ok(response) = services
		.transaction_ids
		.existing_txnid(sender_user, sender_device, &body.txn_id)
		.await
	{
		// The client might have sent a txnid of the /sendToDevice endpoint
		// This txnid has no response associated with it
		if response.is_empty() {
			return Err!(Request(InvalidParam(
				"Tried to use txn id already used for an incompatible endpoint."
			)));
		}

		return Ok(send_message_event::v3::Response {
			event_id: utils::string_from_bytes(&response)
				.map(TryInto::try_into)
				.map_err(|e| err!(Database("Invalid event_id in txnid data: {e:?}")))??,
		});
	}

	let mut unsigned = BTreeMap::new();
	unsigned.insert("transaction_id".to_owned(), body.txn_id.to_string().into());

	let content = from_str(body.body.body.json().get())
		.map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?;

	let event_id = services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: body.event_type.to_string().into(),
				content,
				unsigned: Some(unsigned),
				state_key: None,
				redacts: None,
				timestamp: appservice_info.and(body.timestamp),
			},
			sender_user,
			&body.room_id,
			&state_lock,
		)
		.await
		.map(|event_id| (*event_id).to_owned())?;

	services
		.transaction_ids
		.add_txnid(sender_user, sender_device, &body.txn_id, event_id.as_bytes());

	drop(state_lock);

	Ok(send_message_event::v3::Response {
		event_id,
	})
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/messages`
///
/// Allows paginating through room history.
///
/// - Only works if the user is joined (TODO: always allow, but only show events
///   where the user was joined, depending on `history_visibility`)
pub(crate) async fn get_message_events_route(
	State(services): State<crate::State>, body: Ruma<get_message_events::v3::Request>,
) -> Result<get_message_events::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let sender_device = body.sender_device.as_ref().expect("user is authenticated");

	let room_id = &body.room_id;
	let filter = &body.filter;

	let limit = usize::try_from(body.limit).unwrap_or(10).min(100);
	let from = match body.from.as_ref() {
		Some(from) => PduCount::try_from_string(from)?,
		None => match body.dir {
			ruma::api::Direction::Forward => PduCount::min(),
			ruma::api::Direction::Backward => PduCount::max(),
		},
	};

	let to = body
		.to
		.as_ref()
		.and_then(|t| PduCount::try_from_string(t).ok());

	services
		.rooms
		.lazy_loading
		.lazy_load_confirm_delivery(sender_user, sender_device, room_id, from);

	let mut resp = get_message_events::v3::Response::new();
	let mut lazy_loaded = HashSet::new();
	let next_token;
	match body.dir {
		ruma::api::Direction::Forward => {
			let events_after: Vec<PdusIterItem> = services
				.rooms
				.timeline
				.pdus_after(sender_user, room_id, from)
				.await?
				.ready_filter_map(|item| contains_url_filter(item, filter))
				.filter_map(|item| visibility_filter(&services, item, sender_user))
				.ready_take_while(|(count, _)| Some(*count) != to) // Stop at `to`
				.take(limit)
				.collect()
				.boxed()
				.await;

			for (_, event) in &events_after {
				/* TODO: Remove the not "element_hacks" check when these are resolved:
				 * https://github.com/vector-im/element-android/issues/3417
				 * https://github.com/vector-im/element-web/issues/21034
				 */
				if !cfg!(feature = "element_hacks")
					&& !services
						.rooms
						.lazy_loading
						.lazy_load_was_sent_before(sender_user, sender_device, room_id, &event.sender)
						.await
				{
					lazy_loaded.insert(event.sender.clone());
				}

				if cfg!(features = "element_hacks") {
					lazy_loaded.insert(event.sender.clone());
				}
			}

			next_token = events_after.last().map(|(count, _)| count).copied();

			let events_after: Vec<_> = events_after
				.into_iter()
				.map(|(_, pdu)| pdu.to_room_event())
				.collect();

			resp.start = from.stringify();
			resp.end = next_token.map(|count| count.stringify());
			resp.chunk = events_after;
		},
		ruma::api::Direction::Backward => {
			services
				.rooms
				.timeline
				.backfill_if_required(room_id, from)
				.boxed()
				.await?;

			let events_before: Vec<PdusIterItem> = services
				.rooms
				.timeline
				.pdus_until(sender_user, room_id, from)
				.await?
				.ready_filter_map(|item| contains_url_filter(item, filter))
				.filter_map(|item| visibility_filter(&services, item, sender_user))
				.ready_take_while(|(count, _)| Some(*count) != to) // Stop at `to`
				.take(limit)
				.collect()
				.boxed()
				.await;

			for (_, event) in &events_before {
				/* TODO: Remove the not "element_hacks" check when these are resolved:
				 * https://github.com/vector-im/element-android/issues/3417
				 * https://github.com/vector-im/element-web/issues/21034
				 */
				if !cfg!(feature = "element_hacks")
					&& !services
						.rooms
						.lazy_loading
						.lazy_load_was_sent_before(sender_user, sender_device, room_id, &event.sender)
						.await
				{
					lazy_loaded.insert(event.sender.clone());
				}

				if cfg!(features = "element_hacks") {
					lazy_loaded.insert(event.sender.clone());
				}
			}

			next_token = events_before.last().map(|(count, _)| count).copied();

			let events_before: Vec<_> = events_before
				.into_iter()
				.map(|(_, pdu)| pdu.to_room_event())
				.collect();

			resp.start = from.stringify();
			resp.end = next_token.map(|count| count.stringify());
			resp.chunk = events_before;
		},
	}

	resp.state = Vec::new();
	for ll_id in &lazy_loaded {
		if let Ok(member_event) = services
			.rooms
			.state_accessor
			.room_state_get(room_id, &StateEventType::RoomMember, ll_id.as_str())
			.await
		{
			resp.state.push(member_event.to_state_event());
		}
	}

	// remove the feature check when we are sure clients like element can handle it
	if !cfg!(feature = "element_hacks") {
		if let Some(next_token) = next_token {
			services.rooms.lazy_loading.lazy_load_mark_sent(
				sender_user,
				sender_device,
				room_id,
				lazy_loaded,
				next_token,
			);
		}
	}

	Ok(resp)
}

async fn visibility_filter(services: &Services, item: PdusIterItem, user_id: &UserId) -> Option<PdusIterItem> {
	let (_, pdu) = &item;

	services
		.rooms
		.state_accessor
		.user_can_see_event(user_id, &pdu.room_id, &pdu.event_id)
		.await
		.then_some(item)
}

fn contains_url_filter(item: PdusIterItem, filter: &RoomEventFilter) -> Option<PdusIterItem> {
	let (_, pdu) = &item;

	if filter.url_filter.is_none() {
		return Some(item);
	}

	let content: Value = from_str(pdu.content.get()).unwrap();
	let res = match filter.url_filter {
		Some(UrlFilter::EventsWithoutUrl) => !content["url"].is_string(),
		Some(UrlFilter::EventsWithUrl) => content["url"].is_string(),
		None => true,
	};

	res.then_some(item)
}
