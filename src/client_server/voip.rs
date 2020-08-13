use crate::{ConduitResult, Error};
use ruma::api::client::{error::ErrorKind, r0::message::send_message_event};

#[cfg(feature = "conduit_bin")]
use rocket::get;

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/voip/turnServer"))]
pub fn turn_server_route() -> ConduitResult<send_message_event::Response> {
    Err(Error::BadRequest(
        ErrorKind::NotFound,
        "There is no turn server yet.",
    ))
}
