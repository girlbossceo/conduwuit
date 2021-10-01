use crate::{database::DatabaseGuard, ConduitResult};
use ruma::api::client::r0::voip::get_turn_server_info;
use std::time::Duration;

#[cfg(feature = "conduit_bin")]
use rocket::get;

/// # `GET /_matrix/client/r0/voip/turnServer`
///
/// TODO: Returns information about the recommended turn server.
#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/voip/turnServer"))]
#[tracing::instrument(skip(db))]
pub async fn turn_server_route(db: DatabaseGuard) -> ConduitResult<get_turn_server_info::Response> {
    Ok(get_turn_server_info::Response {
        username: db.globals.turn_username().clone(),
        password: db.globals.turn_password().clone(),
        uris: db.globals.turn_uris().to_vec(),
        ttl: Duration::from_secs(db.globals.turn_ttl()),
    }
    .into())
}
