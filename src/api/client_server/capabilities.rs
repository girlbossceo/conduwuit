use std::collections::BTreeMap;

use ruma::api::client::discovery::get_capabilities::{
	self, Capabilities, RoomVersionStability, RoomVersionsCapability, ThirdPartyIdChangesCapability,
};

use crate::{services, Result, Ruma};

/// # `GET /_matrix/client/v3/capabilities`
///
/// Get information on the supported feature set and other relevent capabilities
/// of this server.
pub(crate) async fn get_capabilities_route(
	_body: Ruma<get_capabilities::v3::Request>,
) -> Result<get_capabilities::v3::Response> {
	let mut available = BTreeMap::new();
	for room_version in &services().globals.unstable_room_versions {
		available.insert(room_version.clone(), RoomVersionStability::Unstable);
	}
	for room_version in &services().globals.stable_room_versions {
		available.insert(room_version.clone(), RoomVersionStability::Stable);
	}

	let mut capabilities = Capabilities::default();
	capabilities.room_versions = RoomVersionsCapability {
		default: services().globals.default_room_version(),
		available,
	};

	// conduit does not implement 3PID stuff
	capabilities.thirdparty_id_changes = ThirdPartyIdChangesCapability {
		enabled: false,
	};

	Ok(get_capabilities::v3::Response {
		capabilities,
	})
}
