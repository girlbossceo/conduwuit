use std::{
	collections::{hash_map::Entry, BTreeMap, HashMap, HashSet},
	net::IpAddr,
	sync::Arc,
	time::Instant,
};

use axum::extract::State;
use axum_client_ip::InsecureClientIp;
use conduit::{
	debug, debug_error, debug_warn, err, error, info,
	pdu::{gen_event_id_canonical_json, PduBuilder},
	trace, utils,
	utils::{math::continue_exponential_backoff_secs, IterStream, ReadyExt},
	warn, Err, Error, PduEvent, Result,
};
use futures::{FutureExt, StreamExt};
use ruma::{
	api::{
		client::{
			error::ErrorKind,
			membership::{
				ban_user, forget_room, get_member_events, invite_user, join_room_by_id, join_room_by_id_or_alias,
				joined_members::{self, v3::RoomMember},
				joined_rooms, kick_user, leave_room, unban_user, ThirdPartySigned,
			},
		},
		federation::{self, membership::create_invite},
	},
	canonical_json::to_canonical_value,
	events::{
		room::{
			join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent},
			member::{MembershipState, RoomMemberEventContent},
			message::RoomMessageEventContent,
		},
		StateEventType,
	},
	serde::Base64,
	state_res, CanonicalJsonObject, CanonicalJsonValue, EventId, OwnedEventId, OwnedRoomId, OwnedServerName,
	OwnedUserId, RoomId, RoomVersionId, ServerName, UserId,
};
use serde_json::value::RawValue as RawJsonValue;
use service::{appservice::RegistrationInfo, rooms::state::RoomMutexGuard, Services};
use tokio::sync::RwLock;

use crate::{client::full_user_deactivate, Ruma};

/// Checks if the room is banned in any way possible and the sender user is not
/// an admin.
///
/// Performs automatic deactivation if `auto_deactivate_banned_room_attempts` is
/// enabled
#[tracing::instrument(skip(services))]
async fn banned_room_check(
	services: &Services, user_id: &UserId, room_id: Option<&RoomId>, server_name: Option<&ServerName>,
	client_ip: IpAddr,
) -> Result<()> {
	if !services.users.is_admin(user_id).await {
		if let Some(room_id) = room_id {
			if services.rooms.metadata.is_banned(room_id).await
				|| services
					.globals
					.config
					.forbidden_remote_server_names
					.contains(&room_id.server_name().unwrap().to_owned())
			{
				warn!(
					"User {user_id} who is not an admin attempted to send an invite for or attempted to join a banned \
					 room or banned room server name: {room_id}"
				);

				if services.globals.config.auto_deactivate_banned_room_attempts {
					warn!("Automatically deactivating user {user_id} due to attempted banned room join");
					services
						.admin
						.send_message(RoomMessageEventContent::text_plain(format!(
							"Automatically deactivating user {user_id} due to attempted banned room join from IP \
							 {client_ip}"
						)))
						.await
						.ok();

					let all_joined_rooms: Vec<OwnedRoomId> = services
						.rooms
						.state_cache
						.rooms_joined(user_id)
						.map(Into::into)
						.collect()
						.await;

					full_user_deactivate(services, user_id, &all_joined_rooms).await?;
				}

				return Err!(Request(Forbidden("This room is banned on this homeserver.")));
			}
		} else if let Some(server_name) = server_name {
			if services
				.globals
				.config
				.forbidden_remote_server_names
				.contains(&server_name.to_owned())
			{
				warn!(
					"User {user_id} who is not an admin tried joining a room which has the server name {server_name} \
					 that is globally forbidden. Rejecting.",
				);

				if services.globals.config.auto_deactivate_banned_room_attempts {
					warn!("Automatically deactivating user {user_id} due to attempted banned room join");
					services
						.admin
						.send_message(RoomMessageEventContent::text_plain(format!(
							"Automatically deactivating user {user_id} due to attempted banned room join from IP \
							 {client_ip}"
						)))
						.await
						.ok();

					let all_joined_rooms: Vec<OwnedRoomId> = services
						.rooms
						.state_cache
						.rooms_joined(user_id)
						.map(Into::into)
						.collect()
						.await;

					full_user_deactivate(services, user_id, &all_joined_rooms).await?;
				}

				return Err!(Request(Forbidden("This remote server is banned on this homeserver.")));
			}
		}
	}

	Ok(())
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/join`
///
/// Tries to join the sender user into a room.
///
/// - If the server knowns about this room: creates the join event and does auth
///   rules locally
/// - If the server does not know about the room: asks other servers over
///   federation
#[tracing::instrument(skip_all, fields(%client_ip), name = "join")]
pub(crate) async fn join_room_by_id_route(
	State(services): State<crate::State>, InsecureClientIp(client_ip): InsecureClientIp,
	body: Ruma<join_room_by_id::v3::Request>,
) -> Result<join_room_by_id::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	banned_room_check(
		&services,
		sender_user,
		Some(&body.room_id),
		body.room_id.server_name(),
		client_ip,
	)
	.await?;

	// There is no body.server_name for /roomId/join
	let mut servers: Vec<_> = services
		.rooms
		.state_cache
		.servers_invite_via(&body.room_id)
		.map(ToOwned::to_owned)
		.collect()
		.await;

	servers.extend(
		services
			.rooms
			.state_cache
			.invite_state(sender_user, &body.room_id)
			.await
			.unwrap_or_default()
			.iter()
			.filter_map(|event| event.get_field("sender").ok().flatten())
			.filter_map(|sender: &str| UserId::parse(sender).ok())
			.map(|user| user.server_name().to_owned()),
	);

	if let Some(server) = body.room_id.server_name() {
		servers.push(server.into());
	}

	join_room_by_id_helper(
		&services,
		sender_user,
		&body.room_id,
		body.reason.clone(),
		&servers,
		body.third_party_signed.as_ref(),
		&body.appservice_info,
	)
	.boxed()
	.await
}

/// # `POST /_matrix/client/r0/join/{roomIdOrAlias}`
///
/// Tries to join the sender user into a room.
///
/// - If the server knowns about this room: creates the join event and does auth
///   rules locally
/// - If the server does not know about the room: use the server name query
///   param if specified. if not specified, asks other servers over federation
///   via room alias server name and room ID server name
#[tracing::instrument(skip_all, fields(%client), name = "join")]
pub(crate) async fn join_room_by_id_or_alias_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<join_room_by_id_or_alias::v3::Request>,
) -> Result<join_room_by_id_or_alias::v3::Response> {
	let sender_user = body.sender_user.as_deref().expect("user is authenticated");
	let appservice_info = &body.appservice_info;
	let body = body.body;

	let (servers, room_id) = match OwnedRoomId::try_from(body.room_id_or_alias) {
		Ok(room_id) => {
			banned_room_check(&services, sender_user, Some(&room_id), room_id.server_name(), client).await?;

			let mut servers = body.via.clone();
			servers.extend(
				services
					.rooms
					.state_cache
					.servers_invite_via(&room_id)
					.map(ToOwned::to_owned)
					.collect::<Vec<_>>()
					.await,
			);

			servers.extend(
				services
					.rooms
					.state_cache
					.invite_state(sender_user, &room_id)
					.await
					.unwrap_or_default()
					.iter()
					.filter_map(|event| event.get_field("sender").ok().flatten())
					.filter_map(|sender: &str| UserId::parse(sender).ok())
					.map(|user| user.server_name().to_owned()),
			);

			if let Some(server) = room_id.server_name() {
				servers.push(server.to_owned());
			}

			(servers, room_id)
		},
		Err(room_alias) => {
			let response = services
				.rooms
				.alias
				.resolve_alias(&room_alias, Some(&body.via.clone()))
				.await?;
			let (room_id, mut pre_servers) = response;

			banned_room_check(&services, sender_user, Some(&room_id), Some(room_alias.server_name()), client).await?;

			let mut servers = body.via;
			if let Some(pre_servers) = &mut pre_servers {
				servers.append(pre_servers);
			}

			servers.extend(
				services
					.rooms
					.state_cache
					.servers_invite_via(&room_id)
					.map(ToOwned::to_owned)
					.collect::<Vec<_>>()
					.await,
			);

			servers.extend(
				services
					.rooms
					.state_cache
					.invite_state(sender_user, &room_id)
					.await
					.unwrap_or_default()
					.iter()
					.filter_map(|event| event.get_field("sender").ok().flatten())
					.filter_map(|sender: &str| UserId::parse(sender).ok())
					.map(|user| user.server_name().to_owned()),
			);

			(servers, room_id)
		},
	};

	let join_room_response = join_room_by_id_helper(
		&services,
		sender_user,
		&room_id,
		body.reason.clone(),
		&servers,
		body.third_party_signed.as_ref(),
		appservice_info,
	)
	.boxed()
	.await?;

	Ok(join_room_by_id_or_alias::v3::Response {
		room_id: join_room_response.room_id,
	})
}

/// # `POST /_matrix/client/v3/rooms/{roomId}/leave`
///
/// Tries to leave the sender user from a room.
///
/// - This should always work if the user is currently joined.
pub(crate) async fn leave_room_route(
	State(services): State<crate::State>, body: Ruma<leave_room::v3::Request>,
) -> Result<leave_room::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	leave_room(&services, sender_user, &body.room_id, body.reason.clone()).await?;

	Ok(leave_room::v3::Response::new())
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/invite`
///
/// Tries to send an invite event into the room.
#[tracing::instrument(skip_all, fields(%client), name = "invite")]
pub(crate) async fn invite_user_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<invite_user::v3::Request>,
) -> Result<invite_user::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if !services.users.is_admin(sender_user).await && services.globals.block_non_admin_invites() {
		info!(
			"User {sender_user} is not an admin and attempted to send an invite to room {}",
			&body.room_id
		);
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"Invites are not allowed on this server.",
		));
	}

	banned_room_check(&services, sender_user, Some(&body.room_id), body.room_id.server_name(), client).await?;

	if let invite_user::v3::InvitationRecipient::UserId {
		user_id,
	} = &body.recipient
	{
		if services.users.user_is_ignored(sender_user, user_id).await {
			return Err!(Request(Forbidden("You cannot invite users you have ignored to rooms.")));
		} else if services.users.user_is_ignored(user_id, sender_user).await {
			// silently drop the invite to the recipient if they've been ignored by the
			// sender, pretend it worked
			return Ok(invite_user::v3::Response {});
		}

		invite_helper(&services, sender_user, user_id, &body.room_id, body.reason.clone(), false).await?;
		Ok(invite_user::v3::Response {})
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "User not found."))
	}
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/kick`
///
/// Tries to send a kick event into the room.
pub(crate) async fn kick_user_route(
	State(services): State<crate::State>, body: Ruma<kick_user::v3::Request>,
) -> Result<kick_user::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

	let event: RoomMemberEventContent = services
		.rooms
		.state_accessor
		.room_state_get_content(&body.room_id, &StateEventType::RoomMember, body.user_id.as_ref())
		.await
		.map_err(|_| err!(Request(BadState("Cannot kick member that's not in the room."))))?;

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				body.user_id.to_string(),
				&RoomMemberEventContent {
					membership: MembershipState::Leave,
					reason: body.reason.clone(),
					..event
				},
			),
			sender_user,
			&body.room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	Ok(kick_user::v3::Response::new())
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/ban`
///
/// Tries to send a ban event into the room.
pub(crate) async fn ban_user_route(
	State(services): State<crate::State>, body: Ruma<ban_user::v3::Request>,
) -> Result<ban_user::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

	let blurhash = services.users.blurhash(&body.user_id).await.ok();

	let event = services
		.rooms
		.state_accessor
		.room_state_get_content(&body.room_id, &StateEventType::RoomMember, body.user_id.as_ref())
		.await
		.map_or_else(
			|_| RoomMemberEventContent {
				blurhash: blurhash.clone(),
				reason: body.reason.clone(),
				..RoomMemberEventContent::new(MembershipState::Ban)
			},
			|event| RoomMemberEventContent {
				membership: MembershipState::Ban,
				displayname: None,
				avatar_url: None,
				blurhash: blurhash.clone(),
				reason: body.reason.clone(),
				join_authorized_via_users_server: None,
				..event
			},
		);

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(body.user_id.to_string(), &event),
			sender_user,
			&body.room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	Ok(ban_user::v3::Response::new())
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/unban`
///
/// Tries to send an unban event into the room.
pub(crate) async fn unban_user_route(
	State(services): State<crate::State>, body: Ruma<unban_user::v3::Request>,
) -> Result<unban_user::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

	let event: RoomMemberEventContent = services
		.rooms
		.state_accessor
		.room_state_get_content(&body.room_id, &StateEventType::RoomMember, body.user_id.as_ref())
		.await
		.map_err(|_| err!(Request(BadState("Cannot unban a user who is not banned."))))?;

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				body.user_id.to_string(),
				&RoomMemberEventContent {
					membership: MembershipState::Leave,
					reason: body.reason.clone(),
					join_authorized_via_users_server: None,
					..event
				},
			),
			sender_user,
			&body.room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	Ok(unban_user::v3::Response::new())
}

/// # `POST /_matrix/client/v3/rooms/{roomId}/forget`
///
/// Forgets about a room.
///
/// - If the sender user currently left the room: Stops sender user from
///   receiving information about the room
///
/// Note: Other devices of the user have no way of knowing the room was
/// forgotten, so this has to be called from every device
pub(crate) async fn forget_room_route(
	State(services): State<crate::State>, body: Ruma<forget_room::v3::Request>,
) -> Result<forget_room::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if services
		.rooms
		.state_cache
		.is_joined(sender_user, &body.room_id)
		.await
	{
		return Err!(Request(Unknown("You must leave the room before forgetting it")));
	}

	services
		.rooms
		.state_cache
		.forget(&body.room_id, sender_user);

	Ok(forget_room::v3::Response::new())
}

/// # `POST /_matrix/client/r0/joined_rooms`
///
/// Lists all rooms the user has joined.
pub(crate) async fn joined_rooms_route(
	State(services): State<crate::State>, body: Ruma<joined_rooms::v3::Request>,
) -> Result<joined_rooms::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	Ok(joined_rooms::v3::Response {
		joined_rooms: services
			.rooms
			.state_cache
			.rooms_joined(sender_user)
			.map(ToOwned::to_owned)
			.collect()
			.await,
	})
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/members`
///
/// Lists all joined users in a room (TODO: at a specific point in time, with a
/// specific membership).
///
/// - Only works if the user is currently joined
pub(crate) async fn get_member_events_route(
	State(services): State<crate::State>, body: Ruma<get_member_events::v3::Request>,
) -> Result<get_member_events::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if !services
		.rooms
		.state_accessor
		.user_can_see_state_events(sender_user, &body.room_id)
		.await
	{
		return Err!(Request(Forbidden("You don't have permission to view this room.")));
	}

	Ok(get_member_events::v3::Response {
		chunk: services
			.rooms
			.state_accessor
			.room_state_full(&body.room_id)
			.await?
			.iter()
			.filter(|(key, _)| key.0 == StateEventType::RoomMember)
			.map(|(_, pdu)| pdu.to_member_event())
			.collect(),
	})
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/joined_members`
///
/// Lists all members of a room.
///
/// - The sender user must be in the room
/// - TODO: An appservice just needs a puppet joined
pub(crate) async fn joined_members_route(
	State(services): State<crate::State>, body: Ruma<joined_members::v3::Request>,
) -> Result<joined_members::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if !services
		.rooms
		.state_accessor
		.user_can_see_state_events(sender_user, &body.room_id)
		.await
	{
		return Err!(Request(Forbidden("You don't have permission to view this room.")));
	}

	let joined: BTreeMap<OwnedUserId, RoomMember> = services
		.rooms
		.state_cache
		.room_members(&body.room_id)
		.map(ToOwned::to_owned)
		.then(|user| async move {
			(
				user.clone(),
				RoomMember {
					display_name: services.users.displayname(&user).await.ok(),
					avatar_url: services.users.avatar_url(&user).await.ok(),
				},
			)
		})
		.collect()
		.await;

	Ok(joined_members::v3::Response {
		joined,
	})
}

pub async fn join_room_by_id_helper(
	services: &Services, sender_user: &UserId, room_id: &RoomId, reason: Option<String>, servers: &[OwnedServerName],
	third_party_signed: Option<&ThirdPartySigned>, appservice_info: &Option<RegistrationInfo>,
) -> Result<join_room_by_id::v3::Response> {
	let state_lock = services.rooms.state.mutex.lock(room_id).await;

	let user_is_guest = services
		.users
		.is_deactivated(sender_user)
		.await
		.unwrap_or(false)
		&& appservice_info.is_none();

	if user_is_guest && !services.rooms.state_accessor.guest_can_join(room_id).await {
		return Err!(Request(Forbidden("Guests are not allowed to join this room")));
	}

	if services
		.rooms
		.state_cache
		.is_joined(sender_user, room_id)
		.await
	{
		debug_warn!("{sender_user} is already joined in {room_id}");
		return Ok(join_room_by_id::v3::Response {
			room_id: room_id.into(),
		});
	}

	if services
		.rooms
		.state_cache
		.server_in_room(services.globals.server_name(), room_id)
		.await || servers.is_empty()
		|| (servers.len() == 1 && services.globals.server_is_ours(&servers[0]))
	{
		join_room_by_id_helper_local(services, sender_user, room_id, reason, servers, third_party_signed, state_lock)
			.boxed()
			.await
	} else {
		// Ask a remote server if we are not participating in this room
		join_room_by_id_helper_remote(services, sender_user, room_id, reason, servers, third_party_signed, state_lock)
			.boxed()
			.await
	}
}

#[tracing::instrument(skip_all, fields(%sender_user, %room_id), name = "join_remote")]
async fn join_room_by_id_helper_remote(
	services: &Services, sender_user: &UserId, room_id: &RoomId, reason: Option<String>, servers: &[OwnedServerName],
	_third_party_signed: Option<&ThirdPartySigned>, state_lock: RoomMutexGuard,
) -> Result<join_room_by_id::v3::Response> {
	info!("Joining {room_id} over federation.");

	let (make_join_response, remote_server) = make_join_request(services, sender_user, room_id, servers).await?;

	info!("make_join finished");

	let room_version_id = match make_join_response.room_version {
		Some(room_version)
			if services
				.globals
				.supported_room_versions()
				.contains(&room_version) =>
		{
			room_version
		},
		_ => return Err!(BadServerResponse("Room version is not supported")),
	};

	let mut join_event_stub: CanonicalJsonObject = serde_json::from_str(make_join_response.event.get())
		.map_err(|e| err!(BadServerResponse("Invalid make_join event json received from server: {e:?}")))?;

	let join_authorized_via_users_server = join_event_stub
		.get("content")
		.map(|s| {
			s.as_object()?
				.get("join_authorised_via_users_server")?
				.as_str()
		})
		.and_then(|s| OwnedUserId::try_from(s.unwrap_or_default()).ok());

	// TODO: Is origin needed?
	join_event_stub.insert(
		"origin".to_owned(),
		CanonicalJsonValue::String(services.globals.server_name().as_str().to_owned()),
	);
	join_event_stub.insert(
		"origin_server_ts".to_owned(),
		CanonicalJsonValue::Integer(
			utils::millis_since_unix_epoch()
				.try_into()
				.expect("Timestamp is valid js_int value"),
		),
	);
	join_event_stub.insert(
		"content".to_owned(),
		to_canonical_value(RoomMemberEventContent {
			displayname: services.users.displayname(sender_user).await.ok(),
			avatar_url: services.users.avatar_url(sender_user).await.ok(),
			blurhash: services.users.blurhash(sender_user).await.ok(),
			reason,
			join_authorized_via_users_server: join_authorized_via_users_server.clone(),
			..RoomMemberEventContent::new(MembershipState::Join)
		})
		.expect("event is valid, we just created it"),
	);

	// We keep the "event_id" in the pdu only in v1 or
	// v2 rooms
	match room_version_id {
		RoomVersionId::V1 | RoomVersionId::V2 => {},
		_ => {
			join_event_stub.remove("event_id");
		},
	};

	// In order to create a compatible ref hash (EventID) the `hashes` field needs
	// to be present
	ruma::signatures::hash_and_sign_event(
		services.globals.server_name().as_str(),
		services.globals.keypair(),
		&mut join_event_stub,
		&room_version_id,
	)
	.expect("event is valid, we just created it");

	// Generate event id
	let event_id = format!(
		"${}",
		ruma::signatures::reference_hash(&join_event_stub, &room_version_id)
			.expect("ruma can calculate reference hashes")
	);
	let event_id = <&EventId>::try_from(event_id.as_str()).expect("ruma's reference hashes are valid event ids");

	// Add event_id back
	join_event_stub.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.as_str().to_owned()));

	// It has enough fields to be called a proper event now
	let mut join_event = join_event_stub;

	info!("Asking {remote_server} for send_join in room {room_id}");
	let send_join_response = services
		.sending
		.send_federation_request(
			&remote_server,
			federation::membership::create_join_event::v2::Request {
				room_id: room_id.to_owned(),
				event_id: event_id.to_owned(),
				omit_members: false,
				pdu: services
					.sending
					.convert_to_outgoing_federation_event(join_event.clone())
					.await,
			},
		)
		.await?;

	info!("send_join finished");

	if join_authorized_via_users_server.is_some() {
		use RoomVersionId::*;
		match &room_version_id {
			V1 | V2 | V3 | V4 | V5 | V6 | V7 => {
				warn!(
					"Found `join_authorised_via_users_server` but room {} is version {}. Ignoring.",
					room_id, &room_version_id
				);
			},
			// only room versions 8 and above using `join_authorized_via_users_server` (restricted joins) need to
			// validate and send signatures
			_ => {
				if let Some(signed_raw) = &send_join_response.room_state.event {
					info!(
						"There is a signed event. This room is probably using restricted joins. Adding signature to \
						 our event"
					);
					let Ok((signed_event_id, signed_value)) = gen_event_id_canonical_json(signed_raw, &room_version_id)
					else {
						// Event could not be converted to canonical json
						return Err(Error::BadRequest(
							ErrorKind::InvalidParam,
							"Could not convert event to canonical json.",
						));
					};

					if signed_event_id != event_id {
						return Err(Error::BadRequest(
							ErrorKind::InvalidParam,
							"Server sent event with wrong event id",
						));
					}

					match signed_value["signatures"]
						.as_object()
						.ok_or(Error::BadRequest(
							ErrorKind::InvalidParam,
							"Server sent invalid signatures type",
						))
						.and_then(|e| {
							e.get(remote_server.as_str())
								.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Server did not send its signature"))
						}) {
						Ok(signature) => {
							join_event
								.get_mut("signatures")
								.expect("we created a valid pdu")
								.as_object_mut()
								.expect("we created a valid pdu")
								.insert(remote_server.to_string(), signature.clone());
						},
						Err(e) => {
							warn!(
								"Server {remote_server} sent invalid signature in sendjoin signatures for event \
								 {signed_value:?}: {e:?}",
							);
						},
					}
				}
			},
		}
	}

	services
		.rooms
		.short
		.get_or_create_shortroomid(room_id)
		.await;

	info!("Parsing join event");
	let parsed_join_pdu = PduEvent::from_id_val(event_id, join_event.clone())
		.map_err(|e| err!(BadServerResponse("Invalid join event PDU: {e:?}")))?;

	let mut state = HashMap::new();
	let pub_key_map = RwLock::new(BTreeMap::new());

	info!("Fetching join signing keys");
	services
		.server_keys
		.fetch_join_signing_keys(&send_join_response, &room_version_id, &pub_key_map)
		.await?;

	info!("Going through send_join response room_state");
	for result in send_join_response
		.room_state
		.state
		.iter()
		.map(|pdu| validate_and_add_event_id(services, pdu, &room_version_id, &pub_key_map))
	{
		let Ok((event_id, value)) = result.await else {
			continue;
		};

		let pdu = PduEvent::from_id_val(&event_id, value.clone()).map_err(|e| {
			debug_warn!("Invalid PDU in send_join response: {value:#?}");
			err!(BadServerResponse("Invalid PDU in send_join response: {e:?}"))
		})?;

		services.rooms.outlier.add_pdu_outlier(&event_id, &value);
		if let Some(state_key) = &pdu.state_key {
			let shortstatekey = services
				.rooms
				.short
				.get_or_create_shortstatekey(&pdu.kind.to_string().into(), state_key)
				.await;
			state.insert(shortstatekey, pdu.event_id.clone());
		}
	}

	info!("Going through send_join response auth_chain");
	for result in send_join_response
		.room_state
		.auth_chain
		.iter()
		.map(|pdu| validate_and_add_event_id(services, pdu, &room_version_id, &pub_key_map))
	{
		let Ok((event_id, value)) = result.await else {
			continue;
		};

		services.rooms.outlier.add_pdu_outlier(&event_id, &value);
	}

	debug!("Running send_join auth check");
	let fetch_state = &state;
	let state_fetch = |k: &'static StateEventType, s: String| async move {
		let shortstatekey = services.rooms.short.get_shortstatekey(k, &s).await.ok()?;

		let event_id = fetch_state.get(&shortstatekey)?;
		services.rooms.timeline.get_pdu(event_id).await.ok()
	};

	let auth_check = state_res::event_auth::auth_check(
		&state_res::RoomVersion::new(&room_version_id).expect("room version is supported"),
		&parsed_join_pdu,
		None, // TODO: third party invite
		|k, s| state_fetch(k, s.to_owned()),
	)
	.await
	.map_err(|e| err!(Request(Forbidden(warn!("Auth check failed: {e:?}")))))?;

	if !auth_check {
		return Err!(Request(Forbidden("Auth check failed")));
	}

	info!("Saving state from send_join");
	let (statehash_before_join, new, removed) = services
		.rooms
		.state_compressor
		.save_state(
			room_id,
			Arc::new(
				state
					.into_iter()
					.stream()
					.then(|(k, id)| async move {
						services
							.rooms
							.state_compressor
							.compress_state_event(k, &id)
							.await
					})
					.collect()
					.await,
			),
		)
		.await?;

	services
		.rooms
		.state
		.force_state(room_id, statehash_before_join, new, removed, &state_lock)
		.await?;

	info!("Updating joined counts for new room");
	services
		.rooms
		.state_cache
		.update_joined_count(room_id)
		.await;

	// We append to state before appending the pdu, so we don't have a moment in
	// time with the pdu without it's state. This is okay because append_pdu can't
	// fail.
	let statehash_after_join = services
		.rooms
		.state
		.append_to_state(&parsed_join_pdu)
		.await?;

	info!("Appending new room join event");
	services
		.rooms
		.timeline
		.append_pdu(
			&parsed_join_pdu,
			join_event,
			vec![(*parsed_join_pdu.event_id).to_owned()],
			&state_lock,
		)
		.await?;

	info!("Setting final room state for new room");
	// We set the room state after inserting the pdu, so that we never have a moment
	// in time where events in the current room state do not exist
	services
		.rooms
		.state
		.set_room_state(room_id, statehash_after_join, &state_lock);

	Ok(join_room_by_id::v3::Response::new(room_id.to_owned()))
}

#[tracing::instrument(skip_all, fields(%sender_user, %room_id), name = "join_local")]
async fn join_room_by_id_helper_local(
	services: &Services, sender_user: &UserId, room_id: &RoomId, reason: Option<String>, servers: &[OwnedServerName],
	_third_party_signed: Option<&ThirdPartySigned>, state_lock: RoomMutexGuard,
) -> Result<join_room_by_id::v3::Response> {
	debug!("We can join locally");

	let join_rules_event_content = services
		.rooms
		.state_accessor
		.room_state_get_content(room_id, &StateEventType::RoomJoinRules, "")
		.await
		.map(|content: RoomJoinRulesEventContent| content);

	let restriction_rooms = match join_rules_event_content {
		Ok(RoomJoinRulesEventContent {
			join_rule: JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted),
		}) => restricted
			.allow
			.into_iter()
			.filter_map(|a| match a {
				AllowRule::RoomMembership(r) => Some(r.room_id),
				_ => None,
			})
			.collect(),
		_ => Vec::new(),
	};

	let local_members: Vec<_> = services
		.rooms
		.state_cache
		.room_members(room_id)
		.ready_filter(|user| services.globals.user_is_local(user))
		.map(ToOwned::to_owned)
		.collect()
		.await;

	let mut join_authorized_via_users_server: Option<OwnedUserId> = None;

	if restriction_rooms
		.iter()
		.stream()
		.any(|restriction_room_id| {
			services
				.rooms
				.state_cache
				.is_joined(sender_user, restriction_room_id)
		})
		.await
	{
		for user in local_members {
			if services
				.rooms
				.state_accessor
				.user_can_invite(room_id, &user, sender_user, &state_lock)
				.await
			{
				join_authorized_via_users_server = Some(user);
				break;
			}
		}
	}

	let content = RoomMemberEventContent {
		displayname: services.users.displayname(sender_user).await.ok(),
		avatar_url: services.users.avatar_url(sender_user).await.ok(),
		blurhash: services.users.blurhash(sender_user).await.ok(),
		reason: reason.clone(),
		join_authorized_via_users_server,
		..RoomMemberEventContent::new(MembershipState::Join)
	};

	// Try normal join first
	let error = match services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(sender_user.to_string(), &content),
			sender_user,
			room_id,
			&state_lock,
		)
		.await
	{
		Ok(_event_id) => return Ok(join_room_by_id::v3::Response::new(room_id.to_owned())),
		Err(e) => e,
	};

	if !restriction_rooms.is_empty()
		&& servers
			.iter()
			.any(|server_name| !services.globals.server_is_ours(server_name))
	{
		warn!("We couldn't do the join locally, maybe federation can help to satisfy the restricted join requirements");
		let (make_join_response, remote_server) = make_join_request(services, sender_user, room_id, servers).await?;

		let room_version_id = match make_join_response.room_version {
			Some(room_version_id)
				if services
					.globals
					.supported_room_versions()
					.contains(&room_version_id) =>
			{
				room_version_id
			},
			_ => return Err!(BadServerResponse("Room version is not supported")),
		};
		let mut join_event_stub: CanonicalJsonObject = serde_json::from_str(make_join_response.event.get())
			.map_err(|e| err!(BadServerResponse("Invalid make_join event json received from server: {e:?}")))?;
		let join_authorized_via_users_server = join_event_stub
			.get("content")
			.map(|s| {
				s.as_object()?
					.get("join_authorised_via_users_server")?
					.as_str()
			})
			.and_then(|s| OwnedUserId::try_from(s.unwrap_or_default()).ok());
		// TODO: Is origin needed?
		join_event_stub.insert(
			"origin".to_owned(),
			CanonicalJsonValue::String(services.globals.server_name().as_str().to_owned()),
		);
		join_event_stub.insert(
			"origin_server_ts".to_owned(),
			CanonicalJsonValue::Integer(
				utils::millis_since_unix_epoch()
					.try_into()
					.expect("Timestamp is valid js_int value"),
			),
		);
		join_event_stub.insert(
			"content".to_owned(),
			to_canonical_value(RoomMemberEventContent {
				displayname: services.users.displayname(sender_user).await.ok(),
				avatar_url: services.users.avatar_url(sender_user).await.ok(),
				blurhash: services.users.blurhash(sender_user).await.ok(),
				reason,
				join_authorized_via_users_server,
				..RoomMemberEventContent::new(MembershipState::Join)
			})
			.expect("event is valid, we just created it"),
		);

		// We keep the "event_id" in the pdu only in v1 or
		// v2 rooms
		match room_version_id {
			RoomVersionId::V1 | RoomVersionId::V2 => {},
			_ => {
				join_event_stub.remove("event_id");
			},
		};

		// In order to create a compatible ref hash (EventID) the `hashes` field needs
		// to be present
		ruma::signatures::hash_and_sign_event(
			services.globals.server_name().as_str(),
			services.globals.keypair(),
			&mut join_event_stub,
			&room_version_id,
		)
		.expect("event is valid, we just created it");

		// Generate event id
		let event_id = format!(
			"${}",
			ruma::signatures::reference_hash(&join_event_stub, &room_version_id)
				.expect("ruma can calculate reference hashes")
		);
		let event_id = <&EventId>::try_from(event_id.as_str()).expect("ruma's reference hashes are valid event ids");

		// Add event_id back
		join_event_stub.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.as_str().to_owned()));

		// It has enough fields to be called a proper event now
		let join_event = join_event_stub;

		let send_join_response = services
			.sending
			.send_federation_request(
				&remote_server,
				federation::membership::create_join_event::v2::Request {
					room_id: room_id.to_owned(),
					event_id: event_id.to_owned(),
					omit_members: false,
					pdu: services
						.sending
						.convert_to_outgoing_federation_event(join_event.clone())
						.await,
				},
			)
			.await?;

		if let Some(signed_raw) = send_join_response.room_state.event {
			let Ok((signed_event_id, signed_value)) = gen_event_id_canonical_json(&signed_raw, &room_version_id) else {
				// Event could not be converted to canonical json
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"Could not convert event to canonical json.",
				));
			};

			if signed_event_id != event_id {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"Server sent event with wrong event id",
				));
			}

			drop(state_lock);
			let pub_key_map = RwLock::new(BTreeMap::new());
			services
				.server_keys
				.fetch_required_signing_keys([&signed_value], &pub_key_map)
				.await?;
			services
				.rooms
				.event_handler
				.handle_incoming_pdu(&remote_server, room_id, &signed_event_id, signed_value, true, &pub_key_map)
				.await?;
		} else {
			return Err(error);
		}
	} else {
		return Err(error);
	}

	Ok(join_room_by_id::v3::Response::new(room_id.to_owned()))
}

async fn make_join_request(
	services: &Services, sender_user: &UserId, room_id: &RoomId, servers: &[OwnedServerName],
) -> Result<(federation::membership::prepare_join_event::v1::Response, OwnedServerName)> {
	let mut make_join_response_and_server = Err!(BadServerResponse("No server available to assist in joining."));

	let mut make_join_counter: u16 = 0;
	let mut incompatible_room_version_count: u8 = 0;

	for remote_server in servers {
		if services.globals.server_is_ours(remote_server) {
			continue;
		}
		info!("Asking {remote_server} for make_join ({make_join_counter})");
		let make_join_response = services
			.sending
			.send_federation_request(
				remote_server,
				federation::membership::prepare_join_event::v1::Request {
					room_id: room_id.to_owned(),
					user_id: sender_user.to_owned(),
					ver: services.globals.supported_room_versions(),
				},
			)
			.await;

		trace!("make_join response: {:?}", make_join_response);
		make_join_counter = make_join_counter.saturating_add(1);

		if let Err(ref e) = make_join_response {
			trace!("make_join ErrorKind string: {:?}", e.kind().to_string());

			// converting to a string is necessary (i think) because ruma is forcing us to
			// fill in the struct for M_INCOMPATIBLE_ROOM_VERSION
			if e.kind().to_string().contains("M_INCOMPATIBLE_ROOM_VERSION")
				|| e.kind().to_string().contains("M_UNSUPPORTED_ROOM_VERSION")
			{
				incompatible_room_version_count = incompatible_room_version_count.saturating_add(1);
			}

			if incompatible_room_version_count > 15 {
				info!(
					"15 servers have responded with M_INCOMPATIBLE_ROOM_VERSION or M_UNSUPPORTED_ROOM_VERSION, \
					 assuming that Conduwuit does not support the room {room_id}: {e}"
				);
				make_join_response_and_server = Err!(BadServerResponse("Room version is not supported by Conduwuit"));
				return make_join_response_and_server;
			}

			if make_join_counter > 50 {
				warn!(
					"50 servers failed to provide valid make_join response, assuming no server can assist in joining."
				);
				make_join_response_and_server = Err!(BadServerResponse("No server available to assist in joining."));
				return make_join_response_and_server;
			}
		}

		make_join_response_and_server = make_join_response.map(|r| (r, remote_server.clone()));

		if make_join_response_and_server.is_ok() {
			break;
		}
	}

	make_join_response_and_server
}

pub async fn validate_and_add_event_id(
	services: &Services, pdu: &RawJsonValue, room_version: &RoomVersionId,
	pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
) -> Result<(OwnedEventId, CanonicalJsonObject)> {
	let mut value: CanonicalJsonObject = serde_json::from_str(pdu.get())
		.map_err(|e| err!(BadServerResponse(debug_error!("Invalid PDU in server response: {e:?}"))))?;
	let event_id = EventId::parse(format!(
		"${}",
		ruma::signatures::reference_hash(&value, room_version).expect("ruma can calculate reference hashes")
	))
	.expect("ruma's reference hashes are valid event ids");

	let back_off = |id| async {
		match services
			.globals
			.bad_event_ratelimiter
			.write()
			.expect("locked")
			.entry(id)
		{
			Entry::Vacant(e) => {
				e.insert((Instant::now(), 1));
			},
			Entry::Occupied(mut e) => {
				*e.get_mut() = (Instant::now(), e.get().1.saturating_add(1));
			},
		}
	};

	if let Some((time, tries)) = services
		.globals
		.bad_event_ratelimiter
		.read()
		.expect("locked")
		.get(&event_id)
	{
		// Exponential backoff
		const MIN: u64 = 60 * 5;
		const MAX: u64 = 60 * 60 * 24;
		if continue_exponential_backoff_secs(MIN, MAX, time.elapsed(), *tries) {
			return Err!(BadServerResponse("bad event {event_id:?}, still backing off"));
		}
	}

	if let Err(e) = ruma::signatures::verify_event(&*pub_key_map.read().await, &value, room_version) {
		debug_error!("Event {event_id} failed verification {pdu:#?}");
		let e = Err!(BadServerResponse(debug_error!("Event {event_id} failed verification: {e:?}")));
		back_off(event_id).await;
		return e;
	}

	value.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.as_str().to_owned()));

	Ok((event_id, value))
}

pub(crate) async fn invite_helper(
	services: &Services, sender_user: &UserId, user_id: &UserId, room_id: &RoomId, reason: Option<String>,
	is_direct: bool,
) -> Result<()> {
	if !services.users.is_admin(user_id).await && services.globals.block_non_admin_invites() {
		info!("User {sender_user} is not an admin and attempted to send an invite to room {room_id}");
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"Invites are not allowed on this server.",
		));
	}

	if !services.globals.user_is_local(user_id) {
		let (pdu, pdu_json, invite_room_state) = {
			let state_lock = services.rooms.state.mutex.lock(room_id).await;

			let content = RoomMemberEventContent {
				avatar_url: services.users.avatar_url(user_id).await.ok(),
				is_direct: Some(is_direct),
				reason,
				..RoomMemberEventContent::new(MembershipState::Invite)
			};

			let (pdu, pdu_json) = services
				.rooms
				.timeline
				.create_hash_and_sign_event(
					PduBuilder::state(user_id.to_string(), &content),
					sender_user,
					room_id,
					&state_lock,
				)
				.await?;

			let invite_room_state = services.rooms.state.summary_stripped(&pdu).await;

			drop(state_lock);

			(pdu, pdu_json, invite_room_state)
		};

		let room_version_id = services.rooms.state.get_room_version(room_id).await?;

		let response = services
			.sending
			.send_federation_request(
				user_id.server_name(),
				create_invite::v2::Request {
					room_id: room_id.to_owned(),
					event_id: (*pdu.event_id).to_owned(),
					room_version: room_version_id.clone(),
					event: services
						.sending
						.convert_to_outgoing_federation_event(pdu_json.clone())
						.await,
					invite_room_state,
					via: services
						.rooms
						.state_cache
						.servers_route_via(room_id)
						.await
						.ok(),
				},
			)
			.await?;

		let pub_key_map = RwLock::new(BTreeMap::new());

		// We do not add the event_id field to the pdu here because of signature and
		// hashes checks
		let Ok((event_id, value)) = gen_event_id_canonical_json(&response.event, &room_version_id) else {
			// Event could not be converted to canonical json
			return Err(Error::BadRequest(
				ErrorKind::InvalidParam,
				"Could not convert event to canonical json.",
			));
		};

		if *pdu.event_id != *event_id {
			warn!(
				"Server {} changed invite event, that's not allowed in the spec: ours: {pdu_json:?}, theirs: {value:?}",
				user_id.server_name(),
			);
		}

		let origin: OwnedServerName = serde_json::from_value(
			serde_json::to_value(
				value
					.get("origin")
					.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Event needs an origin field."))?,
			)
			.expect("CanonicalJson is valid json value"),
		)
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Origin field is invalid."))?;

		services
			.server_keys
			.fetch_required_signing_keys([&value], &pub_key_map)
			.await?;

		let pdu_id: Vec<u8> = services
			.rooms
			.event_handler
			.handle_incoming_pdu(&origin, room_id, &event_id, value, true, &pub_key_map)
			.await?
			.ok_or(Error::BadRequest(
				ErrorKind::InvalidParam,
				"Could not accept incoming PDU as timeline event.",
			))?;

		services.sending.send_pdu_room(room_id, &pdu_id).await?;
		return Ok(());
	}

	if !services
		.rooms
		.state_cache
		.is_joined(sender_user, room_id)
		.await
	{
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"You don't have permission to view this room.",
		));
	}

	let state_lock = services.rooms.state.mutex.lock(room_id).await;

	let content = RoomMemberEventContent {
		displayname: services.users.displayname(user_id).await.ok(),
		avatar_url: services.users.avatar_url(user_id).await.ok(),
		blurhash: services.users.blurhash(user_id).await.ok(),
		is_direct: Some(is_direct),
		reason,
		..RoomMemberEventContent::new(MembershipState::Invite)
	};

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(user_id.to_string(), &content),
			sender_user,
			room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	Ok(())
}

// Make a user leave all their joined rooms, forgets all rooms, and ignores
// errors
pub async fn leave_all_rooms(services: &Services, user_id: &UserId) {
	let rooms_joined = services
		.rooms
		.state_cache
		.rooms_joined(user_id)
		.map(ToOwned::to_owned);

	let rooms_invited = services
		.rooms
		.state_cache
		.rooms_invited(user_id)
		.map(|(r, _)| r);

	let all_rooms: Vec<_> = rooms_joined.chain(rooms_invited).collect().await;

	for room_id in all_rooms {
		// ignore errors
		if let Err(e) = leave_room(services, user_id, &room_id, None).await {
			warn!(%room_id, %user_id, %e, "Failed to leave room");
		}

		services.rooms.state_cache.forget(&room_id, user_id);
	}
}

pub async fn leave_room(services: &Services, user_id: &UserId, room_id: &RoomId, reason: Option<String>) -> Result<()> {
	//use conduit::utils::stream::OptionStream;
	use futures::TryFutureExt;

	// Ask a remote server if we don't have this room
	if !services
		.rooms
		.state_cache
		.server_in_room(services.globals.server_name(), room_id)
		.await
	{
		if let Err(e) = remote_leave_room(services, user_id, room_id).await {
			warn!("Failed to leave room {user_id} remotely: {e}");
			// Don't tell the client about this error
		}

		let last_state = services
			.rooms
			.state_cache
			.invite_state(user_id, room_id)
			.map_err(|_| services.rooms.state_cache.left_state(user_id, room_id))
			.await
			.ok();

		// We always drop the invite, we can't rely on other servers
		services
			.rooms
			.state_cache
			.update_membership(
				room_id,
				user_id,
				RoomMemberEventContent::new(MembershipState::Leave),
				user_id,
				last_state,
				None,
				true,
			)
			.await?;
	} else {
		let state_lock = services.rooms.state.mutex.lock(room_id).await;

		let Ok(event) = services
			.rooms
			.state_accessor
			.room_state_get_content::<RoomMemberEventContent>(room_id, &StateEventType::RoomMember, user_id.as_str())
			.await
		else {
			// Fix for broken rooms
			error!("Trying to leave a room you are not a member of.");

			services
				.rooms
				.state_cache
				.update_membership(
					room_id,
					user_id,
					RoomMemberEventContent::new(MembershipState::Leave),
					user_id,
					None,
					None,
					true,
				)
				.await?;

			return Ok(());
		};

		services
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder::state(
					user_id.to_string(),
					&RoomMemberEventContent {
						membership: MembershipState::Leave,
						reason,
						..event
					},
				),
				user_id,
				room_id,
				&state_lock,
			)
			.await?;
	}

	Ok(())
}

async fn remote_leave_room(services: &Services, user_id: &UserId, room_id: &RoomId) -> Result<()> {
	let mut make_leave_response_and_server = Err!(BadServerResponse("No server available to assist in leaving."));

	let invite_state = services
		.rooms
		.state_cache
		.invite_state(user_id, room_id)
		.await
		.map_err(|_| err!(Request(BadState("User is not invited."))))?;

	let mut servers: HashSet<OwnedServerName> = services
		.rooms
		.state_cache
		.servers_invite_via(room_id)
		.map(ToOwned::to_owned)
		.collect()
		.await;

	servers.extend(
		invite_state
			.iter()
			.filter_map(|event| event.get_field("sender").ok().flatten())
			.filter_map(|sender: &str| UserId::parse(sender).ok())
			.map(|user| user.server_name().to_owned()),
	);

	debug!("servers in remote_leave_room: {servers:?}");

	for remote_server in servers {
		let make_leave_response = services
			.sending
			.send_federation_request(
				&remote_server,
				federation::membership::prepare_leave_event::v1::Request {
					room_id: room_id.to_owned(),
					user_id: user_id.to_owned(),
				},
			)
			.await;

		make_leave_response_and_server = make_leave_response.map(|r| (r, remote_server));

		if make_leave_response_and_server.is_ok() {
			break;
		}
	}

	let (make_leave_response, remote_server) = make_leave_response_and_server?;

	let room_version_id = match make_leave_response.room_version {
		Some(version)
			if services
				.globals
				.supported_room_versions()
				.contains(&version) =>
		{
			version
		},
		_ => return Err!(BadServerResponse("Room version is not supported")),
	};

	let mut leave_event_stub = serde_json::from_str::<CanonicalJsonObject>(make_leave_response.event.get())
		.map_err(|e| err!(BadServerResponse("Invalid make_leave event json received from server: {e:?}")))?;

	// TODO: Is origin needed?
	leave_event_stub.insert(
		"origin".to_owned(),
		CanonicalJsonValue::String(services.globals.server_name().as_str().to_owned()),
	);
	leave_event_stub.insert(
		"origin_server_ts".to_owned(),
		CanonicalJsonValue::Integer(
			utils::millis_since_unix_epoch()
				.try_into()
				.expect("Timestamp is valid js_int value"),
		),
	);

	// room v3 and above removed the "event_id" field from remote PDU format
	match room_version_id {
		RoomVersionId::V1 | RoomVersionId::V2 => {},
		_ => {
			leave_event_stub.remove("event_id");
		},
	};

	// In order to create a compatible ref hash (EventID) the `hashes` field needs
	// to be present
	ruma::signatures::hash_and_sign_event(
		services.globals.server_name().as_str(),
		services.globals.keypair(),
		&mut leave_event_stub,
		&room_version_id,
	)
	.expect("event is valid, we just created it");

	// Generate event id
	let event_id = EventId::parse(format!(
		"${}",
		ruma::signatures::reference_hash(&leave_event_stub, &room_version_id)
			.expect("ruma can calculate reference hashes")
	))
	.expect("ruma's reference hashes are valid event ids");

	// Add event_id back
	leave_event_stub.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.as_str().to_owned()));

	// It has enough fields to be called a proper event now
	let leave_event = leave_event_stub;

	services
		.sending
		.send_federation_request(
			&remote_server,
			federation::membership::create_leave_event::v2::Request {
				room_id: room_id.to_owned(),
				event_id,
				pdu: services
					.sending
					.convert_to_outgoing_federation_event(leave_event.clone())
					.await,
			},
		)
		.await?;

	Ok(())
}
