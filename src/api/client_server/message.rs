use std::{
	collections::{BTreeMap, HashSet},
	sync::Arc,
};

use ruma::{
	api::client::{
		error::ErrorKind,
		message::{get_message_events, send_message_event},
	},
	events::{StateEventType, TimelineEventType},
};
use serde_json::from_str;

use crate::{
	service::{pdu::PduBuilder, rooms::timeline::PduCount},
	services, utils, Error, Result, Ruma,
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
pub async fn send_message_event_route(
	body: Ruma<send_message_event::v3::Request>,
) -> Result<send_message_event::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let sender_device = body.sender_device.as_deref();

	let mutex_state =
		Arc::clone(services().globals.roomid_mutex_state.write().await.entry(body.room_id.clone()).or_default());
	let state_lock = mutex_state.lock().await;

	// Forbid m.room.encrypted if encryption is disabled
	if TimelineEventType::RoomEncrypted == body.event_type.to_string().into() && !services().globals.allow_encryption()
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Encryption has been disabled"));
	}

	// certain event types require certain fields to be valid in request bodies.
	// this helps prevent attempting to handle events that we can't deserialise
	// later so don't waste resources on it.
	//
	// see https://spec.matrix.org/v1.9/client-server-api/#events-2 for what's required per event type.
	match body.event_type.to_string().into() {
		TimelineEventType::RoomMessage => {
			let body_field = body.body.body.get_field::<String>("body");
			let msgtype_field = body.body.body.get_field::<String>("msgtype");

			if body_field.is_err() {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"'body' field in JSON request is invalid",
				));
			}

			if msgtype_field.is_err() {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"'msgtype' field in JSON request is invalid",
				));
			}
		},
		TimelineEventType::RoomName => {
			let name_field = body.body.body.get_field::<String>("name");

			if name_field.is_err() {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"'name' field in JSON request is invalid",
				));
			}
		},
		TimelineEventType::RoomTopic => {
			let topic_field = body.body.body.get_field::<String>("topic");

			if topic_field.is_err() {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"'topic' field in JSON request is invalid",
				));
			}
		},
		_ => {}, // event may be custom/experimental or can be empty don't do anything with it
	};

	// Check if this is a new transaction id
	if let Some(response) = services().transaction_ids.existing_txnid(sender_user, sender_device, &body.txn_id)? {
		// The client might have sent a txnid of the /sendToDevice endpoint
		// This txnid has no response associated with it
		if response.is_empty() {
			return Err(Error::BadRequest(
				ErrorKind::InvalidParam,
				"Tried to use txn id already used for an incompatible endpoint.",
			));
		}

		let event_id = utils::string_from_bytes(&response)
			.map_err(|_| Error::bad_database("Invalid txnid bytes in database."))?
			.try_into()
			.map_err(|_| Error::bad_database("Invalid event id in txnid data."))?;
		return Ok(send_message_event::v3::Response {
			event_id,
		});
	}

	let mut unsigned = BTreeMap::new();
	unsigned.insert("transaction_id".to_owned(), body.txn_id.to_string().into());

	let event_id = services()
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: body.event_type.to_string().into(),
				content: from_str(body.body.body.json().get())
					.map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?,
				unsigned: Some(unsigned),
				state_key: None,
				redacts: None,
			},
			sender_user,
			&body.room_id,
			&state_lock,
		)
		.await?;

	services().transaction_ids.add_txnid(sender_user, sender_device, &body.txn_id, event_id.as_bytes())?;

	drop(state_lock);

	Ok(send_message_event::v3::Response::new((*event_id).to_owned()))
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/messages`
///
/// Allows paginating through room history.
///
/// - Only works if the user is joined (TODO: always allow, but only show events
///   where the user was
/// joined, depending on history_visibility)
pub async fn get_message_events_route(
	body: Ruma<get_message_events::v3::Request>,
) -> Result<get_message_events::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let sender_device = body.sender_device.as_ref().expect("user is authenticated");

	let from = match body.from.clone() {
		Some(from) => PduCount::try_from_string(&from)?,
		None => match body.dir {
			ruma::api::Direction::Forward => PduCount::min(),
			ruma::api::Direction::Backward => PduCount::max(),
		},
	};

	let to = body.to.as_ref().and_then(|t| PduCount::try_from_string(t).ok());

	services().rooms.lazy_loading.lazy_load_confirm_delivery(sender_user, sender_device, &body.room_id, from).await?;

	let limit = u64::from(body.limit).min(100) as usize;

	let next_token;

	let mut resp = get_message_events::v3::Response::new();

	let mut lazy_loaded = HashSet::new();

	match body.dir {
		ruma::api::Direction::Forward => {
			let events_after: Vec<_> = services()
				.rooms
				.timeline
				.pdus_after(sender_user, &body.room_id, from)?
				.take(limit)
				.filter_map(std::result::Result::ok) // Filter out buggy events
				.filter(|(_, pdu)| {
					services()
						.rooms
						.state_accessor
						.user_can_see_event(sender_user, &body.room_id, &pdu.event_id)
						.unwrap_or(false)
				})
				.take_while(|&(k, _)| Some(k) != to) // Stop at `to`
				.collect();

			for (_, event) in &events_after {
				/* TODO: Remove this when these are resolved:
				 * https://github.com/vector-im/element-android/issues/3417
				 * https://github.com/vector-im/element-web/issues/21034
				if !services().rooms.lazy_loading.lazy_load_was_sent_before(
					sender_user,
					sender_device,
					&body.room_id,
					&event.sender,
				)? {
					lazy_loaded.insert(event.sender.clone());
				}
				*/
				lazy_loaded.insert(event.sender.clone());
			}

			next_token = events_after.last().map(|(count, _)| count).copied();

			let events_after: Vec<_> = events_after.into_iter().map(|(_, pdu)| pdu.to_room_event()).collect();

			resp.start = from.stringify();
			resp.end = next_token.map(|count| count.stringify());
			resp.chunk = events_after;
		},
		ruma::api::Direction::Backward => {
			services().rooms.timeline.backfill_if_required(&body.room_id, from).await?;
			let events_before: Vec<_> = services()
				.rooms
				.timeline
				.pdus_until(sender_user, &body.room_id, from)?
				.take(limit)
				.filter_map(std::result::Result::ok) // Filter out buggy events
				.filter(|(_, pdu)| {
					services()
						.rooms
						.state_accessor
						.user_can_see_event(sender_user, &body.room_id, &pdu.event_id)
						.unwrap_or(false)
				})
				.take_while(|&(k, _)| Some(k) != to) // Stop at `to`
				.collect();

			for (_, event) in &events_before {
				/* TODO: Remove this when these are resolved:
				 * https://github.com/vector-im/element-android/issues/3417
				 * https://github.com/vector-im/element-web/issues/21034
				if !services().rooms.lazy_loading.lazy_load_was_sent_before(
					sender_user,
					sender_device,
					&body.room_id,
					&event.sender,
				)? {
					lazy_loaded.insert(event.sender.clone());
				}
				*/
				lazy_loaded.insert(event.sender.clone());
			}

			next_token = events_before.last().map(|(count, _)| count).copied();

			let events_before: Vec<_> = events_before.into_iter().map(|(_, pdu)| pdu.to_room_event()).collect();

			resp.start = from.stringify();
			resp.end = next_token.map(|count| count.stringify());
			resp.chunk = events_before;
		},
	}

	resp.state = Vec::new();
	for ll_id in &lazy_loaded {
		if let Some(member_event) = services().rooms.state_accessor.room_state_get(
			&body.room_id,
			&StateEventType::RoomMember,
			ll_id.as_str(),
		)? {
			resp.state.push(member_event.to_state_event());
		}
	}

	// TODO: enable again when we are sure clients can handle it
	/*
	if let Some(next_token) = next_token {
		services().rooms.lazy_loading.lazy_load_mark_sent(
			sender_user,
			sender_device,
			&body.room_id,
			lazy_loaded,
			next_token,
		).await;
	}
	*/

	Ok(resp)
}
