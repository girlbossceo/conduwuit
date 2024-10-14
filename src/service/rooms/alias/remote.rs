use std::iter::once;

use conduit::{debug, debug_error, err, implement, Result};
use federation::query::get_room_information::v1::Response;
use ruma::{api::federation, OwnedRoomId, OwnedServerName, RoomAliasId, ServerName};

#[implement(super::Service)]
pub(super) async fn remote_resolve(
	&self, room_alias: &RoomAliasId, servers: Vec<OwnedServerName>,
) -> Result<(OwnedRoomId, Vec<OwnedServerName>)> {
	debug!(?room_alias, servers = ?servers, "resolve");
	let servers = once(room_alias.server_name())
		.map(ToOwned::to_owned)
		.chain(servers.into_iter());

	let mut resolved_servers = Vec::new();
	let mut resolved_room_id: Option<OwnedRoomId> = None;
	for server in servers {
		match self.remote_request(room_alias, &server).await {
			Err(e) => debug_error!("Failed to query for {room_alias:?} from {server}: {e}"),
			Ok(Response {
				room_id,
				servers,
			}) => {
				debug!("Server {server} answered with {room_id:?} for {room_alias:?} servers: {servers:?}");

				resolved_room_id.get_or_insert(room_id);
				add_server(&mut resolved_servers, server);

				if !servers.is_empty() {
					add_servers(&mut resolved_servers, servers);
					break;
				}
			},
		}
	}

	resolved_room_id
		.map(|room_id| (room_id, resolved_servers))
		.ok_or_else(|| err!(Request(NotFound("No servers could assist in resolving the room alias"))))
}

#[implement(super::Service)]
async fn remote_request(&self, room_alias: &RoomAliasId, server: &ServerName) -> Result<Response> {
	use federation::query::get_room_information::v1::Request;

	let request = Request {
		room_alias: room_alias.to_owned(),
	};

	self.services
		.sending
		.send_federation_request(server, request)
		.await
}

fn add_servers(servers: &mut Vec<OwnedServerName>, new: Vec<OwnedServerName>) {
	for server in new {
		add_server(servers, server);
	}
}

fn add_server(servers: &mut Vec<OwnedServerName>, server: OwnedServerName) {
	if !servers.contains(&server) {
		servers.push(server);
	}
}
