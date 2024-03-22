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
	OwnedRoomAliasId, OwnedServerName,
};

use crate::{services, Error, Result, Ruma};

/// # `PUT /_matrix/client/v3/directory/room/{roomAlias}`
///
/// Creates a new room alias on this server.
pub async fn create_alias_route(body: Ruma<create_alias::v3::Request>) -> Result<create_alias::v3::Response> {
	if body.room_alias.server_name() != services().globals.server_name() {
		return Err(Error::BadRequest(ErrorKind::InvalidParam, "Alias is from another server."));
	}

	if services().globals.forbidden_alias_names().is_match(body.room_alias.alias()) {
		return Err(Error::BadRequest(ErrorKind::Unknown, "Room alias is forbidden."));
	}

	if services().rooms.alias.resolve_local_alias(&body.room_alias)?.is_some() {
		return Err(Error::Conflict("Alias already exists."));
	}

	if services().rooms.alias.set_alias(&body.room_alias, &body.room_id).is_err() {
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
pub async fn delete_alias_route(body: Ruma<delete_alias::v3::Request>) -> Result<delete_alias::v3::Response> {
	if body.room_alias.server_name() != services().globals.server_name() {
		return Err(Error::BadRequest(ErrorKind::InvalidParam, "Alias is from another server."));
	}

	if services().rooms.alias.resolve_local_alias(&body.room_alias)?.is_none() {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Alias does not exist."));
	}

	if services().rooms.alias.remove_alias(&body.room_alias).is_err() {
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
pub async fn get_alias_route(body: Ruma<get_alias::v3::Request>) -> Result<get_alias::v3::Response> {
	get_alias_helper(body.body.room_alias).await
}

pub(crate) async fn get_alias_helper(room_alias: OwnedRoomAliasId) -> Result<get_alias::v3::Response> {
	if room_alias.server_name() != services().globals.server_name() {
		let response = services()
			.sending
			.send_federation_request(
				room_alias.server_name(),
				federation::query::get_room_information::v1::Request {
					room_alias: room_alias.clone(),
				},
			)
			.await?;

		let room_id = response.room_id;

		let mut servers = response.servers;

		// find active servers in room state cache to suggest
		for extra_servers in services().rooms.state_cache.room_servers(&room_id).filter_map(std::result::Result::ok) {
			servers.push(extra_servers);
		}

		// insert our server as the very first choice if in list
		if let Some(server_index) =
			servers.clone().into_iter().position(|server| server == services().globals.server_name())
		{
			servers.remove(server_index);
			servers.insert(0, services().globals.server_name().to_owned());
		}

		servers.sort_unstable();
		servers.dedup();

		// shuffle list of servers randomly after sort and dedupe
		servers.shuffle(&mut rand::thread_rng());

		return Ok(get_alias::v3::Response::new(room_id, servers));
	}

	let mut room_id = None;
	match services().rooms.alias.resolve_local_alias(&room_alias)? {
		Some(r) => room_id = Some(r),
		None => {
			for appservice in services().appservice.read().await.values() {
				if appservice.aliases.is_match(room_alias.as_str())
					&& if let Some(opt_result) = services()
						.sending
						.send_appservice_request(
							appservice.registration.clone(),
							appservice::query::query_room_alias::v1::Request {
								room_alias: room_alias.clone(),
							},
						)
						.await
					{
						opt_result.is_ok()
					} else {
						false
					} {
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

	let room_id = match room_id {
		Some(room_id) => room_id,
		None => return Err(Error::BadRequest(ErrorKind::NotFound, "Room with alias not found.")),
	};

	let mut servers: Vec<OwnedServerName> = Vec::new();

	// find active servers in room state cache to suggest
	for extra_servers in services().rooms.state_cache.room_servers(&room_id).filter_map(std::result::Result::ok) {
		servers.push(extra_servers);
	}

	// insert our server as the very first choice if in list
	if let Some(server_index) =
		servers.clone().into_iter().position(|server| server == services().globals.server_name())
	{
		servers.remove(server_index);
		servers.insert(0, services().globals.server_name().to_owned());
	}

	servers.sort_unstable();
	servers.dedup();

	// shuffle list of servers randomly after sort and dedupe
	servers.shuffle(&mut rand::thread_rng());

	Ok(get_alias::v3::Response::new(room_id, servers))
}
