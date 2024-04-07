use std::{collections::BTreeMap, fmt::Write as _};

use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, RoomId, ServerName};
use tokio::sync::RwLock;

use crate::{services, utils::HtmlEscape, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum FederationCommand {
	/// - List all rooms we are currently handling an incoming pdu from
	IncomingFederation,

	/// - Disables incoming federation handling for a room.
	DisableRoom {
		room_id: Box<RoomId>,
	},

	/// - Enables incoming federation handling for a room again.
	EnableRoom {
		room_id: Box<RoomId>,
	},

	/// - Verify json signatures
	///
	/// This command needs a JSON blob provided in a Markdown code block below
	/// the command.
	SignJson,

	/// - Verify json signatures
	///
	/// This command needs a JSON blob provided in a Markdown code block below
	/// the command.
	VerifyJson,

	/// - Fetch `/.well-known/matrix/support` from the specified server
	///
	/// Despite the name, this is not a federation endpoint and does not go
	/// through the federation / server resolution process as per-spec this is
	/// supposed to be served at the server_name.
	///
	/// Respecting homeservers put this file here for listing administration,
	/// moderation, and security inquiries. This command provides a way to
	/// easily fetch that information.
	FetchSupportWellKnown {
		server_name: Box<ServerName>,
	},
}

pub(crate) async fn process(command: FederationCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		FederationCommand::DisableRoom {
			room_id,
		} => {
			services().rooms.metadata.disable_room(&room_id, true)?;
			Ok(RoomMessageEventContent::text_plain("Room disabled."))
		},
		FederationCommand::EnableRoom {
			room_id,
		} => {
			services().rooms.metadata.disable_room(&room_id, false)?;
			Ok(RoomMessageEventContent::text_plain("Room enabled."))
		},
		FederationCommand::IncomingFederation => {
			let map = services().globals.roomid_federationhandletime.read().await;
			let mut msg = format!("Handling {} incoming pdus:\n", map.len());

			for (r, (e, i)) in map.iter() {
				let elapsed = i.elapsed();
				let _ = writeln!(msg, "{} {}: {}m{}s", r, e, elapsed.as_secs() / 60, elapsed.as_secs() % 60);
			}
			Ok(RoomMessageEventContent::text_plain(&msg))
		},
		FederationCommand::SignJson => {
			if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
				let string = body[1..body.len() - 1].join("\n");
				match serde_json::from_str(&string) {
					Ok(mut value) => {
						ruma::signatures::sign_json(
							services().globals.server_name().as_str(),
							services().globals.keypair(),
							&mut value,
						)
						.expect("our request json is what ruma expects");
						let json_text = serde_json::to_string_pretty(&value).expect("canonical json is valid json");
						Ok(RoomMessageEventContent::text_plain(json_text))
					},
					Err(e) => Ok(RoomMessageEventContent::text_plain(format!("Invalid json: {e}"))),
				}
			} else {
				Ok(RoomMessageEventContent::text_plain(
					"Expected code block in command body. Add --help for details.",
				))
			}
		},
		FederationCommand::VerifyJson => {
			if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
				let string = body[1..body.len() - 1].join("\n");
				match serde_json::from_str(&string) {
					Ok(value) => {
						let pub_key_map = RwLock::new(BTreeMap::new());

						services()
							.rooms
							.event_handler
							.fetch_required_signing_keys([&value], &pub_key_map)
							.await?;

						let pub_key_map = pub_key_map.read().await;
						match ruma::signatures::verify_json(&pub_key_map, &value) {
							Ok(()) => Ok(RoomMessageEventContent::text_plain("Signature correct")),
							Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
								"Signature verification failed: {e}"
							))),
						}
					},
					Err(e) => Ok(RoomMessageEventContent::text_plain(format!("Invalid json: {e}"))),
				}
			} else {
				Ok(RoomMessageEventContent::text_plain(
					"Expected code block in command body. Add --help for details.",
				))
			}
		},
		FederationCommand::FetchSupportWellKnown {
			server_name,
		} => {
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
		},
	}
}
