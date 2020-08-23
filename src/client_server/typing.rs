use super::State;
use crate::{utils, ConduitResult, Database, Ruma};
use ruma::api::client::r0::typing::create_typing_event;

#[cfg(feature = "conduit_bin")]
use rocket::put;

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/typing/<_>", data = "<body>")
)]
pub fn create_typing_event_route(
    db: State<'_, Database>,
    body: Ruma<create_typing_event::Request>,
) -> ConduitResult<create_typing_event::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if body.typing {
        db.rooms.edus.typing_add(
            &sender_id,
            &body.room_id,
            body.timeout.map(|d| d.as_millis() as u64).unwrap_or(30000)
                + utils::millis_since_unix_epoch(),
            &db.globals,
        )?;
    } else {
        db.rooms
            .edus
            .typing_remove(&sender_id, &body.room_id, &db.globals)?;
    }

    Ok(create_typing_event::Response.into())
}
