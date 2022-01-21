use std::{convert::TryFrom, convert::TryInto, sync::Arc, time::Instant};

use crate::{
    error::{Error, Result},
    pdu::PduBuilder,
    server_server, Database, PduEvent,
};
use regex::Regex;
use rocket::{
    futures::{channel::mpsc, stream::StreamExt},
    http::RawStr,
};
use ruma::{
    events::{room::message::RoomMessageEventContent, EventType},
    EventId, RoomId, RoomVersionId, UserId,
};
use serde_json::value::to_raw_value;
use structopt::StructOpt;
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

// Parse chat messages from the admin room into an AdminCommand object
pub fn parse_admin_command(db: &Database, command_line: &str, body: Vec<&str>) -> AdminCommand {
    let mut argv: Vec<_> = command_line.split_whitespace().skip(1).collect();

    let command_name = match argv.get(0) {
        Some(command) => *command,
        None => {
            let markdown_message = "No command given. Use `help` for a list of commands.";
            let html_message = markdown_to_html(&markdown_message);

            return AdminCommand::SendMessage(RoomMessageEventContent::text_html(
                markdown_message,
                html_message,
            ));
        }
    };

    // Backwards compatibility with `register_appservice`-style commands
    let command_with_dashes;
    if command_line.contains("_") {
        command_with_dashes = command_name.replace("_", "-");
        argv[0] = &command_with_dashes;
    }

    match try_parse_admin_command(db, argv, body) {
        Ok(admin_command) => admin_command,
        Err(error) => {
            let markdown_message = format!(
                "Encountered an error while handling the `{}` command:\n\
                ```\n{}\n```",
                command_name, error,
            );
            let html_message = markdown_to_html(&markdown_message);

            AdminCommand::SendMessage(RoomMessageEventContent::text_html(
                markdown_message,
                html_message,
            ))
        }
    }
}

#[derive(StructOpt)]
enum AdminCommands {
    #[structopt(verbatim_doc_comment)]
    /// Register an appservice using its registration YAML
    ///
    /// This command needs a YAML generated by an appservice (such as a bridge),
    /// which must be provided in a Markdown code-block below the command.
    ///
    /// Registering a new bridge using the ID of an existing bridge will replace
    /// the old one.
    ///
    /// Example:
    /// ````
    /// @conduit:example.com: register-appservice
    /// ```
    /// yaml content here
    /// ```
    /// ````
    RegisterAppservice,

    /// Unregister an appservice using its ID
    /// 
    /// You can find the ID using the `list-appservices` command.
    UnregisterAppservice { appservice_identifier: String },

    /// List all the currently registered appservices
    ListAppservices,

    /// Get the auth_chain of a PDU
    GetAuthChain { event_id: Box<EventId> },

    /// Parse and print a PDU from a JSON
    ///
    /// The PDU event is only checked for validity and is not added to the
    /// database.
    ParsePdu,

    /// Retrieve and print a PDU by ID from the Conduit database
    GetPdu { event_id: Box<EventId> },

    /// Print database memory usage statistics
    DatabaseMemoryUsage,
}

pub fn try_parse_admin_command(
    db: &Database,
    mut argv: Vec<&str>,
    body: Vec<&str>,
) -> Result<AdminCommand> {
    argv.insert(0, "@conduit:example.com:");
    let command = match AdminCommands::from_iter_safe(argv) {
        Ok(command) => command,
        Err(error) => {
            println!("Before:\n{}\n", error.to_string());
            let markdown_message = usage_to_markdown(&error.to_string())
                .replace("example.com", db.globals.server_name().as_str());
            let html_message = markdown_to_html(&markdown_message);

            return Ok(AdminCommand::SendMessage(
                RoomMessageEventContent::text_html(markdown_message, html_message),
            ));
        }
    };

    let admin_command = match command {
        AdminCommands::RegisterAppservice => {
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
        AdminCommands::UnregisterAppservice {
            appservice_identifier,
        } => AdminCommand::UnregisterAppservice(appservice_identifier),
        AdminCommands::ListAppservices => AdminCommand::ListAppservices,
        AdminCommands::GetAuthChain { event_id } => {
            let event_id = Arc::<EventId>::from(event_id);
            if let Some(event) = db.rooms.get_pdu_json(&event_id)? {
                let room_id_str = event
                    .get("room_id")
                    .and_then(|val| val.as_str())
                    .ok_or_else(|| Error::bad_database("Invalid event in database"))?;

                let room_id = <&RoomId>::try_from(room_id_str).map_err(|_| {
                    Error::bad_database("Invalid room id field in event in database")
                })?;
                let start = Instant::now();
                let count = server_server::get_auth_chain(room_id, vec![event_id], db)?.count();
                let elapsed = start.elapsed();
                return Ok(AdminCommand::SendMessage(
                    RoomMessageEventContent::text_plain(format!(
                        "Loaded auth chain with length {} in {:?}",
                        count, elapsed
                    )),
                ));
            } else {
                AdminCommand::SendMessage(RoomMessageEventContent::text_plain("Event not found."))
            }
        }
        AdminCommands::ParsePdu => {
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
        AdminCommands::GetPdu { event_id } => {
            let mut outlier = false;
            let mut pdu_json = db.rooms.get_non_outlier_pdu_json(&event_id)?;
            if pdu_json.is_none() {
                outlier = true;
                pdu_json = db.rooms.get_pdu_json(&event_id)?;
            }
            match pdu_json {
                Some(json) => {
                    let json_text =
                        serde_json::to_string_pretty(&json).expect("canonical json is valid json");
                    AdminCommand::SendMessage(RoomMessageEventContent::text_html(
                        format!(
                            "{}\n```json\n{}\n```",
                            if outlier {
                                "PDU is outlier"
                            } else {
                                "PDU was accepted"
                            },
                            json_text
                        ),
                        format!(
                            "<p>{}</p>\n<pre><code class=\"language-json\">{}\n</code></pre>\n",
                            if outlier {
                                "PDU is outlier"
                            } else {
                                "PDU was accepted"
                            },
                            RawStr::new(&json_text).html_escape()
                        ),
                    ))
                }
                None => {
                    AdminCommand::SendMessage(RoomMessageEventContent::text_plain("PDU not found."))
                }
            }
        }
        AdminCommands::DatabaseMemoryUsage => AdminCommand::ShowMemoryUsage,
    };

    Ok(admin_command)
}

// Utility to turn structopt's `--help` text to markdown.
fn usage_to_markdown(text: &str) -> String {
    // For the conduit admin room, subcommands become main commands
    let text = text.replace("SUBCOMMAND", "COMMAND");
    let text = text.replace("subcommand", "command");

    // Put the first line (command name and version text) on its own paragraph
    let re = Regex::new("^(.*?)\n").expect("Regex compilation should not fail");
    let text = re.replace_all(&text, "*$1*\n\n");

    // Wrap command names in backticks
    // (?m) enables multi-line mode for ^ and $
    let re = Regex::new("(?m)^    ([a-z-]+)  +(.*)$").expect("Regex compilation should not fail");
    let text = re.replace_all(&text, "    `$1`: $2");

    // Add * to list items
    let re = Regex::new("(?m)^    (.*)$").expect("Regex compilation should not fail");
    let text = re.replace_all(&text, "* $1");

    // Turn section names to headings
    let re = Regex::new("(?m)^([A-Z-]+):$").expect("Regex compilation should not fail");
    let text = re.replace_all(&text, "#### $1");

    text.to_string()
}

// Convert markdown to HTML using the CommonMark flavor
fn markdown_to_html(text: &str) -> String {
    // CommonMark's spec allows HTML tags; however, CLI required arguments look
    // very much like tags so escape them.
    let text = text.replace("<", "&lt;").replace(">", "&gt;");

    let mut html_output = String::new();

    let parser = pulldown_cmark::Parser::new(&text);
    pulldown_cmark::html::push_html(&mut html_output, parser);

    html_output
}
