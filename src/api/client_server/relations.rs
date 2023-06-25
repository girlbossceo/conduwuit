use crate::{services, Result, Ruma};
use std::time::{Duration, SystemTime};

/// # `GET /_matrix/client/r0/todo`
pub async fn get_relating_events_route(
    body: Ruma<get_turn_server_info::v3::Request>,
) -> Result<get_turn_server_info::v3::Response> {
    let sender_user = body.sender_user.as_ref().expect("user is authenticated");
    todo!();
}
