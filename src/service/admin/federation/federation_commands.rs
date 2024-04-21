use std::fmt::Write as _;

use ruma::{events::room::message::RoomMessageEventContent, RoomId, ServerName};

use crate::{services, utils::HtmlEscape, Result};

pub(super) async fn disable_room(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	services().rooms.metadata.disable_room(&room_id, true)?;
	Ok(RoomMessageEventContent::text_plain("Room disabled."))
}

pub(super) async fn enable_room(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	services().rooms.metadata.disable_room(&room_id, false)?;
	Ok(RoomMessageEventContent::text_plain("Room enabled."))
}

pub(super) async fn incoming_federeation(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let map = services().globals.roomid_federationhandletime.read().await;
	let mut msg = format!("Handling {} incoming pdus:\n", map.len());

	for (r, (e, i)) in map.iter() {
		let elapsed = i.elapsed();
		let _ = writeln!(msg, "{} {}: {}m{}s", r, e, elapsed.as_secs() / 60, elapsed.as_secs() % 60);
	}
	Ok(RoomMessageEventContent::text_plain(&msg))
}

pub(super) async fn fetch_support_well_known(
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
