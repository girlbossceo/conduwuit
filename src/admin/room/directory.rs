use std::fmt::Write;

use clap::Subcommand;
use conduit::Result;
use futures::StreamExt;
use ruma::{events::room::message::RoomMessageEventContent, RoomId};

use crate::{escape_html, get_room_info, Command, PAGE_SIZE};

#[derive(Debug, Subcommand)]
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

pub(super) async fn process(command: RoomDirectoryCommand, context: &Command<'_>) -> Result<RoomMessageEventContent> {
	let services = context.services;
	match command {
		RoomDirectoryCommand::Publish {
			room_id,
		} => {
			services.rooms.directory.set_public(&room_id);
			Ok(RoomMessageEventContent::notice_plain("Room published"))
		},
		RoomDirectoryCommand::Unpublish {
			room_id,
		} => {
			services.rooms.directory.set_not_public(&room_id);
			Ok(RoomMessageEventContent::notice_plain("Room unpublished"))
		},
		RoomDirectoryCommand::List {
			page,
		} => {
			// TODO: i know there's a way to do this with clap, but i can't seem to find it
			let page = page.unwrap_or(1);
			let mut rooms: Vec<_> = services
				.rooms
				.directory
				.public_rooms()
				.then(|room_id| get_room_info(services, room_id))
				.collect()
				.await;

			rooms.sort_by_key(|r| r.1);
			rooms.reverse();

			let rooms: Vec<_> = rooms
				.into_iter()
				.skip(page.saturating_sub(1).saturating_mul(PAGE_SIZE))
				.take(PAGE_SIZE)
				.collect();

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
				rooms
					.iter()
					.fold(String::new(), |mut output, (id, members, name)| {
						writeln!(
							output,
							"<tr><td>{}</td>\t<td>{}</td>\t<td>{}</td></tr>",
							escape_html(id.as_ref()),
							members,
							escape_html(name.as_ref())
						)
						.expect("should be able to write to string buffer");
						output
					})
			);
			Ok(RoomMessageEventContent::text_html(output_plain, output_html))
		},
	}
}
