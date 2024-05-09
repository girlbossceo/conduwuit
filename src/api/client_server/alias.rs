use rand::seq::SliceRandom;
use ruma::{
	api::{
		appservice,
		client::{
			alias::{create_alias, delete_alias, get_alias},
			error::ErrorKind,
		},
		federation,
	},
	OwnedRoomAliasId, OwnedRoomId, OwnedServerName,
};
use tracing::debug;

use crate::{
	debug_info, debug_warn,
	service::{appservice::RegistrationInfo, server_is_ours},
	services, Error, Result, Ruma,
};

/// # `PUT /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Creates a new room alias on this server.
pub(crate) async fn create_alias_route(body: Ruma<create_alias::v3::Request>) -> Result<create_alias::v3::Response> {
	alias_checks(&body.room_alias, &body.appservice_info).await?;

	// this isn't apart of alias_checks or delete alias route because we should
	// allow removing forbidden room aliases
	if services()
		.globals
		.forbidden_alias_names()
		.is_match(body.room_alias.alias())
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Room alias is forbidden."));
	}

	if services()
		.rooms
		.alias
		.resolve_local_alias(&body.room_alias)?
		.is_some()
	{
		return Err(Error::Conflict("Alias already exists."));
	}

	if services()
		.rooms
		.alias
		.set_alias(&body.room_alias, &body.room_id)
		.is_err()
	{
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Invalid room alias. Alias must be in the form of '#localpart:server_name'",
		));
	};

	Ok(create_alias::v3::Response::new())
}

/// # `DELETE /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Deletes a room alias from this server.
///
/// - TODO: additional access control checks
/// - TODO: Update canonical alias event
pub(crate) async fn delete_alias_route(body: Ruma<delete_alias::v3::Request>) -> Result<delete_alias::v3::Response> {
	alias_checks(&body.room_alias, &body.appservice_info).await?;
	if services()
		.rooms
		.alias
		.resolve_local_alias(&body.room_alias)?
		.is_none()
	{
		return Err(Error::BadRequest(ErrorKind::NotFound, "Alias does not exist."));
	}

	if services()
		.rooms
		.alias
		.remove_alias(&body.room_alias)
		.is_err()
	{
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Invalid room alias. Alias must be in the form of '#localpart:server_name'",
		));
	};

	// TODO: update alt_aliases?

	Ok(delete_alias::v3::Response::new())
}

/// # `GET /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Resolve an alias locally or over federation.
pub(crate) async fn get_alias_route(body: Ruma<get_alias::v3::Request>) -> Result<get_alias::v3::Response> {
	get_alias_helper(body.body.room_alias, None).await
}

pub async fn get_alias_helper(
	room_alias: OwnedRoomAliasId, servers: Option<Vec<OwnedServerName>>,
) -> Result<get_alias::v3::Response> {
	debug!("get_alias_helper servers: {servers:?}");
	if !server_is_ours(room_alias.server_name())
		&& (!servers
			.as_ref()
			.is_some_and(|servers| servers.contains(&services().globals.server_name().to_owned()))
			|| servers.as_ref().is_none())
	{
		let mut response = services()
			.sending
			.send_federation_request(
				room_alias.server_name(),
				federation::query::get_room_information::v1::Request {
					room_alias: room_alias.clone(),
				},
			)
			.await;

		debug_info!("room alias server_name get_alias_helper response: {response:?}");

		if let Err(ref e) = response {
			debug_info!(
				"Server {} of the original room alias failed to assist in resolving room alias: {e}",
				room_alias.server_name()
			);
		}

		if response.as_ref().is_ok_and(|resp| resp.servers.is_empty()) || response.as_ref().is_err() {
			if let Some(servers) = servers {
				for server in servers {
					response = services()
						.sending
						.send_federation_request(
							&server,
							federation::query::get_room_information::v1::Request {
								room_alias: room_alias.clone(),
							},
						)
						.await;
					debug_info!("Got response from server {server} for room aliases: {response:?}");

					if let Ok(ref response) = response {
						if !response.servers.is_empty() {
							break;
						}
						debug_warn!(
							"Server {server} responded with room aliases, but was empty? Response: {response:?}"
						);
					}
				}
			}
		}

		if let Ok(response) = response {
			let room_id = response.room_id;

			let mut pre_servers = response.servers;
			// since the room alis server responded, insert it into the list
			pre_servers.push(room_alias.server_name().into());

			let servers = room_available_servers(&room_id, &room_alias, &Some(pre_servers));
			debug_warn!(
				"room alias servers from federation response for room ID {room_id} and room alias {room_alias}: \
				 {servers:?}"
			);

			return Ok(get_alias::v3::Response::new(room_id, servers));
		}

		return Err(Error::BadRequest(
			ErrorKind::NotFound,
			"No servers could assist in resolving the room alias",
		));
	}

	let mut room_id = None;
	match services().rooms.alias.resolve_local_alias(&room_alias)? {
		Some(r) => room_id = Some(r),
		None => {
			for appservice in services().appservice.read().await.values() {
				if appservice.aliases.is_match(room_alias.as_str())
					&& matches!(
						services()
							.sending
							.send_appservice_request(
								appservice.registration.clone(),
								appservice::query::query_room_alias::v1::Request {
									room_alias: room_alias.clone(),
								},
							)
							.await,
						Ok(Some(_opt_result))
					) {
					room_id = Some(
						services()
							.rooms
							.alias
							.resolve_local_alias(&room_alias)?
							.ok_or_else(|| Error::bad_config("Room does not exist."))?,
					);
					break;
				}
			}
		},
	};

	let Some(room_id) = room_id else {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room with alias not found."));
	};

	let servers = room_available_servers(&room_id, &room_alias, &None);

	debug_warn!("room alias servers for room ID {room_id} and room alias {room_alias}");

	Ok(get_alias::v3::Response::new(room_id, servers))
}

fn room_available_servers(
	room_id: &OwnedRoomId, room_alias: &OwnedRoomAliasId, pre_servers: &Option<Vec<OwnedServerName>>,
) -> Vec<OwnedServerName> {
	// find active servers in room state cache to suggest
	let mut servers: Vec<OwnedServerName> = services()
		.rooms
		.state_cache
		.room_servers(room_id)
		.filter_map(Result::ok)
		.collect();

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
		.position(|server_name| server_is_ours(server_name))
	{
		servers.remove(server_index);
		servers.insert(0, services().globals.server_name().to_owned());
	} else if let Some(alias_server_index) = servers
		.iter()
		.position(|server| server == room_alias.server_name())
	{
		servers.remove(alias_server_index);
		servers.insert(0, room_alias.server_name().into());
	}

	servers
}

async fn alias_checks(room_alias: &OwnedRoomAliasId, appservice_info: &Option<RegistrationInfo>) -> Result<()> {
	if !server_is_ours(room_alias.server_name()) {
		return Err(Error::BadRequest(ErrorKind::InvalidParam, "Alias is from another server."));
	}

	if let Some(ref info) = appservice_info {
		if !info.aliases.is_match(room_alias.as_str()) {
			return Err(Error::BadRequest(ErrorKind::Exclusive, "Room alias is not in namespace."));
		}
	} else if services().appservice.is_exclusive_alias(room_alias).await {
		return Err(Error::BadRequest(ErrorKind::Exclusive, "Room alias reserved by appservice."));
	}

	Ok(())
}
