use std::fmt::Write;

use ruma::{events::room::message::RoomMessageEventContent, RoomAliasId};

use super::RoomAliasCommand;
use crate::{escape_html, services, Result};

pub(crate) async fn process(command: RoomAliasCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		RoomAliasCommand::Set {
			ref room_alias_localpart,
			..
		}
		| RoomAliasCommand::Remove {
			ref room_alias_localpart,
		}
		| RoomAliasCommand::Which {
			ref room_alias_localpart,
		} => {
			let room_alias_str = format!("#{}:{}", room_alias_localpart, services().globals.server_name());
			let room_alias = match RoomAliasId::parse_box(room_alias_str) {
				Ok(alias) => alias,
				Err(err) => return Ok(RoomMessageEventContent::text_plain(format!("Failed to parse alias: {}", err))),
			};
			match command {
				RoomAliasCommand::Set {
					force,
					room_id,
					..
				} => match (force, services().rooms.alias.resolve_local_alias(&room_alias)) {
					(true, Ok(Some(id))) => match services().rooms.alias.set_alias(&room_alias, &room_id) {
						Ok(()) => Ok(RoomMessageEventContent::text_plain(format!(
							"Successfully overwrote alias (formerly {})",
							id
						))),
						Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Failed to remove alias: {}", err))),
					},
					(false, Ok(Some(id))) => Ok(RoomMessageEventContent::text_plain(format!(
						"Refusing to overwrite in use alias for {}, use -f or --force to overwrite",
						id
					))),
					(_, Ok(None)) => match services().rooms.alias.set_alias(&room_alias, &room_id) {
						Ok(()) => Ok(RoomMessageEventContent::text_plain("Successfully set alias")),
						Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Failed to remove alias: {err}"))),
					},
					(_, Err(err)) => Ok(RoomMessageEventContent::text_plain(format!("Unable to lookup alias: {err}"))),
				},
				RoomAliasCommand::Remove {
					..
				} => match services().rooms.alias.resolve_local_alias(&room_alias) {
					Ok(Some(id)) => match services().rooms.alias.remove_alias(&room_alias) {
						Ok(()) => Ok(RoomMessageEventContent::text_plain(format!("Removed alias from {}", id))),
						Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Failed to remove alias: {}", err))),
					},
					Ok(None) => Ok(RoomMessageEventContent::text_plain("Alias isn't in use.")),
					Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Unable to lookup alias: {}", err))),
				},
				RoomAliasCommand::Which {
					..
				} => match services().rooms.alias.resolve_local_alias(&room_alias) {
					Ok(Some(id)) => Ok(RoomMessageEventContent::text_plain(format!("Alias resolves to {}", id))),
					Ok(None) => Ok(RoomMessageEventContent::text_plain("Alias isn't in use.")),
					Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Unable to lookup alias: {}", err))),
				},
				RoomAliasCommand::List {
					..
				} => unreachable!(),
			}
		},
		RoomAliasCommand::List {
			room_id,
		} => {
			if let Some(room_id) = room_id {
				let aliases = services()
					.rooms
					.alias
					.local_aliases_for_room(&room_id)
					.collect::<Result<Vec<_>, _>>();
				match aliases {
					Ok(aliases) => {
						let plain_list = aliases.iter().fold(String::new(), |mut output, alias| {
							writeln!(output, "- {alias}").expect("should be able to write to string buffer");
							output
						});

						let html_list = aliases.iter().fold(String::new(), |mut output, alias| {
							writeln!(output, "<li>{}</li>", escape_html(alias.as_ref()))
								.expect("should be able to write to string buffer");
							output
						});

						let plain = format!("Aliases for {room_id}:\n{plain_list}");
						let html = format!("Aliases for {room_id}:\n<ul>{html_list}</ul>");
						Ok(RoomMessageEventContent::text_html(plain, html))
					},
					Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Unable to list aliases: {}", err))),
				}
			} else {
				let aliases = services()
					.rooms
					.alias
					.all_local_aliases()
					.collect::<Result<Vec<_>, _>>();
				match aliases {
					Ok(aliases) => {
						let server_name = services().globals.server_name();
						let plain_list = aliases
							.iter()
							.fold(String::new(), |mut output, (alias, id)| {
								writeln!(output, "- `{alias}` -> #{id}:{server_name}")
									.expect("should be able to write to string buffer");
								output
							});

						let html_list = aliases
							.iter()
							.fold(String::new(), |mut output, (alias, id)| {
								writeln!(
									output,
									"<li><code>{}</code> -> #{}:{}</li>",
									escape_html(alias.as_ref()),
									escape_html(id.as_ref()),
									server_name
								)
								.expect("should be able to write to string buffer");
								output
							});

						let plain = format!("Aliases:\n{plain_list}");
						let html = format!("Aliases:\n<ul>{html_list}</ul>");
						Ok(RoomMessageEventContent::text_html(plain, html))
					},
					Err(e) => Ok(RoomMessageEventContent::text_plain(format!("Unable to list room aliases: {e}"))),
				}
			}
		},
	}
}
