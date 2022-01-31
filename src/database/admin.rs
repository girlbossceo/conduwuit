use std::{convert::TryInto, sync::Arc};

use crate::{pdu::PduBuilder, Database};
use rocket::futures::{channel::mpsc, stream::StreamExt};
use ruma::{
    events::{room::message::RoomMessageEventContent, EventType},
    UserId,
};
use serde_json::value::to_raw_value;
use tokio::sync::{MutexGuard, RwLock, RwLockReadGuard};
use tracing::warn;

pub enum AdminCommand {
    RegisterAppservice(serde_yaml::Value),
    UnregisterAppservice(String),
    ListAppservices,
    ListLocalUsers,
    ShowMemoryUsage,
    SendMessage(RoomMessageEventContent),
}

#[derive(Clone)]
pub struct Admin {
    pub sender: mpsc::UnboundedSender<AdminCommand>,
}

impl Admin {
    pub fn start_handler(
        &self,
        db: Arc<RwLock<Database>>,
        mut receiver: mpsc::UnboundedReceiver<AdminCommand>,
    ) {
        tokio::spawn(async move {
            // TODO: Use futures when we have long admin commands
            //let mut futures = FuturesUnordered::new();

            let guard = db.read().await;

            let conduit_user = UserId::parse(format!("@conduit:{}", guard.globals.server_name()))
                .expect("@conduit:server_name is valid");

            let conduit_room = guard
                .rooms
                .id_from_alias(
                    format!("#admins:{}", guard.globals.server_name())
                        .as_str()
                        .try_into()
                        .expect("#admins:server_name is a valid room alias"),
                )
                .unwrap();

            let conduit_room = match conduit_room {
                None => {
                    warn!("Conduit instance does not have an #admins room. Logging to that room will not work. Restart Conduit after creating a user to fix this.");
                    return;
                }
                Some(r) => r,
            };

            drop(guard);

            let send_message = |message: RoomMessageEventContent,
                                guard: RwLockReadGuard<'_, Database>,
                                mutex_lock: &MutexGuard<'_, ()>| {
                guard
                    .rooms
                    .build_and_append_pdu(
                        PduBuilder {
                            event_type: EventType::RoomMessage,
                            content: to_raw_value(&message)
                                .expect("event is valid, we just created it"),
                            unsigned: None,
                            state_key: None,
                            redacts: None,
                        },
                        &conduit_user,
                        &conduit_room,
                        &guard,
                        mutex_lock,
                    )
                    .unwrap();
            };

            loop {
                tokio::select! {
                    Some(event) = receiver.next() => {
                        let guard = db.read().await;
                        let mutex_state = Arc::clone(
                            guard.globals
                                .roomid_mutex_state
                                .write()
                                .unwrap()
                                .entry(conduit_room.clone())
                                .or_default(),
                        );
                        let state_lock = mutex_state.lock().await;

                        match event {
                            AdminCommand::ListLocalUsers => {
                                match guard.users.list_local_users() {
                                    Ok(users) => {
                                        let mut msg: String = format!("Found {} local user account(s):\n", users.len());
                                        msg += &users.join("\n");
                                        send_message(RoomMessageEventContent::text_plain(&msg), guard, &state_lock);
                                    }
                                    Err(e) => {
                                        send_message(RoomMessageEventContent::text_plain(e.to_string()), guard, &state_lock);
                                    }
                                }
                            }
                            AdminCommand::RegisterAppservice(yaml) => {
                                match guard.appservice.register_appservice(yaml) {
                                    Ok(Some(id)) => {
                                        let msg: String = format!("OK. Appservice {} created", id);
                                        send_message(RoomMessageEventContent::text_plain(msg), guard, &state_lock);
                                    }
                                    Ok(None) => {
                                        send_message(RoomMessageEventContent::text_plain("WARN. Appservice created, but its ID was not returned!"), guard, &state_lock);
                                    }
                                    Err(_) => {
                                        send_message(RoomMessageEventContent::text_plain("ERR: Failed register appservice. Check server log"), guard, &state_lock);
                                    }
                                }
                            }
                            AdminCommand::UnregisterAppservice(service_name) => {
                                if let Ok(_) = guard.appservice.unregister_appservice(&service_name) {
                                    if let Ok(_) = guard.sending.cleanup_events(&service_name) {
                                        let msg: String = format!("OK. Appservice {} removed", service_name);
                                        send_message(RoomMessageEventContent::text_plain(msg), guard, &state_lock);
                                    } else {
                                        let msg: String = format!("WARN: Appservice {} removed, but failed to cleanup events", service_name);
                                        send_message(RoomMessageEventContent::text_plain(msg), guard, &state_lock);
                                    }
                                } else {
                                    let msg: String = format!("ERR. Appservice {} not removed", service_name);
                                    send_message(RoomMessageEventContent::text_plain(msg), guard, &state_lock);
                                }
                            }
                            AdminCommand::ListAppservices => {
                                if let Ok(appservices) = guard.appservice.iter_ids().map(|ids| ids.collect::<Vec<_>>()) {
                                    let count = appservices.len();
                                    let output = format!(
                                        "Appservices ({}): {}",
                                        count,
                                        appservices.into_iter().filter_map(|r| r.ok()).collect::<Vec<_>>().join(", ")
                                    );
                                    send_message(RoomMessageEventContent::text_plain(output), guard, &state_lock);
                                } else {
                                    send_message(RoomMessageEventContent::text_plain("Failed to get appservices."), guard, &state_lock);
                                }
                            }
                            AdminCommand::ShowMemoryUsage => {
                                if let Ok(response) = guard._db.memory_usage() {
                                    send_message(RoomMessageEventContent::text_plain(response), guard, &state_lock);
                                } else {
                                    send_message(RoomMessageEventContent::text_plain("Failed to get database memory usage.".to_owned()), guard, &state_lock);
                                }
                            }
                            AdminCommand::SendMessage(message) => {
                                send_message(message, guard, &state_lock);
                            }
                        }

                        drop(state_lock);
                    }
                }
            }
        });
    }

    pub fn send(&self, command: AdminCommand) {
        self.sender.unbounded_send(command).unwrap();
    }
}
