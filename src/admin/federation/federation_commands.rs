use std::fmt::Write as _;

use ruma::{events::room::message::RoomMessageEventContent, OwnedRoomId, RoomId, ServerName, UserId};

use crate::{escape_html, get_room_info, services, utils::HtmlEscape, Result};

pub(crate) async fn disable_room(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	services().rooms.metadata.disable_room(&room_id, true)?;
	Ok(RoomMessageEventContent::text_plain("Room disabled."))
}

pub(crate) async fn enable_room(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	services().rooms.metadata.disable_room(&room_id, false)?;
	Ok(RoomMessageEventContent::text_plain("Room enabled."))
}

pub(crate) async fn incoming_federeation(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let map = services().globals.roomid_federationhandletime.read().await;
	let mut msg = format!("Handling {} incoming pdus:\n", map.len());

	for (r, (e, i)) in map.iter() {
		let elapsed = i.elapsed();
		let _ = writeln!(msg, "{} {}: {}m{}s", r, e, elapsed.as_secs() / 60, elapsed.as_secs() % 60);
	}
	Ok(RoomMessageEventContent::text_plain(&msg))
}

pub(crate) async fn fetch_support_well_known(
	_body: Vec<&str>, server_name: Box<ServerName>,
) -> Result<RoomMessageEventContent> {
	let response = services()
		.globals
		.client
		.default
		.get(format!("https://{server_name}/.well-known/matrix/support"))
		.send()
		.await?;

	let text = response.text().await?;

	if text.is_empty() {
		return Ok(RoomMessageEventContent::text_plain("Response text/body is empty."));
	}

	if text.len() > 1500 {
		return Ok(RoomMessageEventContent::text_plain(
			"Response text/body is over 1500 characters, assuming no support well-known.",
		));
	}

	let json: serde_json::Value = match serde_json::from_str(&text) {
		Ok(json) => json,
		Err(_) => {
			return Ok(RoomMessageEventContent::text_plain("Response text/body is not valid JSON."));
		},
	};

	let pretty_json: String = match serde_json::to_string_pretty(&json) {
		Ok(json) => json,
		Err(_) => {
			return Ok(RoomMessageEventContent::text_plain("Response text/body is not valid JSON."));
		},
	};

	Ok(RoomMessageEventContent::text_html(
		format!("Got JSON response:\n\n```json\n{pretty_json}\n```"),
		format!(
			"<p>Got JSON response:</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
			HtmlEscape(&pretty_json)
		),
	))
}

pub(crate) async fn remote_user_in_rooms(_body: Vec<&str>, user_id: Box<UserId>) -> Result<RoomMessageEventContent> {
	if user_id.server_name() == services().globals.config.server_name {
		return Ok(RoomMessageEventContent::text_plain(
			"User belongs to our server, please use `list-joined-rooms` user admin command instead.",
		));
	}

	if !services().users.exists(&user_id)? {
		return Ok(RoomMessageEventContent::text_plain(
			"Remote user does not exist in our database.",
		));
	}

	let mut rooms: Vec<(OwnedRoomId, u64, String)> = services()
		.rooms
		.state_cache
		.rooms_joined(&user_id)
		.filter_map(Result::ok)
		.map(|room_id| get_room_info(&room_id))
		.collect();

	if rooms.is_empty() {
		return Ok(RoomMessageEventContent::text_plain("User is not in any rooms."));
	}

	rooms.sort_by_key(|r| r.1);
	rooms.reverse();

	let output_plain = format!(
		"Rooms {user_id} shares with us:\n{}",
		rooms
			.iter()
			.map(|(id, members, name)| format!("{id}\tMembers: {members}\tName: {name}"))
			.collect::<Vec<_>>()
			.join("\n")
	);
	let output_html = format!(
		"<table><caption>Rooms {user_id} shares with \
		 us</caption>\n<tr><th>id</th>\t<th>members</th>\t<th>name</th></tr>\n{}</table>",
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
}
