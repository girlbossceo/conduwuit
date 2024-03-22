use std::fmt::Write as _;

use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, OwnedRoomId, RoomId};

use crate::{
	service::admin::{escape_html, get_room_info, PAGE_SIZE},
	services, Result,
};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum RoomDirectoryCommand {
	/// - Publish a room to the room directory
	Publish {
		/// The room id of the room to publish
		room_id: Box<RoomId>,
	},

	/// - Unpublish a room to the room directory
	Unpublish {
		/// The room id of the room to unpublish
		room_id: Box<RoomId>,
	},

	/// - List rooms that are published
	List {
		page: Option<usize>,
	},
}

pub(crate) async fn process(command: RoomDirectoryCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		RoomDirectoryCommand::Publish {
			room_id,
		} => match services().rooms.directory.set_public(&room_id) {
			Ok(()) => Ok(RoomMessageEventContent::text_plain("Room published")),
			Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Unable to update room: {}", err))),
		},
		RoomDirectoryCommand::Unpublish {
			room_id,
		} => match services().rooms.directory.set_not_public(&room_id) {
			Ok(()) => Ok(RoomMessageEventContent::text_plain("Room unpublished")),
			Err(err) => Ok(RoomMessageEventContent::text_plain(format!("Unable to update room: {}", err))),
		},
		RoomDirectoryCommand::List {
			page,
		} => {
			// TODO: i know there's a way to do this with clap, but i can't seem to find it
			let page = page.unwrap_or(1);
			let mut rooms = services()
				.rooms
				.directory
				.public_rooms()
				.filter_map(Result::ok)
				.map(|id: OwnedRoomId| get_room_info(&id))
				.collect::<Vec<_>>();
			rooms.sort_by_key(|r| r.1);
			rooms.reverse();

			let rooms = rooms.into_iter().skip(page.saturating_sub(1) * PAGE_SIZE).take(PAGE_SIZE).collect::<Vec<_>>();

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
				"<table><caption>Room directory - page \
				 {page}</caption>\n<tr><th>id</th>\t<th>members</th>\t<th>name</th></tr>\n{}</table>",
				rooms.iter().fold(String::new(), |mut output, (id, members, name)| {
					writeln!(
						output,
						"<tr><td>{}</td>\t<td>{}</td>\t<td>{}</td></tr>",
						escape_html(id.as_ref()),
						members,
						escape_html(name.as_ref())
					)
					.unwrap();
					output
				})
			);
			Ok(RoomMessageEventContent::text_html(output_plain, output_html))
		},
	}
}
