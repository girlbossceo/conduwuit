use crate::{Result, Ruma};
use ruma::{
    api::client::discovery::get_capabilities::{
        self, Capabilities, RoomVersionStability, RoomVersionsCapability,
    },
    RoomVersionId,
};
use std::collections::BTreeMap;

/// # `GET /_matrix/client/r0/capabilities`
///
/// Get information on the supported feature set and other relevent capabilities of this server.
pub async fn get_capabilities_route(
    _body: Ruma<get_capabilities::v3::IncomingRequest>,
) -> Result<get_capabilities::v3::Response> {
    let mut available = BTreeMap::new();
    available.insert(RoomVersionId::V5, RoomVersionStability::Stable);
    available.insert(RoomVersionId::V6, RoomVersionStability::Stable);

    let mut capabilities = Capabilities::new();
    capabilities.room_versions = RoomVersionsCapability {
        default: RoomVersionId::V6,
        available,
    };

    Ok(get_capabilities::v3::Response { capabilities })
}
