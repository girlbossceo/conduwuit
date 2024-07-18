use conduit::{debug, debug_warn, Error, Result};
use ruma::{
	api::{client::error::ErrorKind, federation},
	OwnedRoomId, OwnedServerName, RoomAliasId,
};

impl super::Service {
	pub(super) async fn remote_resolve(
		&self, room_alias: &RoomAliasId, servers: Option<&Vec<OwnedServerName>>,
	) -> Result<(OwnedRoomId, Option<Vec<OwnedServerName>>)> {
		debug!(?room_alias, ?servers, "resolve");

		let mut response = self
			.services
			.sending
			.send_federation_request(
				room_alias.server_name(),
				federation::query::get_room_information::v1::Request {
					room_alias: room_alias.to_owned(),
				},
			)
			.await;

		debug!("room alias server_name get_alias_helper response: {response:?}");

		if let Err(ref e) = response {
			debug_warn!(
				"Server {} of the original room alias failed to assist in resolving room alias: {e}",
				room_alias.server_name(),
			);
		}

		if response.as_ref().is_ok_and(|resp| resp.servers.is_empty()) || response.as_ref().is_err() {
			if let Some(servers) = servers {
				for server in servers {
					response = self
						.services
						.sending
						.send_federation_request(
							server,
							federation::query::get_room_information::v1::Request {
								room_alias: room_alias.to_owned(),
							},
						)
						.await;
					debug!("Got response from server {server} for room aliases: {response:?}");

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

			return Ok((room_id, Some(pre_servers)));
		}

		Err(Error::BadRequest(
			ErrorKind::NotFound,
			"No servers could assist in resolving the room alias",
		))
	}
}
