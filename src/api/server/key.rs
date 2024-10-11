use std::{collections::BTreeMap, time::Duration};

use axum::{extract::State, response::IntoResponse, Json};
use conduit::{utils::timepoint_from_now, Result};
use ruma::{
	api::{
		federation::discovery::{get_server_keys, ServerSigningKeys},
		OutgoingResponse,
	},
	serde::Raw,
	MilliSecondsSinceUnixEpoch,
};

/// # `GET /_matrix/key/v2/server`
///
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by
///   this will be valid forever.
// Response type for this endpoint is Json because we need to calculate a
// signature for the response
pub(crate) async fn get_server_keys_route(State(services): State<crate::State>) -> Result<impl IntoResponse> {
	let server_name = services.globals.server_name();
	let verify_keys = services.server_keys.verify_keys_for(server_name).await;
	let server_key = ServerSigningKeys {
		verify_keys,
		server_name: server_name.to_owned(),
		valid_until_ts: valid_until_ts(),
		old_verify_keys: BTreeMap::new(),
		signatures: BTreeMap::new(),
	};

	let response = get_server_keys::v2::Response {
		server_key: Raw::new(&server_key)?,
	}
	.try_into_http_response::<Vec<u8>>()?;

	let mut response = serde_json::from_slice(response.body())?;
	services.server_keys.sign_json(&mut response)?;

	Ok(Json(response))
}

fn valid_until_ts() -> MilliSecondsSinceUnixEpoch {
	let dur = Duration::from_secs(86400 * 7);
	let timepoint = timepoint_from_now(dur).expect("SystemTime should not overflow");
	MilliSecondsSinceUnixEpoch::from_system_time(timepoint).expect("UInt should not overflow")
}

/// # `GET /_matrix/key/v2/server/{keyId}`
///
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by
///   this will be valid forever.
pub(crate) async fn get_server_keys_deprecated_route(State(services): State<crate::State>) -> impl IntoResponse {
	get_server_keys_route(State(services)).await
}
