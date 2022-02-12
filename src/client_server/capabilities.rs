use crate::{Result, Ruma};
use ruma::{
    api::client::r0::capabilities::{
        get_capabilities, Capabilities, RoomVersionStability, RoomVersionsCapability,
    },
    RoomVersionId,
};
use std::collections::BTreeMap;

/// # `GET /_matrix/client/r0/capabilities`
///
/// Get information on the supported feature set and other relevent capabilities of this server.
pub async fn get_capabilities_route(
    _body: Ruma<get_capabilities::Request>,
) -> Result<get_capabilities::Response> {
    let mut available = BTreeMap::new();
    available.insert(RoomVersionId::V5, RoomVersionStability::Stable);
    available.insert(RoomVersionId::V6, RoomVersionStability::Stable);

    let mut capabilities = Capabilities::new();
    capabilities.room_versions = RoomVersionsCapability {
        default: RoomVersionId::V6,
        available,
    };

    Ok(get_capabilities::Response { capabilities })
}
