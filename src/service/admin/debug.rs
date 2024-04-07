use std::{collections::BTreeMap, sync::Arc, time::Instant};

use clap::Subcommand;
use ruma::{
	api::client::error::ErrorKind, events::room::message::RoomMessageEventContent, CanonicalJsonObject, EventId,
	RoomId, RoomVersionId, ServerName,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::{api::server_server::parse_incoming_pdu, services, utils::HtmlEscape, Error, PduEvent, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum DebugCommand {
	/// - Get the auth_chain of a PDU
	GetAuthChain {
		/// An event ID (the $ character followed by the base64 reference hash)
		event_id: Box<EventId>,
	},

	/// - Parse and print a PDU from a JSON
	///
	/// The PDU event is only checked for validity and is not added to the
	/// database.
	///
	/// This command needs a JSON blob provided in a Markdown code block below
	/// the command.
	ParsePdu,

	/// - Retrieve and print a PDU by ID from the conduwuit database
	GetPdu {
		/// An event ID (a $ followed by the base64 reference hash)
		event_id: Box<EventId>,
	},

	/// - Attempts to retrieve a PDU from a remote server. Inserts it into our
	/// database/timeline if found and we do not have this PDU already
	/// (following normal event auth rules, handles it as an incoming PDU).
	GetRemotePdu {
		/// An event ID (a $ followed by the base64 reference hash)
		event_id: Box<EventId>,

		/// Argument for us to attempt to fetch the event from the
		/// specified remote server.
		server: Box<ServerName>,
	},

	/// - Gets all the room state events for the specified room.
	///
	/// This is functionally equivalent to `GET
	/// /_matrix/client/v3/rooms/{roomid}/state`, except the admin command does
	/// *not* check if the sender user is allowed to see state events. This is
	/// done because it's implied that server admins here have database access
	/// and can see/get room info themselves anyways if they were malicious
	/// admins.
	///
	/// Of course the check is still done on the actual client API.
	GetRoomState {
		/// Room ID
		room_id: Box<RoomId>,
	},

	/// - Sends a federation request to the remote server's
	///   `/_matrix/federation/v1/version` endpoint and measures the latency it
	///   took for the server to respond
	Ping {
		server: Box<ServerName>,
	},

	/// - Forces device lists for all local and remote users to be updated (as
	///   having new keys available)
	ForceDeviceListUpdates,

	/// - Change tracing log level/filter on the fly
	///
	/// This accepts the same format as the `log` config option.
	ChangeLogLevel {
		/// Log level/filter
		filter: Option<String>,

		/// Resets the log level/filter to the one in your config
		#[arg(short, long)]
		reset: bool,
	},
}

pub(crate) async fn process(command: DebugCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		DebugCommand::GetAuthChain {
			event_id,
		} => {
			let event_id = Arc::<EventId>::from(event_id);
			if let Some(event) = services().rooms.timeline.get_pdu_json(&event_id)? {
				let room_id_str = event
					.get("room_id")
					.and_then(|val| val.as_str())
					.ok_or_else(|| Error::bad_database("Invalid event in database"))?;

				let room_id = <&RoomId>::try_from(room_id_str)
					.map_err(|_| Error::bad_database("Invalid room id field in event in database"))?;
				let start = Instant::now();
				let count = services()
					.rooms
					.auth_chain
					.get_auth_chain(room_id, vec![event_id])
					.await?
					.count();
				let elapsed = start.elapsed();
				RoomMessageEventContent::text_plain(format!("Loaded auth chain with length {count} in {elapsed:?}"))
			} else {
				RoomMessageEventContent::text_plain("Event not found.")
			}
		},
		DebugCommand::ParsePdu => {
			if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
				let string = body[1..body.len() - 1].join("\n");
				match serde_json::from_str(&string) {
					Ok(value) => match ruma::signatures::reference_hash(&value, &RoomVersionId::V6) {
						Ok(hash) => {
							let event_id = EventId::parse(format!("${hash}"));

							match serde_json::from_value::<PduEvent>(
								serde_json::to_value(value).expect("value is json"),
							) {
								Ok(pdu) => {
									RoomMessageEventContent::text_plain(format!("EventId: {event_id:?}\n{pdu:#?}"))
								},
								Err(e) => RoomMessageEventContent::text_plain(format!(
									"EventId: {event_id:?}\nCould not parse event: {e}"
								)),
							}
						},
						Err(e) => RoomMessageEventContent::text_plain(format!("Could not parse PDU JSON: {e:?}")),
					},
					Err(e) => RoomMessageEventContent::text_plain(format!("Invalid json in command body: {e}")),
				}
			} else {
				RoomMessageEventContent::text_plain("Expected code block in command body.")
			}
		},
		DebugCommand::GetPdu {
			event_id,
		} => {
			let mut outlier = false;
			let mut pdu_json = services()
				.rooms
				.timeline
				.get_non_outlier_pdu_json(&event_id)?;
			if pdu_json.is_none() {
				outlier = true;
				pdu_json = services().rooms.timeline.get_pdu_json(&event_id)?;
			}
			match pdu_json {
				Some(json) => {
					let json_text = serde_json::to_string_pretty(&json).expect("canonical json is valid json");
					return Ok(RoomMessageEventContent::text_html(
						format!(
							"{}\n```json\n{}\n```",
							if outlier {
								"Outlier PDU found in our database"
							} else {
								"PDU found in our database"
							},
							json_text
						),
						format!(
							"<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
							if outlier {
								"Outlier PDU found in our database"
							} else {
								"PDU found in our database"
							},
							HtmlEscape(&json_text)
						),
					));
				},
				None => {
					return Ok(RoomMessageEventContent::text_plain("PDU not found locally."));
				},
			}
		},
		DebugCommand::GetRemotePdu {
			event_id,
			server,
		} => {
			if !services().globals.config.allow_federation {
				return Ok(RoomMessageEventContent::text_plain(
					"Federation is disabled on this homeserver.",
				));
			}

			if server == services().globals.server_name() {
				return Ok(RoomMessageEventContent::text_plain(
					"Not allowed to send federation requests to ourselves. Please use `get-pdu` for fetching local \
					 PDUs.",
				));
			}

			// TODO: use Futures as some requests may take a while so we dont block the
			// admin room
			match services()
				.sending
				.send_federation_request(
					&server,
					ruma::api::federation::event::get_event::v1::Request {
						event_id: event_id.clone().into(),
					},
				)
				.await
			{
				Ok(response) => {
					let json: CanonicalJsonObject = serde_json::from_str(response.pdu.get()).map_err(|e| {
						warn!(
							"Requested event ID {event_id} from server but failed to convert from RawValue to \
							 CanonicalJsonObject (malformed event/response?): {e}"
						);
						Error::BadRequest(ErrorKind::Unknown, "Received response from server but failed to parse PDU")
					})?;

					debug!("Attempting to parse PDU: {:?}", &response.pdu);
					let parsed_pdu = {
						let parsed_result = parse_incoming_pdu(&response.pdu);
						let (event_id, value, room_id) = match parsed_result {
							Ok(t) => t,
							Err(e) => {
								warn!("Failed to parse PDU: {e}");
								info!("Full PDU: {:?}", &response.pdu);
								return Ok(RoomMessageEventContent::text_plain(format!(
									"Failed to parse PDU remote server {server} sent us: {e}"
								)));
							},
						};

						vec![(event_id, value, room_id)]
					};

					let pub_key_map = RwLock::new(BTreeMap::new());

					debug!("Attempting to fetch homeserver signing keys for {server}");
					services()
						.rooms
						.event_handler
						.fetch_required_signing_keys(
							parsed_pdu.iter().map(|(_event_id, event, _room_id)| event),
							&pub_key_map,
						)
						.await
						.unwrap_or_else(|e| {
							warn!("Could not fetch all signatures for PDUs from {server}: {e:?}");
						});

					info!("Attempting to handle event ID {event_id} as backfilled PDU");
					services()
						.rooms
						.timeline
						.backfill_pdu(&server, response.pdu, &pub_key_map)
						.await?;

					let json_text = serde_json::to_string_pretty(&json).expect("canonical json is valid json");

					return Ok(RoomMessageEventContent::text_html(
						format!(
							"{}\n```json\n{}\n```",
							"Got PDU from specified server and handled as backfilled PDU successfully. Event body:",
							json_text
						),
						format!(
							"<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
							"Got PDU from specified server and handled as backfilled PDU successfully. Event body:",
							HtmlEscape(&json_text)
						),
					));
				},
				Err(e) => {
					return Ok(RoomMessageEventContent::text_plain(format!(
						"Remote server did not have PDU or failed sending request to remote server: {e}"
					)));
				},
			}
		},
		DebugCommand::GetRoomState {
			room_id,
		} => {
			let room_state = services()
				.rooms
				.state_accessor
				.room_state_full(&room_id)
				.await?
				.values()
				.map(|pdu| pdu.to_state_event())
				.collect::<Vec<_>>();

			if room_state.is_empty() {
				return Ok(RoomMessageEventContent::text_plain(
					"Unable to find room state in our database (vector is empty)",
				));
			}

			let json_text = serde_json::to_string_pretty(&room_state).map_err(|e| {
				error!("Failed converting room state vector in our database to pretty JSON: {e}");
				Error::bad_database(
					"Failed to convert room state events to pretty JSON, possible invalid room state events in our \
					 database",
				)
			})?;

			return Ok(RoomMessageEventContent::text_html(
				format!("{}\n```json\n{}\n```", "Found full room state", json_text),
				format!(
					"<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
					"Found full room state",
					HtmlEscape(&json_text)
				),
			));
		},
		DebugCommand::Ping {
			server,
		} => {
			if server == services().globals.server_name() {
				return Ok(RoomMessageEventContent::text_plain(
					"Not allowed to send federation requests to ourselves.",
				));
			}

			let timer = tokio::time::Instant::now();

			match services()
				.sending
				.send_federation_request(&server, ruma::api::federation::discovery::get_server_version::v1::Request {})
				.await
			{
				Ok(response) => {
					let ping_time = timer.elapsed();

					let json_text_res = serde_json::to_string_pretty(&response.server);

					if let Ok(json) = json_text_res {
						return Ok(RoomMessageEventContent::text_html(
							format!("Got response which took {ping_time:?} time:\n```json\n{json}\n```"),
							format!(
								"<p>Got response which took {ping_time:?} time:</p>\n<pre><code \
								 class=\"language-json\">{}\n</code></pre>\n",
								HtmlEscape(&json)
							),
						));
					}

					return Ok(RoomMessageEventContent::text_plain(format!(
						"Got non-JSON response which took {ping_time:?} time:\n{0:?}",
						response
					)));
				},
				Err(e) => {
					error!("Failed sending federation request to specified server from ping debug command: {e}");
					return Ok(RoomMessageEventContent::text_plain(format!(
						"Failed sending federation request to specified server:\n\n{e}",
					)));
				},
			}
		},
		DebugCommand::ForceDeviceListUpdates => {
			// Force E2EE device list updates for all users
			for user_id in services().users.iter().filter_map(Result::ok) {
				services().users.mark_device_key_update(&user_id)?;
			}
			RoomMessageEventContent::text_plain("Marked all devices for all users as having new keys to update")
		},
		DebugCommand::ChangeLogLevel {
			filter,
			reset,
		} => {
			if reset {
				let old_filter_layer = match EnvFilter::try_new(&services().globals.config.log) {
					Ok(s) => s,
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Log level from config appears to be invalid now: {e}"
						)));
					},
				};

				match services()
					.globals
					.tracing_reload_handle
					.modify(|filter| *filter = old_filter_layer)
				{
					Ok(()) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Successfully changed log level back to config value {}",
							services().globals.config.log
						)));
					},
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Failed to modify and reload the global tracing log level: {e}"
						)));
					},
				}
			}

			if let Some(filter) = filter {
				let new_filter_layer = match EnvFilter::try_new(filter) {
					Ok(s) => s,
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Invalid log level filter specified: {e}"
						)));
					},
				};

				match services()
					.globals
					.tracing_reload_handle
					.modify(|filter| *filter = new_filter_layer)
				{
					Ok(()) => {
						return Ok(RoomMessageEventContent::text_plain("Successfully changed log level"));
					},
					Err(e) => {
						return Ok(RoomMessageEventContent::text_plain(format!(
							"Failed to modify and reload the global tracing log level: {e}"
						)));
					},
				}
			}

			return Ok(RoomMessageEventContent::text_plain("No log level was specified."));
		},
	})
}
