use axum_client_ip::InsecureClientIp;
use conduit::warn;
use ruma::{
	api::client::{error::ErrorKind, membership::mutual_rooms, room::get_summary},
	events::room::member::MembershipState,
	OwnedRoomId,
};

use crate::{services, Error, Result, Ruma, RumaResponse};

/// # `GET /_matrix/client/unstable/uk.half-shot.msc2666/user/mutual_rooms`
///
/// Gets all the rooms the sender shares with the specified user.
///
/// TODO: Implement pagination, currently this just returns everything
///
/// An implementation of [MSC2666](https://github.com/matrix-org/matrix-spec-proposals/pull/2666)
#[tracing::instrument(skip_all, fields(%client), name = "mutual_rooms")]
pub(crate) async fn get_mutual_rooms_route(
	InsecureClientIp(client): InsecureClientIp, body: Ruma<mutual_rooms::unstable::Request>,
) -> Result<mutual_rooms::unstable::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if sender_user == &body.user_id {
		return Err(Error::BadRequest(
			ErrorKind::Unknown,
			"You cannot request rooms in common with yourself.",
		));
	}

	if !services().users.exists(&body.user_id)? {
		return Ok(mutual_rooms::unstable::Response {
			joined: vec![],
			next_batch_token: None,
		});
	}

	let mutual_rooms: Vec<OwnedRoomId> = services()
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
	InsecureClientIp(client): InsecureClientIp, body: Ruma<get_summary::msc3266::Request>,
) -> Result<RumaResponse<get_summary::msc3266::Response>> {
	get_room_summary(InsecureClientIp(client), body)
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
	InsecureClientIp(client): InsecureClientIp, body: Ruma<get_summary::msc3266::Request>,
) -> Result<get_summary::msc3266::Response> {
	let sender_user = body.sender_user.as_ref();

	let room_id = services()
		.rooms
		.alias
		.resolve(&body.room_id_or_alias)
		.await?;

	if !services().rooms.metadata.exists(&room_id)? {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server"));
	}

	if sender_user.is_none()
		&& !services()
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
		canonical_alias: services()
			.rooms
			.state_accessor
			.get_canonical_alias(&room_id)
			.unwrap_or(None),
		avatar_url: services()
			.rooms
			.state_accessor
			.get_avatar(&room_id)?
			.into_option()
			.unwrap_or_default()
			.url,
		guest_can_join: services().rooms.state_accessor.guest_can_join(&room_id)?,
		name: services()
			.rooms
			.state_accessor
			.get_name(&room_id)
			.unwrap_or(None),
		num_joined_members: services()
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
		topic: services()
			.rooms
			.state_accessor
			.get_room_topic(&room_id)
			.unwrap_or(None),
		world_readable: services()
			.rooms
			.state_accessor
			.is_world_readable(&room_id)
			.unwrap_or(false),
		join_rule: services().rooms.state_accessor.get_join_rule(&room_id)?.0,
		room_type: services().rooms.state_accessor.get_room_type(&room_id)?,
		room_version: Some(services().rooms.state.get_room_version(&room_id)?),
		membership: if let Some(sender_user) = sender_user {
			services()
				.rooms
				.state_accessor
				.get_member(&room_id, sender_user)?
				.map_or_else(|| Some(MembershipState::Leave), |content| Some(content.membership))
		} else {
			None
		},
		encryption: services()
			.rooms
			.state_accessor
			.get_room_encryption(&room_id)
			.unwrap_or_else(|_e| None),
	})
}
