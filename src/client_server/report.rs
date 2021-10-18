use std::sync::Arc;

use crate::{database::admin::AdminCommand, database::DatabaseGuard, ConduitResult, Error, Ruma};
use ruma::{
    api::client::{error::ErrorKind, r0::room::report_content},
    events::room::message,
    Int,
};

#[cfg(feature = "conduit_bin")]
use rocket::{http::RawStr, post};

/// # `POST /_matrix/client/r0/rooms/{roomId}/report/{eventId}`
///
/// Reports an inappropriate event to homeserver admins
///
#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/report/<_>", data = "<body>")
)]
#[tracing::instrument(skip(db, body))]
pub async fn report_event_route(
    db: DatabaseGuard,
    body: Ruma<report_content::Request<'_>>,
) -> ConduitResult<report_content::Response> {
    let sender_user = body.sender_user.as_ref().expect("user is authenticated");

    let pdu = match db.rooms.get_pdu(&body.event_id) {
        Ok(pdu) if !pdu.is_none() => pdu,
        _ => {
            return Err(Error::BadRequest(
                ErrorKind::InvalidParam,
                "Invalid Event ID",
            ))
        }
    }
    .unwrap();

    if body.score >= Int::from(0) && body.score <= Int::from(-100) {
        return Err(Error::BadRequest(
            ErrorKind::InvalidParam,
            "Invalid score, must be within 0 to -100",
        ));
    };

    if body.reason.chars().count() > 1000 {
        return Err(Error::BadRequest(
            ErrorKind::InvalidParam,
            "Reason too long, should be 1000 characters or fewer",
        ));
    };

    let mutex_state = Arc::clone(
        db.globals
            .roomid_mutex_state
            .write()
            .unwrap()
            .entry(body.room_id.clone())
            .or_default(),
    );
    let state_lock = mutex_state.lock().await;

    db.admin.send(AdminCommand::SendMessage(
        message::RoomMessageEventContent::text_html(
            format!(
                concat!(
                    "Report received from: {}\r\n\r\n",
                    "Event ID: {}\r\n",
                    "Room ID: {}\r\n",
                    "Sent By: {}\r\n\r\n",
                    "Report Score: {}\r\n",
                    "Report Reason: {}"
                ),
                sender_user, pdu.event_id, pdu.room_id, pdu.sender, body.score, body.reason
            )
            .to_owned(),
            format!(
                concat!(
                    "<details><summary>Report received from: {}</summary><details>",
                    "<summary>Event Info</summary><p>Event ID: {}<br>Room ID: {}<br>Sent By: {}",
                    "</p></details><details><summary>Report Info</summary><p>Report Score: {}",
                    "</br>Report Reason: {}</p></details></details>"
                ),
                sender_user,
                pdu.event_id,
                pdu.room_id,
                pdu.sender,
                body.score,
                RawStr::new(&body.reason).html_escape()
            )
            .to_owned(),
        ),
    ));

    drop(state_lock);

    db.flush()?;

    Ok(report_content::Response {}.into())
}
