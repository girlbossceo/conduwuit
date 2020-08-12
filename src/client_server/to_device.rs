use super::State;
use crate::{ConduitResult, Database, Error, Ruma};
use ruma::api::client::{
    error::ErrorKind,
    r0::to_device::{self, send_event_to_device},
};

#[cfg(feature = "conduit_bin")]
use rocket::put;

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/sendToDevice/<_>/<_>", data = "<body>")
)]
pub fn send_event_to_device_route(
    db: State<'_, Database>,
    body: Ruma<send_event_to_device::IncomingRequest>,
) -> ConduitResult<send_event_to_device::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            match target_device_id_maybe {
                to_device::DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    db.users.add_to_device_event(
                        sender_id,
                        &target_user_id,
                        &target_device_id,
                        &body.event_type,
                        serde_json::from_str(event.get()).map_err(|_| {
                            Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid")
                        })?,
                        &db.globals,
                    )?
                }

                to_device::DeviceIdOrAllDevices::AllDevices => {
                    for target_device_id in db.users.all_device_ids(&target_user_id) {
                        db.users.add_to_device_event(
                            sender_id,
                            &target_user_id,
                            &target_device_id?,
                            &body.event_type,
                            serde_json::from_str(event.get()).map_err(|_| {
                                Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid")
                            })?,
                            &db.globals,
                        )?;
                    }
                }
            }
        }
    }

    Ok(send_event_to_device::Response.into())
}
