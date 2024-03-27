use std::fmt::Write as _;

use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, OwnedUserId, RoomAliasId, RoomId, RoomOrAliasId};
use tracing::{debug, error, info};

use crate::{
	api::client_server::{get_alias_helper, leave_room},
	service::admin::{escape_html, Service},
	services, Result,
};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum RoomModerationCommand {
	/// - Bans a room from local users joining and evicts all our local users
	///   from the room. Also blocks any invites (local and remote) for the
	///   banned room.
	///
	/// Server admins (users in the conduwuit admin room) will not be evicted
	/// and server admins can still join the room. To evict admins too, use
	/// --force (also ignores errors) To disable incoming federation of the
	/// room, use --disable-federation
	BanRoom {
		#[arg(short, long)]
		/// Evicts admins out of the room and ignores any potential errors when
		/// making our local users leave the room
		force: bool,

		#[arg(long)]
		/// Disables incoming federation of the room after banning and evicting
		/// users
		disable_federation: bool,

		/// The room in the format of `!roomid:example.com` or a room alias in
		/// the format of `#roomalias:example.com`
		room: Box<RoomOrAliasId>,
	},

	/// - Bans a list of rooms from a newline delimited codeblock similar to
	///   `user deactivate-all`
	BanListOfRooms {
		#[arg(short, long)]
		/// Evicts admins out of the room and ignores any potential errors when
		/// making our local users leave the room
		force: bool,

		#[arg(long)]
		/// Disables incoming federation of the room after banning and evicting
		/// users
		disable_federation: bool,
	},

	/// - Unbans a room to allow local users to join again
	///
	/// To re-enable incoming federation of the room, use --enable-federation
	UnbanRoom {
		#[arg(long)]
		/// Enables incoming federation of the room after unbanning
		enable_federation: bool,

		/// The room in the format of `!roomid:example.com` or a room alias in
		/// the format of `#roomalias:example.com`
		room: Box<RoomOrAliasId>,
	},

	/// - List of all rooms we have banned
	ListBannedRooms,
}

pub(crate) async fn process(command: RoomModerationCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		RoomModerationCommand::BanRoom {
			force,
			room,
			disable_federation,
		} => {
			debug!("Got room alias or ID: {}", room);

			let admin_room_alias: Box<RoomAliasId> = format!("#admins:{}", services().globals.server_name())
				.try_into()
				.expect("#admins:server_name is a valid alias name");

			if let Some(admin_room_id) = Service::get_admin_room()? {
				if room.to_string().eq(&admin_room_id) || room.to_string().eq(&admin_room_alias) {
					return Ok(RoomMessageEventContent::text_plain("Not allowed to ban the admin room."));
				}
			}

			let room_id = if room.is_room_id() {
				let room_id = match RoomId::parse(&room) {
					Ok(room_id) => room_id,
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Failed to parse room ID {room}. Please note that this requires a full room ID \
							 (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias (`#roomalias:example.com`): {e}"
						)))
					},
				};

				debug!("Room specified is a room ID, banning room ID");

				services().rooms.metadata.ban_room(&room_id, true)?;

				room_id
			} else if room.is_room_alias_id() {
				let room_alias = match RoomAliasId::parse(&room) {
					Ok(room_alias) => room_alias,
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Failed to parse room ID {room}. Please note that this requires a full room ID \
							 (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias (`#roomalias:example.com`): {e}"
						)))
					},
				};

				debug!(
					"Room specified is not a room ID, attempting to resolve room alias to a room ID locally, if not \
					 using get_alias_helper to fetch room ID remotely"
				);

				let room_id = if let Some(room_id) = services().rooms.alias.resolve_local_alias(&room_alias)? {
					room_id
				} else {
					debug!(
						"We don't have this room alias to a room ID locally, attempting to fetch room ID over \
						 federation"
					);

					match get_alias_helper(room_alias).await {
						Ok(response) => {
							debug!("Got federation response fetching room ID for room {room}: {:?}", response);
							response.room_id
						},
						Err(e) => {
							return Ok(RoomMessageEventContent::text_plain(format!(
								"Failed to resolve room alias {room} to a room ID: {e}"
							)));
						},
					}
				};

				services().rooms.metadata.ban_room(&room_id, true)?;

				room_id
			} else {
				return Ok(RoomMessageEventContent::text_plain(
					"Room specified is not a room ID or room alias. Please note that this requires a full room ID \
					 (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias (`#roomalias:example.com`)",
				));
			};

			debug!("Making all users leave the room {}", &room);
			if force {
				for local_user in services()
					.rooms
					.state_cache
					.room_members(&room_id)
					.filter_map(|user| {
						user.ok().filter(|local_user| {
							local_user.server_name() == services().globals.server_name()
                            // additional wrapped check here is to avoid adding remote users
                            // who are in the admin room to the list of local users (would fail auth check)
                            && (local_user.server_name()
                                == services().globals.server_name()
                                && services()
                                    .users
                                    .is_admin(local_user)
                                    .unwrap_or(true)) // since this is a force
							                          // operation, assume user
							                          // is an admin if somehow
							                          // this fails
						})
					})
					.collect::<Vec<OwnedUserId>>()
				{
					debug!(
						"Attempting leave for user {} in room {} (forced, ignoring all errors, evicting admins too)",
						&local_user, &room_id
					);

					_ = leave_room(&local_user, &room_id, None).await;
				}
			} else {
				for local_user in services()
					.rooms
					.state_cache
					.room_members(&room_id)
					.filter_map(|user| {
						user.ok().filter(|local_user| {
							local_user.server_name() == services().globals.server_name()
                            // additional wrapped check here is to avoid adding remote users
                            // who are in the admin room to the list of local users (would fail auth check)
                            && (local_user.server_name()
                                == services().globals.server_name()
                                && !services()
                                    .users
                                    .is_admin(local_user)
                                    .unwrap_or(false))
						})
					})
					.collect::<Vec<OwnedUserId>>()
				{
					debug!("Attempting leave for user {} in room {}", &local_user, &room_id);
					if let Err(e) = leave_room(&local_user, &room_id, None).await {
						error!(
							"Error attempting to make local user {} leave room {} during room banning: {}",
							&local_user, &room_id, e
						);
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Error attempting to make local user {} leave room {} during room banning (room is still \
							 banned but not removing any more users): {}\nIf you would like to ignore errors, use \
							 --force",
							&local_user, &room_id, e
						)));
					}
				}
			}

			if disable_federation {
				services().rooms.metadata.disable_room(&room_id, true)?;
				return Ok(RoomMessageEventContent::text_plain(
					"Room banned, removed all our local users, and disabled incoming federation with room.",
				));
			}

			Ok(RoomMessageEventContent::text_plain(
				"Room banned and removed all our local users, use disable-room to stop receiving new inbound \
				 federation events as well if needed.",
			))
		},
		RoomModerationCommand::BanListOfRooms {
			force,
			disable_federation,
		} => {
			if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
				let rooms_s = body.clone().drain(1..body.len() - 1).collect::<Vec<_>>();

				let mut room_ban_count = 0;
				let mut room_ids: Vec<&RoomId> = Vec::new();

				for &room_id in &rooms_s {
					match <&RoomId>::try_from(room_id) {
						Ok(owned_room_id) => {
							// silently ignore deleting admin room
							if let Some(admin_room_id) = Service::get_admin_room()? {
								if owned_room_id.eq(&admin_room_id) {
									info!("User specified admin room in bulk ban list, ignoring");
									continue;
								}
							}

							room_ids.push(owned_room_id);
						},
						Err(e) => {
							if force {
								// ignore rooms we failed to parse if we're force deleting
								error!(
									"Error parsing room ID {room_id} during bulk room banning, ignoring error and \
									 logging here: {e}"
								);
								continue;
							}

							return Ok(RoomMessageEventContent::text_plain(format!(
								"{room_id} is not a valid room ID, please fix the list and try again: {e}"
							)));
						},
					}
				}

				for room_id in room_ids {
					if services().rooms.metadata.ban_room(room_id, true).is_ok() {
						debug!("Banned {room_id} successfully");
						room_ban_count += 1;
					}

					debug!("Making all users leave the room {}", &room_id);
					if force {
						for local_user in services()
							.rooms
							.state_cache
							.room_members(room_id)
							.filter_map(|user| {
								user.ok().filter(|local_user| {
									local_user.server_name() == services().globals.server_name()
                                    // additional wrapped check here is to avoid adding remote users
                                    // who are in the admin room to the list of local users (would fail auth check)
                                    && (local_user.server_name()
                                        == services().globals.server_name()
                                        && services()
                                            .users
                                            .is_admin(local_user)
                                            .unwrap_or(true)) // since this is a
									                          // force operation,
									                          // assume user is
									                          // an admin if
									                          // somehow this
									                          // fails
								})
							})
							.collect::<Vec<OwnedUserId>>()
						{
							debug!(
								"Attempting leave for user {} in room {} (forced, ignoring all errors, evicting \
								 admins too)",
								&local_user, room_id
							);
							_ = leave_room(&local_user, room_id, None).await;
						}
					} else {
						for local_user in services()
							.rooms
							.state_cache
							.room_members(room_id)
							.filter_map(|user| {
								user.ok().filter(|local_user| {
									local_user.server_name() == services().globals.server_name()
                                    // additional wrapped check here is to avoid adding remote users
                                    // who are in the admin room to the list of local users (would fail auth check)
                                    && (local_user.server_name()
                                        == services().globals.server_name()
                                        && !services()
                                            .users
                                            .is_admin(local_user)
                                            .unwrap_or(false))
								})
							})
							.collect::<Vec<OwnedUserId>>()
						{
							debug!("Attempting leave for user {} in room {}", &local_user, &room_id);
							if let Err(e) = leave_room(&local_user, room_id, None).await {
								error!(
									"Error attempting to make local user {} leave room {} during bulk room banning: {}",
									&local_user, &room_id, e
								);
								return Ok(RoomMessageEventContent::text_plain(format!(
									"Error attempting to make local user {} leave room {} during room banning (room \
									 is still banned but not removing any more users and not banning any more rooms): \
									 {}\nIf you would like to ignore errors, use --force",
									&local_user, &room_id, e
								)));
							}
						}
					}

					if disable_federation {
						services().rooms.metadata.disable_room(room_id, true)?;
					}
				}

				if disable_federation {
					return Ok(RoomMessageEventContent::text_plain(format!(
						"Finished bulk room ban, banned {room_ban_count} total rooms, evicted all users, and disabled \
						 incoming federation with the room."
					)));
				}
				return Ok(RoomMessageEventContent::text_plain(format!(
					"Finished bulk room ban, banned {room_ban_count} total rooms and evicted all users."
				)));
			}

			Ok(RoomMessageEventContent::text_plain(
				"Expected code block in command body. Add --help for details.",
			))
		},
		RoomModerationCommand::UnbanRoom {
			room,
			enable_federation,
		} => {
			let room_id = if room.is_room_id() {
				let room_id = match RoomId::parse(&room) {
					Ok(room_id) => room_id,
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Failed to parse room ID {room}. Please note that this requires a full room ID \
							 (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias (`#roomalias:example.com`): {e}"
						)))
					},
				};

				debug!("Room specified is a room ID, unbanning room ID");

				services().rooms.metadata.ban_room(&room_id, false)?;

				room_id
			} else if room.is_room_alias_id() {
				let room_alias = match RoomAliasId::parse(&room) {
					Ok(room_alias) => room_alias,
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Failed to parse room ID {room}. Please note that this requires a full room ID \
							 (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias (`#roomalias:example.com`): {e}"
						)))
					},
				};

				debug!(
					"Room specified is not a room ID, attempting to resolve room alias to a room ID locally, if not \
					 using get_alias_helper to fetch room ID remotely"
				);

				let room_id = if let Some(room_id) = services().rooms.alias.resolve_local_alias(&room_alias)? {
					room_id
				} else {
					debug!(
						"We don't have this room alias to a room ID locally, attempting to fetch room ID over \
						 federation"
					);

					match get_alias_helper(room_alias).await {
						Ok(response) => {
							debug!("Got federation response fetching room ID for room {room}: {:?}", response);
							response.room_id
						},
						Err(e) => {
							return Ok(RoomMessageEventContent::text_plain(format!(
								"Failed to resolve room alias {room} to a room ID: {e}"
							)));
						},
					}
				};

				services().rooms.metadata.ban_room(&room_id, false)?;

				room_id
			} else {
				return Ok(RoomMessageEventContent::text_plain(
					"Room specified is not a room ID or room alias. Please note that this requires a full room ID \
					 (`!awIh6gGInaS5wLQJwa:example.com`) or a room alias (`#roomalias:example.com`)",
				));
			};

			if enable_federation {
				services().rooms.metadata.disable_room(&room_id, false)?;
				return Ok(RoomMessageEventContent::text_plain("Room unbanned."));
			}

			Ok(RoomMessageEventContent::text_plain(
				"Room unbanned, you may need to re-enable federation with the room using enable-room if this is a \
				 remote room to make it fully functional.",
			))
		},
		RoomModerationCommand::ListBannedRooms => {
			let rooms = services()
				.rooms
				.metadata
				.list_banned_rooms()
				.collect::<Result<Vec<_>, _>>();

			match rooms {
				Ok(room_ids) => {
					// TODO: add room name from our state cache if available, default to the room ID
					// as the room name if we dont have it TODO: do same if we have a room alias for
					// this
					let plain_list = room_ids.iter().fold(String::new(), |mut output, room_id| {
						writeln!(output, "- `{}`", room_id).unwrap();
						output
					});

					let html_list = room_ids.iter().fold(String::new(), |mut output, room_id| {
						writeln!(output, "<li><code>{}</code></li>", escape_html(room_id.as_ref())).unwrap();
						output
					});

					let plain = format!("Rooms:\n{}", plain_list);
					let html = format!("Rooms:\n<ul>{}</ul>", html_list);
					Ok(RoomMessageEventContent::text_html(plain, html))
				},
				Err(e) => {
					error!("Failed to list banned rooms: {}", e);
					Ok(RoomMessageEventContent::text_plain(format!(
						"Unable to list room aliases: {}",
						e
					)))
				},
			}
		},
	}
}
