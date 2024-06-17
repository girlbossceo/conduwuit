use std::{
	collections::{BTreeMap, HashMap},
	fmt::Write,
	sync::{Arc, Mutex},
	time::Instant,
};

use api::client::validate_and_add_event_id;
use conduit::{
	debug, info, log,
	log::{capture, Capture},
	warn, Error, PduEvent, Result,
};
use ruma::{
	api::{client::error::ErrorKind, federation::event::get_room_state},
	events::room::message::RoomMessageEventContent,
	CanonicalJsonObject, EventId, OwnedRoomOrAliasId, RoomId, RoomVersionId, ServerName,
};
use service::services;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

pub(super) async fn echo(_body: Vec<&str>, message: Vec<String>) -> Result<RoomMessageEventContent> {
	let message = message.join(" ");

	Ok(RoomMessageEventContent::notice_plain(message))
}

pub(super) async fn get_auth_chain(_body: Vec<&str>, event_id: Box<EventId>) -> Result<RoomMessageEventContent> {
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

pub(super) async fn parse_pdu(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() < 2 || !body[0].trim().starts_with("```") || body.last().unwrap_or(&"").trim() != "```" {
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

	let string = body[1..body.len().saturating_sub(1)].join("\n");
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
}

pub(super) async fn get_pdu(_body: Vec<&str>, event_id: Box<EventId>) -> Result<RoomMessageEventContent> {
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
			Ok(RoomMessageEventContent::notice_markdown(format!(
				"{}\n```json\n{}\n```",
				if outlier {
					"Outlier PDU found in our database"
				} else {
					"PDU found in our database"
				},
				json_text
			)))
		},
		None => Ok(RoomMessageEventContent::text_plain("PDU not found locally.")),
	}
}

pub(super) async fn get_remote_pdu_list(
	body: Vec<&str>, server: Box<ServerName>, force: bool,
) -> Result<RoomMessageEventContent> {
	if !services().globals.config.allow_federation {
		return Ok(RoomMessageEventContent::text_plain(
			"Federation is disabled on this homeserver.",
		));
	}

	if server == services().globals.server_name() {
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to send federation requests to ourselves. Please use `get-pdu` for fetching local PDUs from \
			 the database.",
		));
	}

	if body.len() < 2 || !body[0].trim().starts_with("```") || body.last().unwrap_or(&"").trim() != "```" {
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

	let list = body
		.clone()
		.drain(1..body.len().checked_sub(1).unwrap())
		.filter_map(|pdu| EventId::parse(pdu).ok())
		.collect::<Vec<_>>();

	for pdu in list {
		if force {
			if let Err(e) = get_remote_pdu(Vec::new(), Box::from(pdu), server.clone()).await {
				services()
					.admin
					.send_message(RoomMessageEventContent::text_plain(format!(
						"Failed to get remote PDU, ignoring error: {e}"
					)))
					.await;
				warn!(%e, "Failed to get remote PDU, ignoring error");
			}
		} else {
			get_remote_pdu(Vec::new(), Box::from(pdu), server.clone()).await?;
		}
	}

	Ok(RoomMessageEventContent::text_plain("Fetched list of remote PDUs."))
}

pub(super) async fn get_remote_pdu(
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
				let parsed_result = services()
					.rooms
					.event_handler
					.parse_incoming_pdu(&response.pdu);
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

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"{}\n```json\n{}\n```",
				"Got PDU from specified server and handled as backfilled PDU successfully. Event body:", json_text
			)))
		},
		Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
			"Remote server did not have PDU or failed sending request to remote server: {e}"
		))),
	}
}

pub(super) async fn get_room_state(_body: Vec<&str>, room: OwnedRoomOrAliasId) -> Result<RoomMessageEventContent> {
	let room_id = services().rooms.alias.resolve(&room).await?;
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

	let json = serde_json::to_string_pretty(&room_state).map_err(|e| {
		warn!("Failed converting room state vector in our database to pretty JSON: {e}");
		Error::bad_database(
			"Failed to convert room state events to pretty JSON, possible invalid room state events in our database",
		)
	})?;

	Ok(RoomMessageEventContent::notice_markdown(format!("```json\n{json}\n```")))
}

pub(super) async fn ping(_body: Vec<&str>, server: Box<ServerName>) -> Result<RoomMessageEventContent> {
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
				return Ok(RoomMessageEventContent::notice_markdown(format!(
					"Got response which took {ping_time:?} time:\n```json\n{json}\n```"
				)));
			}

			Ok(RoomMessageEventContent::text_plain(format!(
				"Got non-JSON response which took {ping_time:?} time:\n{response:?}"
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

pub(super) async fn force_device_list_updates(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	// Force E2EE device list updates for all users
	for user_id in services().users.iter().filter_map(Result::ok) {
		services().users.mark_device_key_update(&user_id)?;
	}
	Ok(RoomMessageEventContent::text_plain(
		"Marked all devices for all users as having new keys to update",
	))
}

pub(super) async fn change_log_level(
	_body: Vec<&str>, filter: Option<String>, reset: bool,
) -> Result<RoomMessageEventContent> {
	let handles = &["console"];

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
			.log
			.reload
			.reload(&old_filter_layer, Some(handles))
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
			.log
			.reload
			.reload(&new_filter_layer, Some(handles))
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

pub(super) async fn sign_json(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() < 2 || !body[0].trim().starts_with("```") || body.last().unwrap_or(&"").trim() != "```" {
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

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
}

pub(super) async fn verify_json(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() < 2 || !body[0].trim().starts_with("```") || body.last().unwrap_or(&"").trim() != "```" {
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

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
}

#[tracing::instrument(skip(_body))]
pub(super) async fn first_pdu_in_room(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	if !services()
		.rooms
		.state_cache
		.server_in_room(&services().globals.config.server_name, &room_id)?
	{
		return Ok(RoomMessageEventContent::text_plain(
			"We are not participating in the room / we don't know about the room ID.",
		));
	}

	let first_pdu = services()
		.rooms
		.timeline
		.first_pdu_in_room(&room_id)?
		.ok_or_else(|| Error::bad_database("Failed to find the first PDU in database"))?;

	Ok(RoomMessageEventContent::text_plain(format!("{first_pdu:?}")))
}

#[tracing::instrument(skip(_body))]
pub(super) async fn latest_pdu_in_room(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	if !services()
		.rooms
		.state_cache
		.server_in_room(&services().globals.config.server_name, &room_id)?
	{
		return Ok(RoomMessageEventContent::text_plain(
			"We are not participating in the room / we don't know about the room ID.",
		));
	}

	let latest_pdu = services()
		.rooms
		.timeline
		.latest_pdu_in_room(&room_id)?
		.ok_or_else(|| Error::bad_database("Failed to find the latest PDU in database"))?;

	Ok(RoomMessageEventContent::text_plain(format!("{latest_pdu:?}")))
}

#[tracing::instrument(skip(_body))]
pub(super) async fn force_set_room_state_from_server(
	_body: Vec<&str>, server_name: Box<ServerName>, room_id: Box<RoomId>,
) -> Result<RoomMessageEventContent> {
	if !services()
		.rooms
		.state_cache
		.server_in_room(&services().globals.config.server_name, &room_id)?
	{
		return Ok(RoomMessageEventContent::text_plain(
			"We are not participating in the room / we don't know about the room ID.",
		));
	}

	let first_pdu = services()
		.rooms
		.timeline
		.latest_pdu_in_room(&room_id)?
		.ok_or_else(|| Error::bad_database("Failed to find the latest PDU in database"))?;

	let room_version = services().rooms.state.get_room_version(&room_id)?;

	let mut state: HashMap<u64, Arc<EventId>> = HashMap::new();
	let pub_key_map = RwLock::new(BTreeMap::new());

	let remote_state_response = services()
		.sending
		.send_federation_request(
			&server_name,
			get_room_state::v1::Request {
				room_id: room_id.clone().into(),
				event_id: first_pdu.event_id.clone().into(),
			},
		)
		.await?;

	let mut events = Vec::with_capacity(remote_state_response.pdus.len());

	for pdu in remote_state_response.pdus.clone() {
		events.push(match services().rooms.event_handler.parse_incoming_pdu(&pdu) {
			Ok(t) => t,
			Err(e) => {
				warn!("Could not parse PDU, ignoring: {e}");
				continue;
			},
		});
	}

	info!("Fetching required signing keys for all the state events we got");
	services()
		.rooms
		.event_handler
		.fetch_required_signing_keys(events.iter().map(|(_event_id, event, _room_id)| event), &pub_key_map)
		.await?;

	info!("Going through room_state response PDUs");
	for result in remote_state_response
		.pdus
		.iter()
		.map(|pdu| validate_and_add_event_id(services(), pdu, &room_version, &pub_key_map))
	{
		let Ok((event_id, value)) = result.await else {
			continue;
		};

		let pdu = PduEvent::from_id_val(&event_id, value.clone()).map_err(|e| {
			warn!("Invalid PDU in fetching remote room state PDUs response: {} {:?}", e, value);
			Error::BadServerResponse("Invalid PDU in send_join response.")
		})?;

		services()
			.rooms
			.outlier
			.add_pdu_outlier(&event_id, &value)?;
		if let Some(state_key) = &pdu.state_key {
			let shortstatekey = services()
				.rooms
				.short
				.get_or_create_shortstatekey(&pdu.kind.to_string().into(), state_key)?;
			state.insert(shortstatekey, pdu.event_id.clone());
		}
	}

	info!("Going through auth_chain response");
	for result in remote_state_response
		.auth_chain
		.iter()
		.map(|pdu| validate_and_add_event_id(services(), pdu, &room_version, &pub_key_map))
	{
		let Ok((event_id, value)) = result.await else {
			continue;
		};

		services()
			.rooms
			.outlier
			.add_pdu_outlier(&event_id, &value)?;
	}

	let new_room_state = services()
		.rooms
		.event_handler
		.resolve_state(room_id.clone().as_ref(), &room_version, state)
		.await?;

	info!("Forcing new room state");
	let (short_state_hash, new, removed) = services()
		.rooms
		.state_compressor
		.save_state(room_id.clone().as_ref(), new_room_state)?;

	let state_lock = services().rooms.state.mutex.lock(&room_id).await;
	services()
		.rooms
		.state
		.force_state(room_id.clone().as_ref(), short_state_hash, new, removed, &state_lock)
		.await?;

	info!(
		"Updating joined counts for room just in case (e.g. we may have found a difference in the room's \
		 m.room.member state"
	);
	services().rooms.state_cache.update_joined_count(&room_id)?;

	drop(state_lock);

	Ok(RoomMessageEventContent::text_plain(
		"Successfully forced the room state from the requested remote server.",
	))
}

pub(super) async fn get_signing_keys(
	_body: Vec<&str>, server_name: Option<Box<ServerName>>, _cached: bool,
) -> Result<RoomMessageEventContent> {
	let server_name = server_name.unwrap_or_else(|| services().server.config.server_name.clone().into());
	let signing_keys = services().globals.signing_keys_for(&server_name)?;

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"```rs\n{signing_keys:#?}\n```"
	)))
}

#[allow(dead_code)]
pub(super) async fn get_verify_keys(
	_body: Vec<&str>, server_name: Option<Box<ServerName>>, cached: bool,
) -> Result<RoomMessageEventContent> {
	let server_name = server_name.unwrap_or_else(|| services().server.config.server_name.clone().into());
	let mut out = String::new();

	if cached {
		writeln!(out, "| Key ID | VerifyKey |")?;
		writeln!(out, "| --- | --- |")?;
		for (key_id, verify_key) in services().globals.verify_keys_for(&server_name)? {
			writeln!(out, "| {key_id} | {verify_key:?} |")?;
		}

		return Ok(RoomMessageEventContent::notice_markdown(out));
	}

	let signature_ids: Vec<String> = Vec::new();
	let keys = services()
		.rooms
		.event_handler
		.fetch_signing_keys_for_server(&server_name, signature_ids)
		.await?;

	writeln!(out, "| Key ID | Public Key |")?;
	writeln!(out, "| --- | --- |")?;
	for (key_id, key) in keys {
		writeln!(out, "| {key_id} | {key} |")?;
	}

	Ok(RoomMessageEventContent::notice_markdown(out))
}

pub(super) async fn resolve_true_destination(
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

	let filter: &capture::Filter = &|data| {
		data.level() <= log::Level::DEBUG
			&& data.mod_name().starts_with("conduit")
			&& matches!(data.span_name(), "actual" | "well-known" | "srv")
	};

	let state = &services().server.log.capture;
	let logs = Arc::new(Mutex::new(String::new()));
	let capture = Capture::new(state, Some(filter), capture::fmt_markdown(logs.clone()));

	let capture_scope = capture.start();
	let actual = services()
		.resolver
		.resolve_actual_dest(&server_name, !no_cache)
		.await?;
	drop(capture_scope);

	let msg = format!(
		"{}\nDestination: {}\nHostname URI: {}",
		logs.lock().expect("locked"),
		actual.dest,
		actual.host,
	);
	Ok(RoomMessageEventContent::text_markdown(msg))
}

#[must_use]
pub(super) fn memory_stats() -> RoomMessageEventContent {
	let html_body = conduit::alloc::memory_stats();

	if html_body.is_none() {
		return RoomMessageEventContent::text_plain("malloc stats are not supported on your compiled malloc.");
	}

	RoomMessageEventContent::text_html(
		"This command's output can only be viewed by clients that render HTML.".to_owned(),
		html_body.expect("string result"),
	)
}

#[cfg(tokio_unstable)]
pub(super) async fn runtime_metrics(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let out = services().server.metrics.runtime_metrics().map_or_else(
		|| "Runtime metrics are not available.".to_owned(),
		|metrics| format!("```rs\n{metrics:#?}\n```"),
	);

	Ok(RoomMessageEventContent::text_markdown(out))
}

#[cfg(not(tokio_unstable))]
pub(super) async fn runtime_metrics(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(RoomMessageEventContent::text_markdown(
		"Runtime metrics require building with `tokio_unstable`.",
	))
}

#[cfg(tokio_unstable)]
pub(super) async fn runtime_interval(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let out = services().server.metrics.runtime_interval().map_or_else(
		|| "Runtime metrics are not available.".to_owned(),
		|metrics| format!("```rs\n{metrics:#?}\n```"),
	);

	Ok(RoomMessageEventContent::text_markdown(out))
}

#[cfg(not(tokio_unstable))]
pub(super) async fn runtime_interval(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(RoomMessageEventContent::text_markdown(
		"Runtime metrics require building with `tokio_unstable`.",
	))
}
