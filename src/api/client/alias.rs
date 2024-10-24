use axum::extract::State;
use conduit::{debug, Err, Result};
use futures::StreamExt;
use rand::seq::SliceRandom;
use ruma::{
	api::client::alias::{create_alias, delete_alias, get_alias},
	OwnedServerName, RoomAliasId, RoomId,
};
use service::Services;

use crate::Ruma;

/// # `PUT /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Creates a new room alias on this server.
pub(crate) async fn create_alias_route(
	State(services): State<crate::State>, body: Ruma<create_alias::v3::Request>,
) -> Result<create_alias::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	services
		.rooms
		.alias
		.appservice_checks(&body.room_alias, &body.appservice_info)
		.await?;

	// this isn't apart of alias_checks or delete alias route because we should
	// allow removing forbidden room aliases
	if services
		.globals
		.forbidden_alias_names()
		.is_match(body.room_alias.alias())
	{
		return Err!(Request(Forbidden("Room alias is forbidden.")));
	}

	if services
		.rooms
		.alias
		.resolve_local_alias(&body.room_alias)
		.await
		.is_ok()
	{
		return Err!(Conflict("Alias already exists."));
	}

	services
		.rooms
		.alias
		.set_alias(&body.room_alias, &body.room_id, sender_user)?;

	Ok(create_alias::v3::Response::new())
}

/// # `DELETE /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Deletes a room alias from this server.
///
/// - TODO: Update canonical alias event
pub(crate) async fn delete_alias_route(
	State(services): State<crate::State>, body: Ruma<delete_alias::v3::Request>,
) -> Result<delete_alias::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	services
		.rooms
		.alias
		.appservice_checks(&body.room_alias, &body.appservice_info)
		.await?;

	services
		.rooms
		.alias
		.remove_alias(&body.room_alias, sender_user)
		.await?;

	// TODO: update alt_aliases?

	Ok(delete_alias::v3::Response::new())
}

/// # `GET /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Resolve an alias locally or over federation.
pub(crate) async fn get_alias_route(
	State(services): State<crate::State>, body: Ruma<get_alias::v3::Request>,
) -> Result<get_alias::v3::Response> {
	let room_alias = body.body.room_alias;
	let servers = None;

	let Ok((room_id, pre_servers)) = services
		.rooms
		.alias
		.resolve_alias(&room_alias, servers.as_ref())
		.await
	else {
		return Err!(Request(NotFound("Room with alias not found.")));
	};

	let servers = room_available_servers(&services, &room_id, &room_alias, &pre_servers).await;
	debug!(?room_alias, ?room_id, "available servers: {servers:?}");

	Ok(get_alias::v3::Response::new(room_id, servers))
}

async fn room_available_servers(
	services: &Services, room_id: &RoomId, room_alias: &RoomAliasId, pre_servers: &Option<Vec<OwnedServerName>>,
) -> Vec<OwnedServerName> {
	// find active servers in room state cache to suggest
	let mut servers: Vec<OwnedServerName> = services
		.rooms
		.state_cache
		.room_servers(room_id)
		.map(ToOwned::to_owned)
		.collect()
		.await;

	// push any servers we want in the list already (e.g. responded remote alias
	// servers, room alias server itself)
	if let Some(pre_servers) = pre_servers {
		servers.extend(pre_servers.clone());
	};

	servers.sort_unstable();
	servers.dedup();

	// shuffle list of servers randomly after sort and dedupe
	servers.shuffle(&mut rand::thread_rng());

	// insert our server as the very first choice if in list, else check if we can
	// prefer the room alias server first
	if let Some(server_index) = servers
		.iter()
		.position(|server_name| services.globals.server_is_ours(server_name))
	{
		servers.swap_remove(server_index);
		servers.insert(0, services.globals.server_name().to_owned());
	} else if let Some(alias_server_index) = servers
		.iter()
		.position(|server| server == room_alias.server_name())
	{
		servers.swap_remove(alias_server_index);
		servers.insert(0, room_alias.server_name().into());
	}

	servers
}
