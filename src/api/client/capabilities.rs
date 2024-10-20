use std::collections::BTreeMap;

use axum::extract::State;
use ruma::{
	api::client::discovery::get_capabilities::{
		self, Capabilities, GetLoginTokenCapability, RoomVersionStability, RoomVersionsCapability,
		ThirdPartyIdChangesCapability,
	},
	RoomVersionId,
};
use serde_json::json;

use crate::{Result, Ruma};

/// # `GET /_matrix/client/v3/capabilities`
///
/// Get information on the supported feature set and other relevent capabilities
/// of this server.
pub(crate) async fn get_capabilities_route(
	State(services): State<crate::State>, _body: Ruma<get_capabilities::v3::Request>,
) -> Result<get_capabilities::v3::Response> {
	let available: BTreeMap<RoomVersionId, RoomVersionStability> = services
		.globals
		.unstable_room_versions
		.iter()
		.map(|unstable_room_version| (unstable_room_version.clone(), RoomVersionStability::Unstable))
		.chain(
			services
				.globals
				.stable_room_versions
				.iter()
				.map(|stable_room_version| (stable_room_version.clone(), RoomVersionStability::Stable)),
		)
		.collect();

	let mut capabilities = Capabilities::default();
	capabilities.room_versions = RoomVersionsCapability {
		default: services.globals.default_room_version(),
		available,
	};

	// we do not implement 3PID stuff
	capabilities.thirdparty_id_changes = ThirdPartyIdChangesCapability {
		enabled: false,
	};

	// we dont support generating tokens yet
	capabilities.get_login_token = GetLoginTokenCapability {
		enabled: false,
	};

	// MSC4133 capability
	capabilities
		.set("uk.tcpip.msc4133.profile_fields", json!({"enabled": true}))
		.expect("this is valid JSON we created");

	Ok(get_capabilities::v3::Response {
		capabilities,
	})
}
