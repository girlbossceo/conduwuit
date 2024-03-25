use std::fmt::Write as _;

use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, OwnedRoomId};

use crate::{
	service::admin::{
		escape_html, get_room_info, room_alias, room_alias::RoomAliasCommand, room_directory,
		room_directory::RoomDirectoryCommand, room_moderation, room_moderation::RoomModerationCommand, PAGE_SIZE,
	},
	services, Result,
};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum RoomCommand {
	/// - List all rooms the server knows about
	List {
		page: Option<usize>,
	},

	#[command(subcommand)]
	/// - Manage moderation of remote or local rooms
	Moderation(RoomModerationCommand),

	#[command(subcommand)]
	/// - Manage rooms' aliases
	Alias(RoomAliasCommand),

	#[command(subcommand)]
	/// - Manage the room directory
	Directory(RoomDirectoryCommand),
}

pub(crate) async fn process(command: RoomCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		RoomCommand::Alias(command) => room_alias::process(command, body).await,

		RoomCommand::Directory(command) => room_directory::process(command, body).await,

		RoomCommand::Moderation(command) => room_moderation::process(command, body).await,

		RoomCommand::List {
			page,
		} => {
			// TODO: i know there's a way to do this with clap, but i can't seem to find it
			let page = page.unwrap_or(1);
			let mut rooms = services()
				.rooms
				.metadata
				.iter_ids()
				.filter_map(Result::ok)
				.map(|id: OwnedRoomId| get_room_info(&id))
				.collect::<Vec<_>>();
			rooms.sort_by_key(|r| r.1);
			rooms.reverse();

			let rooms = rooms
				.into_iter()
				.skip(page.saturating_sub(1) * PAGE_SIZE)
				.take(PAGE_SIZE)
				.collect::<Vec<_>>();

			if rooms.is_empty() {
				return Ok(RoomMessageEventContent::text_plain("No more rooms."));
			};

			let output_plain = format!(
				"Rooms:\n{}",
				rooms
					.iter()
					.map(|(id, members, name)| format!("{id}\tMembers: {members}\tName: {name}"))
					.collect::<Vec<_>>()
					.join("\n")
			);
			let output_html = format!(
				"<table><caption>Room list - page \
				 {page}</caption>\n<tr><th>id</th>\t<th>members</th>\t<th>name</th></tr>\n{}</table>",
				rooms
					.iter()
					.fold(String::new(), |mut output, (id, members, name)| {
						writeln!(
							output,
							"<tr><td>{}</td>\t<td>{}</td>\t<td>{}</td></tr>",
							escape_html(id.as_ref()),
							members,
							escape_html(name)
						)
						.unwrap();
						output
					})
			);
			Ok(RoomMessageEventContent::text_html(output_plain, output_html))
		},
	}
}
