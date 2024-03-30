#![allow(deprecated)]
// Conduit implements the older APIs

use std::{
	collections::BTreeMap,
	sync::Arc,
	time::{Duration, Instant, SystemTime},
};

use axum::{response::IntoResponse, Json};
use get_profile_information::v1::ProfileField;
use ruma::{
	api::{
		client::error::ErrorKind,
		federation::{
			authorization::get_event_authorization,
			backfill::get_backfill,
			device::get_devices::{self, v1::UserDevice},
			directory::{get_public_rooms, get_public_rooms_filtered},
			discovery::{get_server_keys, get_server_version, ServerSigningKeys, VerifyKey},
			event::{get_event, get_missing_events, get_room_state, get_room_state_ids},
			keys::{claim_keys, get_keys},
			membership::{create_invite, create_join_event, prepare_join_event},
			query::{get_profile_information, get_room_information},
			space::get_hierarchy,
			transactions::{
				edu::{DeviceListUpdateContent, DirectDeviceContent, Edu, SigningKeyUpdateContent},
				send_transaction_message,
			},
		},
		OutgoingResponse,
	},
	directory::{Filter, RoomNetwork},
	events::{
		receipt::{ReceiptEvent, ReceiptEventContent, ReceiptType},
		room::{
			join_rules::{JoinRule, RoomJoinRulesEventContent},
			member::{MembershipState, RoomMemberEventContent},
		},
		StateEventType, TimelineEventType,
	},
	serde::{Base64, JsonObject, Raw},
	to_device::DeviceIdOrAllDevices,
	uint, user_id, CanonicalJsonObject, CanonicalJsonValue, EventId, MilliSecondsSinceUnixEpoch, OwnedEventId,
	OwnedRoomId, OwnedServerName, OwnedServerSigningKeyId, OwnedUserId, RoomId, RoomVersionId, ServerName,
};
use serde_json::value::{to_raw_value, RawValue as RawJsonValue};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
	api::client_server::{self, claim_keys_helper, get_keys_helper},
	service::pdu::{gen_event_id_canonical_json, PduBuilder},
	services, utils, Error, PduEvent, Result, Ruma,
};

/// # `GET /_matrix/federation/v1/version`
///
/// Get version information on this server.
pub async fn get_server_version_route(
	_body: Ruma<get_server_version::v1::Request>,
) -> Result<get_server_version::v1::Response> {
	if !services().globals.allow_federation() {
		return Err(Error::bad_config("Federation is disabled."));
	}

	let version = match option_env!("CONDUIT_VERSION_EXTRA") {
		Some(extra) => format!("{} ({})", env!("CARGO_PKG_VERSION"), extra),
		None => env!("CARGO_PKG_VERSION").to_owned(),
	};

	Ok(get_server_version::v1::Response {
		server: Some(get_server_version::v1::Server {
			name: Some("Conduwuit".to_owned()),
			version: Some(version),
		}),
	})
}

/// # `GET /_matrix/key/v2/server`
///
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by
///   this will be valid
/// forever.
// Response type for this endpoint is Json because we need to calculate a
// signature for the response
pub async fn get_server_keys_route() -> Result<impl IntoResponse> {
	if !services().globals.allow_federation() {
		return Err(Error::bad_config("Federation is disabled."));
	}

	let mut verify_keys: BTreeMap<OwnedServerSigningKeyId, VerifyKey> = BTreeMap::new();
	verify_keys.insert(
		format!("ed25519:{}", services().globals.keypair().version())
			.try_into()
			.expect("found invalid server signing keys in DB"),
		VerifyKey {
			key: Base64::new(services().globals.keypair().public_key().to_vec()),
		},
	);
	let mut response = serde_json::from_slice(
		get_server_keys::v2::Response {
			server_key: Raw::new(&ServerSigningKeys {
				server_name: services().globals.server_name().to_owned(),
				verify_keys,
				old_verify_keys: BTreeMap::new(),
				signatures: BTreeMap::new(),
				valid_until_ts: MilliSecondsSinceUnixEpoch::from_system_time(
					SystemTime::now() + Duration::from_secs(86400 * 7),
				)
				.expect("time is valid"),
			})
			.expect("static conversion, no errors"),
		}
		.try_into_http_response::<Vec<u8>>()
		.unwrap()
		.body(),
	)
	.unwrap();

	ruma::signatures::sign_json(
		services().globals.server_name().as_str(),
		services().globals.keypair(),
		&mut response,
	)
	.unwrap();

	Ok(Json(response))
}

/// # `GET /_matrix/key/v2/server/{keyId}`
///
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by
///   this will be valid
/// forever.
pub async fn get_server_keys_deprecated_route() -> impl IntoResponse { get_server_keys_route().await }

/// # `POST /_matrix/federation/v1/publicRooms`
///
/// Lists the public rooms on this server.
pub async fn get_public_rooms_filtered_route(
	body: Ruma<get_public_rooms_filtered::v1::Request>,
) -> Result<get_public_rooms_filtered::v1::Response> {
	if !services()
		.globals
		.allow_public_room_directory_over_federation()
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Room directory is not public"));
	}

	let response = client_server::get_public_rooms_filtered_helper(
		None,
		body.limit,
		body.since.as_deref(),
		&body.filter,
		&body.room_network,
	)
	.await?;

	Ok(get_public_rooms_filtered::v1::Response {
		chunk: response.chunk,
		prev_batch: response.prev_batch,
		next_batch: response.next_batch,
		total_room_count_estimate: response.total_room_count_estimate,
	})
}

/// # `GET /_matrix/federation/v1/publicRooms`
///
/// Lists the public rooms on this server.
pub async fn get_public_rooms_route(
	body: Ruma<get_public_rooms::v1::Request>,
) -> Result<get_public_rooms::v1::Response> {
	if !services()
		.globals
		.allow_public_room_directory_over_federation()
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Room directory is not public"));
	}

	let response = client_server::get_public_rooms_filtered_helper(
		None,
		body.limit,
		body.since.as_deref(),
		&Filter::default(),
		&RoomNetwork::Matrix,
	)
	.await?;

	Ok(get_public_rooms::v1::Response {
		chunk: response.chunk,
		prev_batch: response.prev_batch,
		next_batch: response.next_batch,
		total_room_count_estimate: response.total_room_count_estimate,
	})
}

pub fn parse_incoming_pdu(pdu: &RawJsonValue) -> Result<(OwnedEventId, CanonicalJsonObject, OwnedRoomId)> {
	let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
		warn!("Error parsing incoming event {:?}: {:?}", pdu, e);
		Error::BadServerResponse("Invalid PDU in server response")
	})?;

	let room_id: OwnedRoomId = value
		.get("room_id")
		.and_then(|id| RoomId::parse(id.as_str()?).ok())
		.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Invalid room id in pdu"))?;

	let room_version_id = services().rooms.state.get_room_version(&room_id)?;

	let Ok((event_id, value)) = gen_event_id_canonical_json(pdu, &room_version_id) else {
		// Event could not be converted to canonical json
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Could not convert event to canonical json.",
		));
	};
	Ok((event_id, value, room_id))
}

/// # `PUT /_matrix/federation/v1/send/{txnId}`
///
/// Push EDUs and PDUs to this server.
pub async fn send_transaction_message_route(
	body: Ruma<send_transaction_message::v1::Request>,
) -> Result<send_transaction_message::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	let mut resolved_map = BTreeMap::new();

	let pub_key_map = RwLock::new(BTreeMap::new());

	// This is all the auth_events that have been recursively fetched so they don't
	// have to be deserialized over and over again.
	// TODO: make this persist across requests but not in a DB Tree (in globals?)
	// TODO: This could potentially also be some sort of trie (suffix tree) like
	// structure so that once an auth event is known it would know (using indexes
	// maybe) all of the auth events that it references.
	// let mut auth_cache = EventMap::new();

	let mut parsed_pdus = vec![];
	for pdu in &body.pdus {
		let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
			warn!("Error parsing incoming event {:?}: {:?}", pdu, e);
			Error::BadServerResponse("Invalid PDU in server response")
		})?;
		let room_id: OwnedRoomId = value
			.get("room_id")
			.and_then(|id| RoomId::parse(id.as_str()?).ok())
			.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Invalid room id in pdu"))?;

		if services().rooms.state.get_room_version(&room_id).is_err() {
			debug!("Server is not in room {room_id}");
			continue;
		}

		let r = parse_incoming_pdu(pdu);
		let (event_id, value, room_id) = match r {
			Ok(t) => t,
			Err(e) => {
				warn!("Could not parse PDU: {e}");
				info!("Full PDU: {:?}", &pdu);
				continue;
			},
		};
		parsed_pdus.push((event_id, value, room_id));
		// We do not add the event_id field to the pdu here because of signature
		// and hashes checks
	}

	// We go through all the signatures we see on the PDUs and fetch the
	// corresponding signing keys
	services()
		.rooms
		.event_handler
		.fetch_required_signing_keys(parsed_pdus.iter().map(|(_event_id, event, _room_id)| event), &pub_key_map)
		.await
		.unwrap_or_else(|e| {
			warn!("Could not fetch all signatures for PDUs from {}: {:?}", sender_servername, e);
		});

	for (event_id, value, room_id) in parsed_pdus {
		let mutex = Arc::clone(
			services()
				.globals
				.roomid_mutex_federation
				.write()
				.await
				.entry(room_id.clone())
				.or_default(),
		);
		let mutex_lock = mutex.lock().await;
		let start_time = Instant::now();
		resolved_map.insert(
			event_id.clone(),
			services()
				.rooms
				.event_handler
				.handle_incoming_pdu(sender_servername, &event_id, &room_id, value, true, &pub_key_map)
				.await
				.map(|_| ()),
		);
		drop(mutex_lock);

		let elapsed = start_time.elapsed();
		debug!(
			"Handling transaction of event {} took {}m{}s",
			event_id,
			elapsed.as_secs() / 60,
			elapsed.as_secs() % 60
		);
	}

	for pdu in &resolved_map {
		if let Err(e) = pdu.1 {
			if matches!(e, Error::BadRequest(ErrorKind::NotFound, _)) {
				warn!("Incoming PDU failed {:?}", pdu);
			}
		}
	}

	for edu in body
		.edus
		.iter()
		.filter_map(|edu| serde_json::from_str::<Edu>(edu.json().get()).ok())
	{
		match edu {
			Edu::Presence(presence) => {
				if !services().globals.allow_incoming_presence() {
					continue;
				}

				for update in presence.push {
					for room_id in services().rooms.state_cache.rooms_joined(&update.user_id) {
						services().rooms.edus.presence.set_presence(
							&room_id?,
							&update.user_id,
							update.presence.clone(),
							Some(update.currently_active),
							Some(update.last_active_ago),
							update.status_msg.clone(),
						)?;
					}
				}
			},
			Edu::Receipt(receipt) => {
				if !services().globals.allow_incoming_read_receipts() {
					continue;
				}

				for (room_id, room_updates) in receipt.receipts {
					for (user_id, user_updates) in room_updates.read {
						if let Some((event_id, _)) = user_updates
							.event_ids
							.iter()
							.filter_map(|id| {
								services()
									.rooms
									.timeline
									.get_pdu_count(id)
									.ok()
									.flatten()
									.map(|r| (id, r))
							})
							.max_by_key(|(_, count)| *count)
						{
							let mut user_receipts = BTreeMap::new();
							user_receipts.insert(user_id.clone(), user_updates.data);

							let mut receipts = BTreeMap::new();
							receipts.insert(ReceiptType::Read, user_receipts);

							let mut receipt_content = BTreeMap::new();
							receipt_content.insert(event_id.to_owned(), receipts);

							let event = ReceiptEvent {
								content: ReceiptEventContent(receipt_content),
								room_id: room_id.clone(),
							};
							services()
								.rooms
								.edus
								.read_receipt
								.readreceipt_update(&user_id, &room_id, event)?;
						} else {
							// TODO fetch missing events
							debug!("No known event ids in read receipt: {:?}", user_updates);
						}
					}
				}
			},
			Edu::Typing(typing) => {
				if !services().globals.config.allow_incoming_typing {
					continue;
				}

				if services()
					.rooms
					.state_cache
					.is_joined(&typing.user_id, &typing.room_id)?
				{
					if typing.typing {
						let timeout = utils::millis_since_unix_epoch()
							+ services().globals.config.typing_federation_timeout_s * 1000;
						services()
							.rooms
							.edus
							.typing
							.typing_add(&typing.user_id, &typing.room_id, timeout)
							.await?;
					} else {
						services()
							.rooms
							.edus
							.typing
							.typing_remove(&typing.user_id, &typing.room_id)
							.await?;
					}
				}
			},
			Edu::DeviceListUpdate(DeviceListUpdateContent {
				user_id,
				..
			}) => {
				services().users.mark_device_key_update(&user_id)?;
			},
			Edu::DirectToDevice(DirectDeviceContent {
				sender,
				ev_type,
				message_id,
				messages,
			}) => {
				// Check if this is a new transaction id
				if services()
					.transaction_ids
					.existing_txnid(&sender, None, &message_id)?
					.is_some()
				{
					continue;
				}

				for (target_user_id, map) in &messages {
					for (target_device_id_maybe, event) in map {
						match target_device_id_maybe {
							DeviceIdOrAllDevices::DeviceId(target_device_id) => {
								services().users.add_to_device_event(
									&sender,
									target_user_id,
									target_device_id,
									&ev_type.to_string(),
									event.deserialize_as().map_err(|e| {
										warn!("To-Device event is invalid: {event:?} {e}");
										Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid")
									})?,
								)?;
							},

							DeviceIdOrAllDevices::AllDevices => {
								for target_device_id in services().users.all_device_ids(target_user_id) {
									services().users.add_to_device_event(
										&sender,
										target_user_id,
										&target_device_id?,
										&ev_type.to_string(),
										event.deserialize_as().map_err(|_| {
											Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid")
										})?,
									)?;
								}
							},
						}
					}
				}

				// Save transaction id with empty data
				services()
					.transaction_ids
					.add_txnid(&sender, None, &message_id, &[])?;
			},
			Edu::SigningKeyUpdate(SigningKeyUpdateContent {
				user_id,
				master_key,
				self_signing_key,
			}) => {
				if user_id.server_name() != sender_servername {
					continue;
				}
				if let Some(master_key) = master_key {
					services()
						.users
						.add_cross_signing_keys(&user_id, &master_key, &self_signing_key, &None, true)?;
				}
			},
			Edu::_Custom(_) => {},
		}
	}

	Ok(send_transaction_message::v1::Response {
		pdus: resolved_map
			.into_iter()
			.map(|(e, r)| (e, r.map_err(|e| e.sanitized_error())))
			.collect(),
	})
}

/// # `GET /_matrix/federation/v1/event/{eventId}`
///
/// Retrieves a single event from the server.
///
/// - Only works if a user of this server is currently invited or joined the
///   room
pub async fn get_event_route(body: Ruma<get_event::v1::Request>) -> Result<get_event::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	let event = services()
		.rooms
		.timeline
		.get_pdu_json(&body.event_id)?
		.ok_or_else(|| {
			warn!("Event not found, event ID: {:?}", &body.event_id);
			Error::BadRequest(ErrorKind::NotFound, "Event not found.")
		})?;

	let room_id_str = event
		.get("room_id")
		.and_then(|val| val.as_str())
		.ok_or_else(|| Error::bad_database("Invalid event in database"))?;

	let room_id = <&RoomId>::try_from(room_id_str)
		.map_err(|_| Error::bad_database("Invalid room id field in event in database"))?;

	if !services()
		.rooms
		.state_cache
		.server_in_room(sender_servername, room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not in room"));
	}

	if !services()
		.rooms
		.state_accessor
		.server_can_see_event(sender_servername, room_id, &body.event_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not allowed to see event."));
	}

	Ok(get_event::v1::Response {
		origin: services().globals.server_name().to_owned(),
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
		pdu: PduEvent::convert_to_outgoing_federation_event(event),
	})
}

/// # `GET /_matrix/federation/v1/backfill/<room_id>`
///
/// Retrieves events from before the sender joined the room, if the room's
/// history visibility allows.
pub async fn get_backfill_route(body: Ruma<get_backfill::v1::Request>) -> Result<get_backfill::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	debug!("Got backfill request from: {}", sender_servername);

	if !services()
		.rooms
		.state_cache
		.server_in_room(sender_servername, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not in room."));
	}

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	let until = body
		.v
		.iter()
		.map(|eventid| services().rooms.timeline.get_pdu_count(eventid))
		.filter_map(|r| r.ok().flatten())
		.max()
		.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "No known eventid in v"))?;

	let limit = body.limit.min(uint!(100));

	let all_events = services()
		.rooms
		.timeline
		.pdus_until(user_id!("@doesntmatter:conduit.rs"), &body.room_id, until)?
		.take(limit.try_into().unwrap());

	let events = all_events
		.filter_map(Result::ok)
		.filter(|(_, e)| {
			matches!(
				services()
					.rooms
					.state_accessor
					.server_can_see_event(sender_servername, &e.room_id, &e.event_id,),
				Ok(true),
			)
		})
		.map(|(_, pdu)| services().rooms.timeline.get_pdu_json(&pdu.event_id))
		.filter_map(|r| r.ok().flatten())
		.map(PduEvent::convert_to_outgoing_federation_event)
		.collect();

	Ok(get_backfill::v1::Response {
		origin: services().globals.server_name().to_owned(),
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
		pdus: events,
	})
}

/// # `POST /_matrix/federation/v1/get_missing_events/{roomId}`
///
/// Retrieves events that the sender is missing.
pub async fn get_missing_events_route(
	body: Ruma<get_missing_events::v1::Request>,
) -> Result<get_missing_events::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	if !services()
		.rooms
		.state_cache
		.server_in_room(sender_servername, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not in room"));
	}

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	let mut queued_events = body.latest_events.clone();
	let mut events = Vec::new();

	let mut i = 0;
	while i < queued_events.len() && events.len() < u64::from(body.limit) as usize {
		if let Some(pdu) = services().rooms.timeline.get_pdu_json(&queued_events[i])? {
			let room_id_str = pdu
				.get("room_id")
				.and_then(|val| val.as_str())
				.ok_or_else(|| Error::bad_database("Invalid event in database"))?;

			let event_room_id = <&RoomId>::try_from(room_id_str)
				.map_err(|_| Error::bad_database("Invalid room id field in event in database"))?;

			if event_room_id != body.room_id {
				warn!(
					"Evil event detected: Event {} found while searching in room {}",
					queued_events[i], body.room_id
				);
				return Err(Error::BadRequest(ErrorKind::InvalidParam, "Evil event detected"));
			}

			if body.earliest_events.contains(&queued_events[i]) {
				i += 1;
				continue;
			}

			if !services().rooms.state_accessor.server_can_see_event(
				sender_servername,
				&body.room_id,
				&queued_events[i],
			)? {
				i += 1;
				continue;
			}

			queued_events.extend_from_slice(
				&serde_json::from_value::<Vec<OwnedEventId>>(
					serde_json::to_value(
						pdu.get("prev_events")
							.cloned()
							.ok_or_else(|| Error::bad_database("Event in db has no prev_events field."))?,
					)
					.expect("canonical json is valid json value"),
				)
				.map_err(|_| Error::bad_database("Invalid prev_events content in pdu in db."))?,
			);
			events.push(PduEvent::convert_to_outgoing_federation_event(pdu));
		}
		i += 1;
	}

	Ok(get_missing_events::v1::Response {
		events,
	})
}

/// # `GET /_matrix/federation/v1/event_auth/{roomId}/{eventId}`
///
/// Retrieves the auth chain for a given event.
///
/// - This does not include the event itself
pub async fn get_event_authorization_route(
	body: Ruma<get_event_authorization::v1::Request>,
) -> Result<get_event_authorization::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	if !services()
		.rooms
		.state_cache
		.server_in_room(sender_servername, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not in room."));
	}

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	let event = services()
		.rooms
		.timeline
		.get_pdu_json(&body.event_id)?
		.ok_or_else(|| {
			warn!("Event not found, event ID: {:?}", &body.event_id);
			Error::BadRequest(ErrorKind::NotFound, "Event not found.")
		})?;

	let room_id_str = event
		.get("room_id")
		.and_then(|val| val.as_str())
		.ok_or_else(|| Error::bad_database("Invalid event in database"))?;

	let room_id = <&RoomId>::try_from(room_id_str)
		.map_err(|_| Error::bad_database("Invalid room id field in event in database"))?;

	let auth_chain_ids = services()
		.rooms
		.auth_chain
		.get_auth_chain(room_id, vec![Arc::from(&*body.event_id)])
		.await?;

	Ok(get_event_authorization::v1::Response {
		auth_chain: auth_chain_ids
			.filter_map(|id| services().rooms.timeline.get_pdu_json(&id).ok()?)
			.map(PduEvent::convert_to_outgoing_federation_event)
			.collect(),
	})
}

/// # `GET /_matrix/federation/v1/state/{roomId}`
///
/// Retrieves the current state of the room.
pub async fn get_room_state_route(body: Ruma<get_room_state::v1::Request>) -> Result<get_room_state::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	if !services()
		.rooms
		.state_cache
		.server_in_room(sender_servername, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not in room."));
	}

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	let shortstatehash = services()
		.rooms
		.state_accessor
		.pdu_shortstatehash(&body.event_id)?
		.ok_or(Error::BadRequest(ErrorKind::NotFound, "Pdu state not found."))?;

	let pdus = services()
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await?
		.into_values()
		.map(|id| {
			PduEvent::convert_to_outgoing_federation_event(
				services()
					.rooms
					.timeline
					.get_pdu_json(&id)
					.unwrap()
					.unwrap(),
			)
		})
		.collect();

	let auth_chain_ids = services()
		.rooms
		.auth_chain
		.get_auth_chain(&body.room_id, vec![Arc::from(&*body.event_id)])
		.await?;

	Ok(get_room_state::v1::Response {
		auth_chain: auth_chain_ids
			.filter_map(|id| {
				if let Some(json) = services().rooms.timeline.get_pdu_json(&id).ok()? {
					Some(PduEvent::convert_to_outgoing_federation_event(json))
				} else {
					error!("Could not find event json for {id} in db.");
					None
				}
			})
			.collect(),
		pdus,
	})
}

/// # `GET /_matrix/federation/v1/state_ids/{roomId}`
///
/// Retrieves the current state of the room.
pub async fn get_room_state_ids_route(
	body: Ruma<get_room_state_ids::v1::Request>,
) -> Result<get_room_state_ids::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	if !services()
		.rooms
		.state_cache
		.server_in_room(sender_servername, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::Forbidden, "Server is not in room."));
	}

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	let shortstatehash = services()
		.rooms
		.state_accessor
		.pdu_shortstatehash(&body.event_id)?
		.ok_or(Error::BadRequest(ErrorKind::NotFound, "Pdu state not found."))?;

	let pdu_ids = services()
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await?
		.into_values()
		.map(|id| (*id).to_owned())
		.collect();

	let auth_chain_ids = services()
		.rooms
		.auth_chain
		.get_auth_chain(&body.room_id, vec![Arc::from(&*body.event_id)])
		.await?;

	Ok(get_room_state_ids::v1::Response {
		auth_chain_ids: auth_chain_ids.map(|id| (*id).to_owned()).collect(),
		pdu_ids,
	})
}

/// # `GET /_matrix/federation/v1/make_join/{roomId}/{userId}`
///
/// Creates a join template.
pub async fn create_join_event_template_route(
	body: Ruma<prepare_join_event::v1::Request>,
) -> Result<prepare_join_event::v1::Response> {
	if !services().rooms.metadata.exists(&body.room_id)? {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server."));
	}

	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	let mutex_state = Arc::clone(
		services()
			.globals
			.roomid_mutex_state
			.write()
			.await
			.entry(body.room_id.clone())
			.or_default(),
	);
	let state_lock = mutex_state.lock().await;

	// TODO: Conduit does not implement restricted join rules yet, we always reject
	let join_rules_event =
		services()
			.rooms
			.state_accessor
			.room_state_get(&body.room_id, &StateEventType::RoomJoinRules, "")?;

	let join_rules_event_content: Option<RoomJoinRulesEventContent> = join_rules_event
		.as_ref()
		.map(|join_rules_event| {
			serde_json::from_str(join_rules_event.content.get()).map_err(|e| {
				warn!("Invalid join rules event: {}", e);
				Error::bad_database("Invalid join rules event in db.")
			})
		})
		.transpose()?;

	if let Some(join_rules_event_content) = join_rules_event_content {
		if matches!(
			join_rules_event_content.join_rule,
			JoinRule::Restricted { .. } | JoinRule::KnockRestricted { .. }
		) {
			return Err(Error::BadRequest(
				ErrorKind::UnableToAuthorizeJoin,
				"Conduit does not support restricted rooms yet.",
			));
		}
	}

	let room_version_id = services().rooms.state.get_room_version(&body.room_id)?;
	if !body.ver.contains(&room_version_id) {
		return Err(Error::BadRequest(
			ErrorKind::IncompatibleRoomVersion {
				room_version: room_version_id,
			},
			"Room version not supported.",
		));
	}

	let content = to_raw_value(&RoomMemberEventContent {
		avatar_url: None,
		blurhash: None,
		displayname: None,
		is_direct: None,
		membership: MembershipState::Join,
		third_party_invite: None,
		reason: None,
		join_authorized_via_users_server: None,
	})
	.expect("member event is valid value");

	let (_pdu, mut pdu_json) = services().rooms.timeline.create_hash_and_sign_event(
		PduBuilder {
			event_type: TimelineEventType::RoomMember,
			content,
			unsigned: None,
			state_key: Some(body.user_id.to_string()),
			redacts: None,
		},
		&body.user_id,
		&body.room_id,
		&state_lock,
	)?;

	drop(state_lock);

	// room v3 and above removed the "event_id" field from remote PDU format
	match room_version_id {
		RoomVersionId::V1 | RoomVersionId::V2 => {},
		RoomVersionId::V3
		| RoomVersionId::V4
		| RoomVersionId::V5
		| RoomVersionId::V6
		| RoomVersionId::V7
		| RoomVersionId::V8
		| RoomVersionId::V9
		| RoomVersionId::V10
		| RoomVersionId::V11 => {
			pdu_json.remove("event_id");
		},
		_ => {
			warn!("Unexpected or unsupported room version {room_version_id}");
			return Err(Error::BadRequest(
				ErrorKind::BadJson,
				"Unexpected or unsupported room version found",
			));
		},
	};

	Ok(prepare_join_event::v1::Response {
		room_version: Some(room_version_id),
		event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
	})
}

async fn create_join_event(
	sender_servername: &ServerName, room_id: &RoomId, pdu: &RawJsonValue,
) -> Result<create_join_event::v1::RoomState> {
	if !services().rooms.metadata.exists(room_id)? {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server."));
	}

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, room_id)?;

	// TODO: Conduit does not implement restricted join rules yet, we always reject
	let join_rules_event =
		services()
			.rooms
			.state_accessor
			.room_state_get(room_id, &StateEventType::RoomJoinRules, "")?;

	let join_rules_event_content: Option<RoomJoinRulesEventContent> = join_rules_event
		.as_ref()
		.map(|join_rules_event| {
			serde_json::from_str(join_rules_event.content.get()).map_err(|e| {
				warn!("Invalid join rules event: {}", e);
				Error::bad_database("Invalid join rules event in db.")
			})
		})
		.transpose()?;

	if let Some(join_rules_event_content) = join_rules_event_content {
		if matches!(
			join_rules_event_content.join_rule,
			JoinRule::Restricted { .. } | JoinRule::KnockRestricted { .. }
		) {
			return Err(Error::BadRequest(
				ErrorKind::UnableToAuthorizeJoin,
				"Conduit does not support restricted rooms yet.",
			));
		}
	}

	// We need to return the state prior to joining, let's keep a reference to that
	// here
	let shortstatehash = services()
		.rooms
		.state
		.get_room_shortstatehash(room_id)?
		.ok_or(Error::BadRequest(ErrorKind::NotFound, "Pdu state not found."))?;

	let pub_key_map = RwLock::new(BTreeMap::new());
	// let mut auth_cache = EventMap::new();

	// We do not add the event_id field to the pdu here because of signature and
	// hashes checks
	let room_version_id = services().rooms.state.get_room_version(room_id)?;
	let Ok((event_id, value)) = gen_event_id_canonical_json(pdu, &room_version_id) else {
		// Event could not be converted to canonical json
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Could not convert event to canonical json.",
		));
	};

	let origin: OwnedServerName = serde_json::from_value(
		serde_json::to_value(
			value
				.get("origin")
				.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Event needs an origin field."))?,
		)
		.expect("CanonicalJson is valid json value"),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Origin field is invalid."))?;

	services()
		.rooms
		.event_handler
		.fetch_required_signing_keys([&value], &pub_key_map)
		.await?;

	let mutex = Arc::clone(
		services()
			.globals
			.roomid_mutex_federation
			.write()
			.await
			.entry(room_id.to_owned())
			.or_default(),
	);
	let mutex_lock = mutex.lock().await;
	let pdu_id: Vec<u8> = services()
		.rooms
		.event_handler
		.handle_incoming_pdu(&origin, &event_id, room_id, value, true, &pub_key_map)
		.await?
		.ok_or(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Could not accept incoming PDU as timeline event.",
		))?;
	drop(mutex_lock);

	let state_ids = services()
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await?;
	let auth_chain_ids = services()
		.rooms
		.auth_chain
		.get_auth_chain(room_id, state_ids.values().cloned().collect())
		.await?;

	services().sending.send_pdu_room(room_id, &pdu_id)?;

	Ok(create_join_event::v1::RoomState {
		auth_chain: auth_chain_ids
			.filter_map(|id| services().rooms.timeline.get_pdu_json(&id).ok().flatten())
			.map(PduEvent::convert_to_outgoing_federation_event)
			.collect(),
		state: state_ids
			.iter()
			.filter_map(|(_, id)| services().rooms.timeline.get_pdu_json(id).ok().flatten())
			.map(PduEvent::convert_to_outgoing_federation_event)
			.collect(),
		event: None, // TODO: handle restricted joins
	})
}

/// # `PUT /_matrix/federation/v1/send_join/{roomId}/{eventId}`
///
/// Submits a signed join event.
pub async fn create_join_event_v1_route(
	body: Ruma<create_join_event::v1::Request>,
) -> Result<create_join_event::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	let room_state = create_join_event(sender_servername, &body.room_id, &body.pdu).await?;

	Ok(create_join_event::v1::Response {
		room_state,
	})
}

/// # `PUT /_matrix/federation/v2/send_join/{roomId}/{eventId}`
///
/// Submits a signed join event.
pub async fn create_join_event_v2_route(
	body: Ruma<create_join_event::v2::Request>,
) -> Result<create_join_event::v2::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	let create_join_event::v1::RoomState {
		auth_chain,
		state,
		event,
	} = create_join_event(sender_servername, &body.room_id, &body.pdu).await?;
	let room_state = create_join_event::v2::RoomState {
		members_omitted: false,
		auth_chain,
		state,
		event,
		servers_in_room: None,
	};

	Ok(create_join_event::v2::Response {
		room_state,
	})
}

/// # `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}`
///
/// Invites a remote user to a room.
pub async fn create_invite_route(body: Ruma<create_invite::v2::Request>) -> Result<create_invite::v2::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	services()
		.rooms
		.event_handler
		.acl_check(sender_servername, &body.room_id)?;

	if !services()
		.globals
		.supported_room_versions()
		.contains(&body.room_version)
	{
		return Err(Error::BadRequest(
			ErrorKind::IncompatibleRoomVersion {
				room_version: body.room_version.clone(),
			},
			"Server does not support this room version.",
		));
	}

	let mut signed_event = utils::to_canonical_object(&body.event).map_err(|e| {
		error!("Failed to convert invite event to canonical JSON: {}", e);
		Error::BadRequest(ErrorKind::InvalidParam, "Invite event is invalid.")
	})?;

	ruma::signatures::hash_and_sign_event(
		services().globals.server_name().as_str(),
		services().globals.keypair(),
		&mut signed_event,
		&body.room_version,
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Failed to sign event."))?;

	// Generate event id
	let event_id = EventId::parse(format!(
		"${}",
		ruma::signatures::reference_hash(&signed_event, &body.room_version)
			.expect("ruma can calculate reference hashes")
	))
	.expect("ruma's reference hashes are valid event ids");

	// Add event_id back
	signed_event.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.to_string()));

	let sender: OwnedUserId = serde_json::from_value(
		signed_event
			.get("sender")
			.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Event had no sender field."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "sender is not a user id."))?;

	let invited_user: Box<_> = serde_json::from_value(
		signed_event
			.get("state_key")
			.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Event had no state_key field."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "state_key is not a user id."))?;

	if services().rooms.metadata.is_banned(&body.room_id)? && !services().users.is_admin(&invited_user)? {
		info!(
			"Received remote invite from server {} for room {} and for user {invited_user}, but room is banned by us.",
			&sender_servername, &body.room_id
		);
		return Err(Error::BadRequest(
			ErrorKind::Forbidden,
			"This room is banned on this homeserver.",
		));
	}

	if services().globals.block_non_admin_invites() && !services().users.is_admin(&invited_user)? {
		info!(
			"Received remote invite from server {} for room {} and for user {invited_user} who is not an admin, but \
			 \"block_non_admin_invites\" is enabled, rejecting.",
			&sender_servername, &body.room_id
		);
		return Err(Error::BadRequest(
			ErrorKind::Forbidden,
			"This server does not allow room invites.",
		));
	}

	let mut invite_state = body.invite_room_state.clone();

	let mut event: JsonObject = serde_json::from_str(body.event.get())
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid invite event bytes."))?;

	event.insert("event_id".to_owned(), "$placeholder".into());

	let pdu: PduEvent = serde_json::from_value(event.into()).map_err(|e| {
		warn!("Invalid invite event: {}", e);
		Error::BadRequest(ErrorKind::InvalidParam, "Invalid invite event.")
	})?;

	invite_state.push(pdu.to_stripped_state_event());

	// If we are active in the room, the remote server will notify us about the join
	// via /send
	if !services()
		.rooms
		.state_cache
		.server_in_room(services().globals.server_name(), &body.room_id)?
	{
		services()
			.rooms
			.state_cache
			.update_membership(
				&body.room_id,
				&invited_user,
				RoomMemberEventContent::new(MembershipState::Invite),
				&sender,
				Some(invite_state),
				true,
			)
			.await?;
	}

	Ok(create_invite::v2::Response {
		event: PduEvent::convert_to_outgoing_federation_event(signed_event),
	})
}

/// # `GET /_matrix/federation/v1/user/devices/{userId}`
///
/// Gets information on all devices of the user.
pub async fn get_devices_route(body: Ruma<get_devices::v1::Request>) -> Result<get_devices::v1::Response> {
	if body.user_id.server_name() != services().globals.server_name() {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Tried to access user from other server.",
		));
	}

	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	Ok(get_devices::v1::Response {
		user_id: body.user_id.clone(),
		stream_id: services()
			.users
			.get_devicelist_version(&body.user_id)?
			.unwrap_or(0)
			.try_into()
			.expect("version will not grow that large"),
		devices: services()
			.users
			.all_devices_metadata(&body.user_id)
			.filter_map(Result::ok)
			.filter_map(|metadata| {
				let device_id_string = metadata.device_id.as_str().to_owned();
				let device_display_name = if services().globals.allow_device_name_federation() {
					metadata.display_name
				} else {
					Some(device_id_string)
				};
				Some(UserDevice {
					keys: services()
						.users
						.get_device_keys(&body.user_id, &metadata.device_id)
						.ok()??,
					device_id: metadata.device_id,
					device_display_name,
				})
			})
			.collect(),
		master_key: services()
			.users
			.get_master_key(None, &body.user_id, &|u| u.server_name() == sender_servername)?,
		self_signing_key: services()
			.users
			.get_self_signing_key(None, &body.user_id, &|u| u.server_name() == sender_servername)?,
	})
}

/// # `GET /_matrix/federation/v1/query/directory`
///
/// Resolve a room alias to a room id.
pub async fn get_room_information_route(
	body: Ruma<get_room_information::v1::Request>,
) -> Result<get_room_information::v1::Response> {
	let room_id = services()
		.rooms
		.alias
		.resolve_local_alias(&body.room_alias)?
		.ok_or(Error::BadRequest(ErrorKind::NotFound, "Room alias not found."))?;

	Ok(get_room_information::v1::Response {
		room_id,
		servers: vec![services().globals.server_name().to_owned()],
	})
}

/// # `GET /_matrix/federation/v1/query/profile`
///
/// Gets information on a profile.
pub async fn get_profile_information_route(
	body: Ruma<get_profile_information::v1::Request>,
) -> Result<get_profile_information::v1::Response> {
	if body.user_id.server_name() != services().globals.server_name() {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"User does not belong to this server",
		));
	}

	let mut displayname = None;
	let mut avatar_url = None;
	let mut blurhash = None;

	match &body.field {
		Some(ProfileField::DisplayName) => {
			displayname = services().users.displayname(&body.user_id)?;
		},
		Some(ProfileField::AvatarUrl) => {
			avatar_url = services().users.avatar_url(&body.user_id)?;
			blurhash = services().users.blurhash(&body.user_id)?;
		},
		// TODO: what to do with custom
		Some(_) => {},
		None => {
			displayname = services().users.displayname(&body.user_id)?;
			avatar_url = services().users.avatar_url(&body.user_id)?;
			blurhash = services().users.blurhash(&body.user_id)?;
		},
	}

	Ok(get_profile_information::v1::Response {
		displayname,
		avatar_url,
		blurhash,
	})
}

/// # `POST /_matrix/federation/v1/user/keys/query`
///
/// Gets devices and identity keys for the given users.
pub async fn get_keys_route(body: Ruma<get_keys::v1::Request>) -> Result<get_keys::v1::Response> {
	if body
		.device_keys
		.iter()
		.any(|(u, _)| u.server_name() != services().globals.server_name())
	{
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"User does not belong to this server.",
		));
	}

	let result = get_keys_helper(
		None,
		&body.device_keys,
		|u| Some(u.server_name()) == body.sender_servername.as_deref(),
		services().globals.allow_device_name_federation(),
	)
	.await?;

	Ok(get_keys::v1::Response {
		device_keys: result.device_keys,
		master_keys: result.master_keys,
		self_signing_keys: result.self_signing_keys,
	})
}

/// # `POST /_matrix/federation/v1/user/keys/claim`
///
/// Claims one-time keys.
pub async fn claim_keys_route(body: Ruma<claim_keys::v1::Request>) -> Result<claim_keys::v1::Response> {
	if body
		.one_time_keys
		.iter()
		.any(|(u, _)| u.server_name() != services().globals.server_name())
	{
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Tried to access user from other server.",
		));
	}

	let result = claim_keys_helper(&body.one_time_keys).await?;

	Ok(claim_keys::v1::Response {
		one_time_keys: result.one_time_keys,
	})
}

/// # `GET /.well-known/matrix/server`
pub async fn well_known_server_route() -> Result<impl IntoResponse> {
	if !services().globals.allow_federation() {
		return Err(Error::bad_config("Federation is disabled."));
	}

	let server_url = match services().globals.well_known_server() {
		Some(url) => url.clone(),
		None => return Err(Error::BadRequest(ErrorKind::NotFound, "Not found.")),
	};

	Ok(Json(serde_json::json!({
		"m.server": server_url
	})))
}

/// # `GET /_matrix/federation/v1/hierarchy/{roomId}`
///
/// Gets the space tree in a depth-first manner to locate child rooms of a given
/// space.
pub async fn get_hierarchy_route(body: Ruma<get_hierarchy::v1::Request>) -> Result<get_hierarchy::v1::Response> {
	let sender_servername = body
		.sender_servername
		.as_ref()
		.expect("server is authenticated");

	if services().rooms.metadata.exists(&body.room_id)? {
		services()
			.rooms
			.spaces
			.get_federation_hierarchy(&body.room_id, sender_servername, body.suggested_only)
			.await
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "Room does not exist."))
	}
}
