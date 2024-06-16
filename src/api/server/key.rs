use std::{
	collections::BTreeMap,
	time::{Duration, SystemTime},
};

use axum::{response::IntoResponse, Json};
use ruma::{
	api::{
		federation::discovery::{get_server_keys, ServerSigningKeys, VerifyKey},
		OutgoingResponse,
	},
	serde::{Base64, Raw},
	MilliSecondsSinceUnixEpoch, OwnedServerSigningKeyId,
};

use crate::{services, Result};

/// # `GET /_matrix/key/v2/server`
///
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by
///   this will be valid forever.
// Response type for this endpoint is Json because we need to calculate a
// signature for the response
pub(crate) async fn get_server_keys_route() -> Result<impl IntoResponse> {
	let verify_keys: BTreeMap<OwnedServerSigningKeyId, VerifyKey> = BTreeMap::from([(
		format!("ed25519:{}", services().globals.keypair().version())
			.try_into()
			.expect("found invalid server signing keys in DB"),
		VerifyKey {
			key: Base64::new(services().globals.keypair().public_key().to_vec()),
		},
	)]);

	let mut response = serde_json::from_slice(
		get_server_keys::v2::Response {
			server_key: Raw::new(&ServerSigningKeys {
				server_name: services().globals.server_name().to_owned(),
				verify_keys,
				old_verify_keys: BTreeMap::new(),
				signatures: BTreeMap::new(),
				valid_until_ts: MilliSecondsSinceUnixEpoch::from_system_time(
					SystemTime::now()
						.checked_add(Duration::from_secs(86400 * 7))
						.expect("valid_until_ts should not get this high"),
				)
				.expect("time is valid"),
			})
			.expect("static conversion, no errors"),
		}
		.try_into_http_response::<Vec<u8>>()
		.unwrap()
		.body(),
	)
	.unwrap();

	ruma::signatures::sign_json(
		services().globals.server_name().as_str(),
		services().globals.keypair(),
		&mut response,
	)
	.unwrap();

	Ok(Json(response))
}

/// # `GET /_matrix/key/v2/server/{keyId}`
///
/// Gets the public signing keys of this server.
///
/// - Matrix does not support invalidating public keys, so the key returned by
///   this will be valid forever.
pub(crate) async fn get_server_keys_deprecated_route() -> impl IntoResponse { get_server_keys_route().await }
