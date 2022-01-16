use std::{convert::TryFrom, convert::TryInto, sync::Arc, time::Instant};

use crate::{
    error::{Error, Result},
    pdu::PduBuilder,
    server_server, Database, PduEvent,
};
use rocket::{
    futures::{channel::mpsc, stream::StreamExt},
    http::RawStr,
};
use ruma::{
    events::{room::message::RoomMessageEventContent, EventType},
    EventId, RoomId, RoomVersionId, UserId,
};
use serde_json::value::to_raw_value;
use tokio::sync::{MutexGuard, RwLock, RwLockReadGuard};
use tracing::warn;

pub enum AdminCommand {
    RegisterAppservice(serde_yaml::Value),
    UnregisterAppservice(String),
    ListAppservices,
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
                            AdminCommand::RegisterAppservice(yaml) => {
                                guard.appservice.register_appservice(yaml).unwrap(); // TODO handle error
                            }
                            AdminCommand::UnregisterAppservice(service_name) => {
                                guard.appservice.unregister_appservice(&service_name).unwrap(); // TODO: see above
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
                                    send_message(RoomMessageEventContent::text_plain("Failed to get database memory usage.".to_string()), guard, &state_lock);
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

pub fn parse_admin_command(db: &Database, command_line: &str, body: Vec<&str>) -> AdminCommand {
    let mut parts = command_line.split_whitespace().skip(1);

    let command_name = match parts.next() {
        Some(command) => command,
        None => {
            let message = "No command given. Use <code>help</code> for a list of commands.";
            return AdminCommand::SendMessage(RoomMessageEventContent::text_html(
                html_to_markdown(message),
                message,
            ));
        }
    };

    let args: Vec<_> = parts.collect();

    match try_parse_admin_command(db, command_name, args, body) {
        Ok(admin_command) => admin_command,
        Err(error) => {
            let message = format!(
                "Encountered error while handling <code>{}</code> command:\n\
                <pre>{}</pre>",
                command_name, error,
            );

            AdminCommand::SendMessage(RoomMessageEventContent::text_html(
                html_to_markdown(&message),
                message,
            ))
        }
    }
}

// Helper for `RoomMessageEventContent::text_html`, which needs the content as
// both markdown and HTML.
fn html_to_markdown(text: &str) -> String {
    text.replace("<p>", "")
        .replace("</p>", "\n")
        .replace("<pre>", "```\n")
        .replace("</pre>", "\n```")
        .replace("<code>", "`")
        .replace("</code>", "`")
        .replace("<li>", "* ")
        .replace("</li>", "")
        .replace("<ul>\n", "")
        .replace("</ul>\n", "")
}

const HELP_TEXT: &'static str = r#"
<p>The following commands are available:</p>
<ul>
<li><code>register_appservice</code>: Register a bridge using its registration YAML</li>
<li><code>unregister_appservice</code>: Unregister a bridge using its ID</li>
<li><code>list_appservices</code>: List all the currently registered bridges</li>
<li><code>get_auth_chain</code>: Get the `auth_chain` of a PDU</li>
<li><code>parse_pdu</code>: Parse and print a PDU from a JSON</li>
<li><code>get_pdu</code>: Retrieve and print a PDU by ID from the Conduit database</li>
<li><code>database_memory_usage</code>: Print database memory usage statistics</li>
<ul>
"#;

pub fn try_parse_admin_command(
    db: &Database,
    command: &str,
    args: Vec<&str>,
    body: Vec<&str>,
) -> Result<AdminCommand> {
    let command = match command {
        "help" => AdminCommand::SendMessage(RoomMessageEventContent::text_html(
            html_to_markdown(HELP_TEXT),
            HELP_TEXT,
        )),
        "register_appservice" => {
            if body.len() > 2 && body[0].trim() == "```" && body.last().unwrap().trim() == "```" {
                let appservice_config = body[1..body.len() - 1].join("\n");
                let parsed_config = serde_yaml::from_str::<serde_yaml::Value>(&appservice_config);
                match parsed_config {
                    Ok(yaml) => AdminCommand::RegisterAppservice(yaml),
                    Err(e) => AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                        format!("Could not parse appservice config: {}", e),
                    )),
                }
            } else {
                AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                    "Expected code block in command body.",
                ))
            }
        }
        "unregister_appservice" => {
            if args.len() == 1 {
                AdminCommand::UnregisterAppservice(args[0].to_owned())
            } else {
                AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                    "Missing appservice identifier",
                ))
            }
        }
        "list_appservices" => AdminCommand::ListAppservices,
        "get_auth_chain" => {
            if args.len() == 1 {
                if let Ok(event_id) = EventId::parse_arc(args[0]) {
                    if let Some(event) = db.rooms.get_pdu_json(&event_id)? {
                        let room_id_str = event
                            .get("room_id")
                            .and_then(|val| val.as_str())
                            .ok_or_else(|| Error::bad_database("Invalid event in database"))?;

                        let room_id = <&RoomId>::try_from(room_id_str).map_err(|_| {
                            Error::bad_database("Invalid room id field in event in database")
                        })?;
                        let start = Instant::now();
                        let count =
                            server_server::get_auth_chain(room_id, vec![event_id], db)?.count();
                        let elapsed = start.elapsed();
                        return Ok(AdminCommand::SendMessage(
                            RoomMessageEventContent::text_plain(format!(
                                "Loaded auth chain with length {} in {:?}",
                                count, elapsed
                            )),
                        ));
                    }
                }
            }

            AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                "Usage: get_auth_chain <event-id>",
            ))
        }
        "parse_pdu" => {
            if body.len() > 2 && body[0].trim() == "```" && body.last().unwrap().trim() == "```" {
                let string = body[1..body.len() - 1].join("\n");
                match serde_json::from_str(&string) {
                    Ok(value) => {
                        let event_id = EventId::parse(format!(
                            "${}",
                            // Anything higher than version3 behaves the same
                            ruma::signatures::reference_hash(&value, &RoomVersionId::V6)
                                .expect("ruma can calculate reference hashes")
                        ))
                        .expect("ruma's reference hashes are valid event ids");

                        match serde_json::from_value::<PduEvent>(
                            serde_json::to_value(value).expect("value is json"),
                        ) {
                            Ok(pdu) => {
                                AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                                    format!("EventId: {:?}\n{:#?}", event_id, pdu),
                                ))
                            }
                            Err(e) => AdminCommand::SendMessage(
                                RoomMessageEventContent::text_plain(format!(
                                    "EventId: {:?}\nCould not parse event: {}",
                                    event_id, e
                                )),
                            ),
                        }
                    }
                    Err(e) => AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                        format!("Invalid json in command body: {}", e),
                    )),
                }
            } else {
                AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                    "Expected code block in command body.",
                ))
            }
        }
        "get_pdu" => {
            if args.len() == 1 {
                if let Ok(event_id) = EventId::parse(args[0]) {
                    let mut outlier = false;
                    let mut pdu_json = db.rooms.get_non_outlier_pdu_json(&event_id)?;
                    if pdu_json.is_none() {
                        outlier = true;
                        pdu_json = db.rooms.get_pdu_json(&event_id)?;
                    }
                    match pdu_json {
                        Some(json) => {
                            let json_text = serde_json::to_string_pretty(&json)
                                .expect("canonical json is valid json");
                            AdminCommand::SendMessage(
                                RoomMessageEventContent::text_html(
                                    format!("{}\n```json\n{}\n```",
                                    if outlier {
                                        "PDU is outlier"
                                    } else { "PDU was accepted"}, json_text),
                                    format!("<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
                                    if outlier {
                                        "PDU is outlier"
                                    } else { "PDU was accepted"}, RawStr::new(&json_text).html_escape())
                                ),
                            )
                        }
                        None => AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                            "PDU not found.",
                        )),
                    }
                } else {
                    AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                        "Event ID could not be parsed.",
                    ))
                }
            } else {
                AdminCommand::SendMessage(RoomMessageEventContent::text_plain(
                    "Usage: get_pdu <eventid>",
                ))
            }
        }
        "database_memory_usage" => AdminCommand::ShowMemoryUsage,
        _ => {
            let message = format!(
                "Unrecognized command <code>{}</code>, try <code>help</code> for a list of commands.",
                command,
            );
            AdminCommand::SendMessage(RoomMessageEventContent::text_html(
                html_to_markdown(&message),
                message,
            ))
        }
    };

    Ok(command)
}
