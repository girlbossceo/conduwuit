use axum::extract::State;
use axum_client_ip::InsecureClientIp;
use conduit::{warn, Err};
use ruma::{
	api::{
		client::{
			error::ErrorKind,
			membership::mutual_rooms,
			profile::{delete_timezone_key, get_timezone_key, set_timezone_key},
			room::get_summary,
		},
		federation,
	},
	events::room::member::MembershipState,
	presence::PresenceState,
	OwnedRoomId,
};

use crate::{Error, Result, Ruma, RumaResponse};

/// # `GET /_matrix/client/unstable/uk.half-shot.msc2666/user/mutual_rooms`
///
/// Gets all the rooms the sender shares with the specified user.
///
/// TODO: Implement pagination, currently this just returns everything
///
/// An implementation of [MSC2666](https://github.com/matrix-org/matrix-spec-proposals/pull/2666)
#[tracing::instrument(skip_all, fields(%client), name = "mutual_rooms")]
pub(crate) async fn get_mutual_rooms_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<mutual_rooms::unstable::Request>,
) -> Result<mutual_rooms::unstable::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if sender_user == &body.user_id {
		return Err(Error::BadRequest(
			ErrorKind::Unknown,
			"You cannot request rooms in common with yourself.",
		));
	}

	if !services.users.exists(&body.user_id)? {
		return Ok(mutual_rooms::unstable::Response {
			joined: vec![],
			next_batch_token: None,
		});
	}

	let mutual_rooms: Vec<OwnedRoomId> = services
		.rooms
		.user
		.get_shared_rooms(vec![sender_user.clone(), body.user_id.clone()])?
		.filter_map(Result::ok)
		.collect();

	Ok(mutual_rooms::unstable::Response {
		joined: mutual_rooms,
		next_batch_token: None,
	})
}

/// # `GET /_matrix/client/unstable/im.nheko.summary/rooms/{roomIdOrAlias}/summary`
///
/// Returns a short description of the state of a room.
///
/// This is the "wrong" endpoint that some implementations/clients may use
/// according to the MSC. Request and response bodies are the same as
/// `get_room_summary`.
///
/// An implementation of [MSC3266](https://github.com/matrix-org/matrix-spec-proposals/pull/3266)
pub(crate) async fn get_room_summary_legacy(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_summary::msc3266::Request>,
) -> Result<RumaResponse<get_summary::msc3266::Response>> {
	get_room_summary(State(services), InsecureClientIp(client), body)
		.await
		.map(RumaResponse)
}

/// # `GET /_matrix/client/unstable/im.nheko.summary/summary/{roomIdOrAlias}`
///
/// Returns a short description of the state of a room.
///
/// TODO: support fetching remote room info if we don't know the room
///
/// An implementation of [MSC3266](https://github.com/matrix-org/matrix-spec-proposals/pull/3266)
#[tracing::instrument(skip_all, fields(%client), name = "room_summary")]
pub(crate) async fn get_room_summary(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_summary::msc3266::Request>,
) -> Result<get_summary::msc3266::Response> {
	let sender_user = body.sender_user.as_ref();

	let room_id = services.rooms.alias.resolve(&body.room_id_or_alias).await?;

	if !services.rooms.metadata.exists(&room_id)? {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server"));
	}

	if sender_user.is_none()
		&& !services
			.rooms
			.state_accessor
			.is_world_readable(&room_id)
			.unwrap_or(false)
	{
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"Room is not world readable, authentication is required",
		));
	}

	Ok(get_summary::msc3266::Response {
		room_id: room_id.clone(),
		canonical_alias: services
			.rooms
			.state_accessor
			.get_canonical_alias(&room_id)
			.unwrap_or(None),
		avatar_url: services
			.rooms
			.state_accessor
			.get_avatar(&room_id)?
			.into_option()
			.unwrap_or_default()
			.url,
		guest_can_join: services.rooms.state_accessor.guest_can_join(&room_id)?,
		name: services
			.rooms
			.state_accessor
			.get_name(&room_id)
			.unwrap_or(None),
		num_joined_members: services
			.rooms
			.state_cache
			.room_joined_count(&room_id)
			.unwrap_or_default()
			.unwrap_or_else(|| {
				warn!("Room {room_id} has no member count");
				0
			})
			.try_into()
			.expect("user count should not be that big"),
		topic: services
			.rooms
			.state_accessor
			.get_room_topic(&room_id)
			.unwrap_or(None),
		world_readable: services
			.rooms
			.state_accessor
			.is_world_readable(&room_id)
			.unwrap_or(false),
		join_rule: services.rooms.state_accessor.get_join_rule(&room_id)?.0,
		room_type: services.rooms.state_accessor.get_room_type(&room_id)?,
		room_version: Some(services.rooms.state.get_room_version(&room_id)?),
		membership: if let Some(sender_user) = sender_user {
			services
				.rooms
				.state_accessor
				.get_member(&room_id, sender_user)?
				.map_or_else(|| Some(MembershipState::Leave), |content| Some(content.membership))
		} else {
			None
		},
		encryption: services
			.rooms
			.state_accessor
			.get_room_encryption(&room_id)
			.unwrap_or_else(|_e| None),
	})
}

/// # `DELETE /_matrix/client/unstable/uk.tcpip.msc4133/profile/:user_id/us.cloke.msc4175.tz`
///
/// Deletes the `tz` (timezone) of a user, as per MSC4133 and MSC4175.
///
/// - Also makes sure other users receive the update using presence EDUs
pub(crate) async fn delete_timezone_key_route(
	State(services): State<crate::State>, body: Ruma<delete_timezone_key::unstable::Request>,
) -> Result<delete_timezone_key::unstable::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if *sender_user != body.user_id && body.appservice_info.is_none() {
		return Err!(Request(Forbidden("You cannot update the profile of another user")));
	}

	services.users.set_timezone(&body.user_id, None).await?;

	if services.globals.allow_local_presence() {
		// Presence update
		services
			.presence
			.ping_presence(&body.user_id, &PresenceState::Online)?;
	}

	Ok(delete_timezone_key::unstable::Response {})
}

/// # `PUT /_matrix/client/unstable/uk.tcpip.msc4133/profile/:user_id/us.cloke.msc4175.tz`
///
/// Updates the `tz` (timezone) of a user, as per MSC4133 and MSC4175.
///
/// - Also makes sure other users receive the update using presence EDUs
pub(crate) async fn set_timezone_key_route(
	State(services): State<crate::State>, body: Ruma<set_timezone_key::unstable::Request>,
) -> Result<set_timezone_key::unstable::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if *sender_user != body.user_id && body.appservice_info.is_none() {
		return Err!(Request(Forbidden("You cannot update the profile of another user")));
	}

	services
		.users
		.set_timezone(&body.user_id, body.tz.clone())
		.await?;

	if services.globals.allow_local_presence() {
		// Presence update
		services
			.presence
			.ping_presence(&body.user_id, &PresenceState::Online)?;
	}

	Ok(set_timezone_key::unstable::Response {})
}

/// # `GET /_matrix/client/unstable/uk.tcpip.msc4133/profile/:user_id/us.cloke.msc4175.tz`
///
/// Returns the `timezone` of the user as per MSC4133 and MSC4175.
///
/// - If user is on another server and we do not have a local copy already fetch
///   `timezone` over federation
pub(crate) async fn get_timezone_key_route(
	State(services): State<crate::State>, body: Ruma<get_timezone_key::unstable::Request>,
) -> Result<get_timezone_key::unstable::Response> {
	if !services.globals.user_is_local(&body.user_id) {
		// Create and update our local copy of the user
		if let Ok(response) = services
			.sending
			.send_federation_request(
				body.user_id.server_name(),
				federation::query::get_profile_information::v1::Request {
					user_id: body.user_id.clone(),
					field: None, // we want the full user's profile to update locally as well
				},
			)
			.await
		{
			if !services.users.exists(&body.user_id)? {
				services.users.create(&body.user_id, None)?;
			}

			services
				.users
				.set_displayname(&body.user_id, response.displayname.clone())
				.await?;
			services
				.users
				.set_avatar_url(&body.user_id, response.avatar_url.clone())
				.await?;
			services
				.users
				.set_blurhash(&body.user_id, response.blurhash.clone())
				.await?;
			services
				.users
				.set_timezone(&body.user_id, response.tz.clone())
				.await?;

			return Ok(get_timezone_key::unstable::Response {
				tz: response.tz,
			});
		}
	}

	if !services.users.exists(&body.user_id)? {
		// Return 404 if this user doesn't exist and we couldn't fetch it over
		// federation
		return Err(Error::BadRequest(ErrorKind::NotFound, "Profile was not found."));
	}

	Ok(get_timezone_key::unstable::Response {
		tz: services.users.timezone(&body.user_id)?,
	})
}
