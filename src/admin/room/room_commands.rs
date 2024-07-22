use std::fmt::Write;

use ruma::events::room::message::RoomMessageEventContent;

use crate::{escape_html, get_room_info, services, Result, PAGE_SIZE};

pub(super) async fn list(
	_body: Vec<&str>, page: Option<usize>, exclude_disabled: bool, exclude_banned: bool,
) -> Result<RoomMessageEventContent> {
	// TODO: i know there's a way to do this with clap, but i can't seem to find it
	let page = page.unwrap_or(1);
	let mut rooms = services()
		.rooms
		.metadata
		.iter_ids()
		.filter_map(|room_id| {
			room_id
				.ok()
				.filter(|room_id| {
					if exclude_disabled
						&& services()
							.rooms
							.metadata
							.is_disabled(room_id)
							.unwrap_or(false)
					{
						return false;
					}

					if exclude_banned
						&& services()
							.rooms
							.metadata
							.is_banned(room_id)
							.unwrap_or(false)
					{
						return false;
					}

					true
				})
				.map(|room_id| get_room_info(services(), &room_id))
		})
		.collect::<Vec<_>>();
	rooms.sort_by_key(|r| r.1);
	rooms.reverse();

	let rooms = rooms
		.into_iter()
		.skip(page.saturating_sub(1).saturating_mul(PAGE_SIZE))
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
				.expect("should be able to write to string buffer");
				output
			})
	);
	Ok(RoomMessageEventContent::text_html(output_plain, output_html))
}
