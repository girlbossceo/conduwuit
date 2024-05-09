use std::{collections::BTreeMap, sync::Arc, time::Instant};

use conduit::{utils::HtmlEscape, Error, Result};
use ruma::{
	api::client::error::ErrorKind, events::room::message::RoomMessageEventContent, CanonicalJsonObject, EventId,
	RoomId, RoomVersionId, ServerName,
};
use service::{rooms::event_handler::parse_incoming_pdu, sending::send::resolve_actual_dest, services, PduEvent};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

pub(crate) async fn get_auth_chain(_body: Vec<&str>, event_id: Box<EventId>) -> Result<RoomMessageEventContent> {
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
			.event_ids_iter(room_id, vec![event_id])
			.await?
			.count();
		let elapsed = start.elapsed();
		Ok(RoomMessageEventContent::text_plain(format!(
			"Loaded auth chain with length {count} in {elapsed:?}"
		)))
	} else {
		Ok(RoomMessageEventContent::text_plain("Event not found."))
	}
}

pub(crate) async fn parse_pdu(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let string = body[1..body.len() - 1].join("\n");
		match serde_json::from_str(&string) {
			Ok(value) => match ruma::signatures::reference_hash(&value, &RoomVersionId::V6) {
				Ok(hash) => {
					let event_id = EventId::parse(format!("${hash}"));

					match serde_json::from_value::<PduEvent>(serde_json::to_value(value).expect("value is json")) {
						Ok(pdu) => Ok(RoomMessageEventContent::text_plain(format!("EventId: {event_id:?}\n{pdu:#?}"))),
						Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
							"EventId: {event_id:?}\nCould not parse event: {e}"
						))),
					}
				},
				Err(e) => Ok(RoomMessageEventContent::text_plain(format!("Could not parse PDU JSON: {e:?}"))),
			},
			Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
				"Invalid json in command body: {e}"
			))),
		}
	} else {
		Ok(RoomMessageEventContent::text_plain("Expected code block in command body."))
	}
}

pub(crate) async fn get_pdu(_body: Vec<&str>, event_id: Box<EventId>) -> Result<RoomMessageEventContent> {
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
			Ok(RoomMessageEventContent::text_html(
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
			))
		},
		None => Ok(RoomMessageEventContent::text_plain("PDU not found locally.")),
	}
}

pub(crate) async fn get_remote_pdu_list(
	body: Vec<&str>, server: Box<ServerName>, force: bool,
) -> Result<RoomMessageEventContent> {
	if !services().globals.config.allow_federation {
		return Ok(RoomMessageEventContent::text_plain(
			"Federation is disabled on this homeserver.",
		));
	}

	if server == services().globals.server_name() {
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to send federation requests to ourselves. Please use `get-pdu` for fetching local PDUs.",
		));
	}

	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let list = body
			.clone()
			.drain(1..body.len().checked_sub(1).unwrap())
			.filter_map(|pdu| EventId::parse(pdu).ok())
			.collect::<Vec<_>>();

		for pdu in list {
			if force {
				_ = get_remote_pdu(Vec::new(), Box::from(pdu), server.clone()).await;
			} else {
				get_remote_pdu(Vec::new(), Box::from(pdu), server.clone()).await?;
			}
		}

		return Ok(RoomMessageEventContent::text_plain("Fetched list of remote PDUs."));
	}

	Ok(RoomMessageEventContent::text_plain(
		"Expected code block in command body. Add --help for details.",
	))
}

pub(crate) async fn get_remote_pdu(
	_body: Vec<&str>, event_id: Box<EventId>, server: Box<ServerName>,
) -> Result<RoomMessageEventContent> {
	if !services().globals.config.allow_federation {
		return Ok(RoomMessageEventContent::text_plain(
			"Federation is disabled on this homeserver.",
		));
	}

	if server == services().globals.server_name() {
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to send federation requests to ourselves. Please use `get-pdu` for fetching local PDUs.",
		));
	}

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
				.fetch_required_signing_keys(parsed_pdu.iter().map(|(_event_id, event, _room_id)| event), &pub_key_map)
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

			Ok(RoomMessageEventContent::text_html(
				format!(
					"{}\n```json\n{}\n```",
					"Got PDU from specified server and handled as backfilled PDU successfully. Event body:", json_text
				),
				format!(
					"<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
					"Got PDU from specified server and handled as backfilled PDU successfully. Event body:",
					HtmlEscape(&json_text)
				),
			))
		},
		Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
			"Remote server did not have PDU or failed sending request to remote server: {e}"
		))),
	}
}

pub(crate) async fn get_room_state(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
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
		warn!("Failed converting room state vector in our database to pretty JSON: {e}");
		Error::bad_database(
			"Failed to convert room state events to pretty JSON, possible invalid room state events in our database",
		)
	})?;

	Ok(RoomMessageEventContent::text_html(
		format!("{}\n```json\n{}\n```", "Found full room state", json_text),
		format!(
			"<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
			"Found full room state",
			HtmlEscape(&json_text)
		),
	))
}

pub(crate) async fn ping(_body: Vec<&str>, server: Box<ServerName>) -> Result<RoomMessageEventContent> {
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

			Ok(RoomMessageEventContent::text_plain(format!(
				"Got non-JSON response which took {ping_time:?} time:\n{0:?}",
				response
			)))
		},
		Err(e) => {
			warn!("Failed sending federation request to specified server from ping debug command: {e}");
			Ok(RoomMessageEventContent::text_plain(format!(
				"Failed sending federation request to specified server:\n\n{e}",
			)))
		},
	}
}

pub(crate) async fn force_device_list_updates(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	// Force E2EE device list updates for all users
	for user_id in services().users.iter().filter_map(Result::ok) {
		services().users.mark_device_key_update(&user_id)?;
	}
	Ok(RoomMessageEventContent::text_plain(
		"Marked all devices for all users as having new keys to update",
	))
}

pub(crate) async fn change_log_level(
	_body: Vec<&str>, filter: Option<String>, reset: bool,
) -> Result<RoomMessageEventContent> {
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
			.server
			.tracing_reload_handle
			.reload(&old_filter_layer)
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
			.server
			.tracing_reload_handle
			.reload(&new_filter_layer)
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

	Ok(RoomMessageEventContent::text_plain("No log level was specified."))
}

pub(crate) async fn sign_json(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let string = body[1..body.len().checked_sub(1).unwrap()].join("\n");
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
}

pub(crate) async fn verify_json(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let string = body[1..body.len().checked_sub(1).unwrap()].join("\n");
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
}

pub(crate) async fn resolve_true_destination(
	_body: Vec<&str>, server_name: Box<ServerName>, no_cache: bool,
) -> Result<RoomMessageEventContent> {
	if !services().globals.config.allow_federation {
		return Ok(RoomMessageEventContent::text_plain(
			"Federation is disabled on this homeserver.",
		));
	}

	if server_name == services().globals.config.server_name {
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to send federation requests to ourselves. Please use `get-pdu` for fetching local PDUs.",
		));
	}

	let (actual_dest, hostname_uri) = resolve_actual_dest(&server_name, no_cache).await?;

	Ok(RoomMessageEventContent::text_plain(format!(
		"Actual destination: {actual_dest:?} | Hostname URI: {hostname_uri}"
	)))
}

#[must_use]
pub(crate) fn memory_stats() -> RoomMessageEventContent {
	let html_body = conduit::alloc::memory_stats();

	if html_body.is_empty() {
		return RoomMessageEventContent::text_plain("malloc stats are not supported on your compiled malloc.");
	}

	RoomMessageEventContent::text_html(
		"This command's output can only be viewed by clients that render HTML.".to_owned(),
		html_body,
	)
}
