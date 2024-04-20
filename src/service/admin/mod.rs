use std::{collections::BTreeMap, sync::Arc};

use clap::Parser;
use regex::Regex;
use ruma::{
	api::client::error::ErrorKind,
	events::{
		relation::InReplyTo,
		room::{
			canonical_alias::RoomCanonicalAliasEventContent,
			create::RoomCreateEventContent,
			guest_access::{GuestAccess, RoomGuestAccessEventContent},
			history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
			join_rules::{JoinRule, RoomJoinRulesEventContent},
			member::{MembershipState, RoomMemberEventContent},
			message::{Relation::Reply, RoomMessageEventContent},
			name::RoomNameEventContent,
			power_levels::RoomPowerLevelsEventContent,
			topic::RoomTopicEventContent,
		},
		TimelineEventType,
	},
	EventId, MxcUri, OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId, RoomVersionId, ServerName, UserId,
};
use serde_json::value::to_raw_value;
use tokio::sync::Mutex;
use tracing::{error, warn};

use super::pdu::PduBuilder;
use crate::{
	service::admin::{
		appservice::AppserviceCommand, debug::DebugCommand, federation::FederationCommand, media::MediaCommand,
		query::QueryCommand, room::RoomCommand, server::ServerCommand, user::UserCommand,
	},
	services, Error, Result,
};

pub(crate) mod appservice;
pub(crate) mod debug;
pub(crate) mod federation;
pub(crate) mod fsck;
pub(crate) mod media;
pub(crate) mod query;
pub(crate) mod room;
pub(crate) mod room_alias;
pub(crate) mod room_directory;
pub(crate) mod room_moderation;
pub(crate) mod server;
pub(crate) mod user;

const PAGE_SIZE: usize = 100;

#[cfg_attr(test, derive(Debug))]
#[derive(Parser)]
#[command(name = "@conduit:server.name:", version = env!("CARGO_PKG_VERSION"))]
enum AdminCommand {
	#[command(subcommand)]
	/// - Commands for managing appservices
	Appservices(AppserviceCommand),

	#[command(subcommand)]
	/// - Commands for managing local users
	Users(UserCommand),

	#[command(subcommand)]
	/// - Commands for managing rooms
	Rooms(RoomCommand),

	#[command(subcommand)]
	/// - Commands for managing federation
	Federation(FederationCommand),

	#[command(subcommand)]
	/// - Commands for managing the server
	Server(ServerCommand),

	#[command(subcommand)]
	/// - Commands for managing media
	Media(MediaCommand),

	#[command(subcommand)]
	/// - Commands for debugging things
	Debug(DebugCommand),

	#[command(subcommand)]
	/// - Query all the database getters and iterators
	Query(QueryCommand),
}

#[derive(Debug)]
pub enum AdminRoomEvent {
	ProcessMessage(String, Arc<EventId>),
	SendMessage(RoomMessageEventContent),
}

pub struct Service {
	pub sender: loole::Sender<AdminRoomEvent>,
	receiver: Mutex<loole::Receiver<AdminRoomEvent>>,
}

impl Service {
	pub fn build() -> Arc<Self> {
		let (sender, receiver) = loole::unbounded();
		Arc::new(Self {
			sender,
			receiver: Mutex::new(receiver),
		})
	}

	pub fn start_handler(self: &Arc<Self>) {
		let self2 = Arc::clone(self);
		tokio::spawn(async move {
			self2
				.handler()
				.await
				.expect("Failed to initialize admin room handler");
		});
	}

	async fn handler(&self) -> Result<()> {
		let receiver = self.receiver.lock().await;
		// TODO: Use futures when we have long admin commands
		//let mut futures = FuturesUnordered::new();

		let conduit_user = UserId::parse(format!("@conduit:{}", services().globals.server_name()))
			.expect("@conduit:server_name is valid");

		if let Ok(Some(conduit_room)) = Self::get_admin_room() {
			loop {
				tokio::select! {
						event = receiver.recv_async() => {
							match event {
								Ok(event) => {
									let (mut message_content, reply) = match event {
										AdminRoomEvent::SendMessage(content) => (content, None),
										AdminRoomEvent::ProcessMessage(room_message, reply_id) => {
											(self.process_admin_message(room_message).await, Some(reply_id))
										}
									};

									let mutex_state = Arc::clone(
										services().globals
											.roomid_mutex_state
											.write()
											.await
											.entry(conduit_room.clone())
											.or_default(),
									);

									let state_lock = mutex_state.lock().await;

									if let Some(reply) = reply {
										message_content.relates_to = Some(Reply { in_reply_to: InReplyTo { event_id: reply.into() } });
									}

									if let Err(e) = services().rooms.timeline.build_and_append_pdu(
										PduBuilder {
										  event_type: TimelineEventType::RoomMessage,
										  content: to_raw_value(&message_content)
											  .expect("event is valid, we just created it"),
										  unsigned: None,
										  state_key: None,
										  redacts: None,
										},
										&conduit_user,
										&conduit_room,
										&state_lock)
									  .await {
										error!("Failed to build and append admin room response PDU: \"{e}\"");

										let error_room_message = RoomMessageEventContent::text_plain(format!("Failed to build and append admin room PDU: \"{e}\"\n\nThe original admin command may have finished successfully, but we could not return the output."));

										services().rooms.timeline.build_and_append_pdu(
											PduBuilder {
											  event_type: TimelineEventType::RoomMessage,
											  content: to_raw_value(&error_room_message)
												  .expect("event is valid, we just created it"),
											  unsigned: None,
											  state_key: None,
											  redacts: None,
											},
											&conduit_user,
											&conduit_room,
											&state_lock)
										  .await?;
									}
									drop(state_lock);
							}
							Err(e) => {
								 // generally shouldn't happen
								 error!("Failed to receive admin room event from channel: {e}");
							}
						}
					}
				}
			}
		}

		Ok(())
	}

	pub fn process_message(&self, room_message: String, event_id: Arc<EventId>) {
		self.sender
			.send(AdminRoomEvent::ProcessMessage(room_message, event_id))
			.unwrap();
	}

	pub fn send_message(&self, message_content: RoomMessageEventContent) {
		self.sender
			.send(AdminRoomEvent::SendMessage(message_content))
			.unwrap();
	}

	// Parse and process a message from the admin room
	async fn process_admin_message(&self, room_message: String) -> RoomMessageEventContent {
		let mut lines = room_message.lines().filter(|l| !l.trim().is_empty());
		let command_line = lines.next().expect("each string has at least one line");
		let body = lines.collect::<Vec<_>>();

		let admin_command = match self.parse_admin_command(command_line) {
			Ok(command) => command,
			Err(error) => {
				let server_name = services().globals.server_name();
				let message = error.replace("server.name", server_name.as_str());
				let html_message = self.usage_to_html(&message, server_name);

				return RoomMessageEventContent::text_html(message, html_message);
			},
		};

		match self.process_admin_command(admin_command, body).await {
			Ok(reply_message) => reply_message,
			Err(error) => {
				let markdown_message = format!("Encountered an error while handling the command:\n```\n{error}\n```",);
				let html_message = format!("Encountered an error while handling the command:\n<pre>\n{error}\n</pre>",);

				RoomMessageEventContent::text_html(markdown_message, html_message)
			},
		}
	}

	// Parse chat messages from the admin room into an AdminCommand object
	fn parse_admin_command(&self, command_line: &str) -> Result<AdminCommand, String> {
		// Note: argv[0] is `@conduit:servername:`, which is treated as the main command
		let mut argv = command_line.split_whitespace().collect::<Vec<_>>();

		// Replace `help command` with `command --help`
		// Clap has a help subcommand, but it omits the long help description.
		if argv.len() > 1 && argv[1] == "help" {
			argv.remove(1);
			argv.push("--help");
		}

		// Backwards compatibility with `register_appservice`-style commands
		let command_with_dashes_argv1;
		if argv.len() > 1 && argv[1].contains('_') {
			command_with_dashes_argv1 = argv[1].replace('_', "-");
			argv[1] = &command_with_dashes_argv1;
		}

		// Backwards compatibility with `register_appservice`-style commands
		let command_with_dashes_argv2;
		if argv.len() > 2 && argv[2].contains('_') {
			command_with_dashes_argv2 = argv[2].replace('_', "-");
			argv[2] = &command_with_dashes_argv2;
		}

		// if the user is using the `query` command (argv[1]), replace the database
		// function/table calls with underscores to match the codebase
		let command_with_dashes_argv3;
		if argv.len() > 3 && argv[1].eq("query") {
			command_with_dashes_argv3 = argv[3].replace('_', "-");
			argv[3] = &command_with_dashes_argv3;
		}

		AdminCommand::try_parse_from(argv).map_err(|error| error.to_string())
	}

	async fn process_admin_command(&self, command: AdminCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
		let reply_message_content = match command {
			AdminCommand::Appservices(command) => appservice::process(command, body).await?,
			AdminCommand::Media(command) => media::process(command, body).await?,
			AdminCommand::Users(command) => user::user::process(command, body).await?,
			AdminCommand::Rooms(command) => room::process(command, body).await?,
			AdminCommand::Federation(command) => federation::process(command, body).await?,
			AdminCommand::Server(command) => server::process(command, body).await?,
			AdminCommand::Debug(command) => debug::debug::process(command, body).await?,
			AdminCommand::Query(command) => query::process(command, body).await?,
		};

		Ok(reply_message_content)
	}

	// Utility to turn clap's `--help` text to HTML.
	fn usage_to_html(&self, text: &str, server_name: &ServerName) -> String {
		// Replace `@conduit:servername:-subcmdname` with `@conduit:servername:
		// subcmdname`
		let text = text.replace(&format!("@conduit:{server_name}:-"), &format!("@conduit:{server_name}: "));

		// For the conduit admin room, subcommands become main commands
		let text = text.replace("SUBCOMMAND", "COMMAND");
		let text = text.replace("subcommand", "command");

		// Escape option names (e.g. `<element-id>`) since they look like HTML tags
		let text = escape_html(&text);

		// Italicize the first line (command name and version text)
		let re = Regex::new("^(.*?)\n").expect("Regex compilation should not fail");
		let text = re.replace_all(&text, "<em>$1</em>\n");

		// Unmerge wrapped lines
		let text = text.replace("\n            ", "  ");

		// Wrap option names in backticks. The lines look like:
		//     -V, --version  Prints version information
		// And are converted to:
		// <code>-V, --version</code>: Prints version information
		// (?m) enables multi-line mode for ^ and $
		let re = Regex::new("(?m)^ {4}(([a-zA-Z_&;-]+(, )?)+)  +(.*)$").expect("Regex compilation should not fail");
		let text = re.replace_all(&text, "<code>$1</code>: $4");

		// Look for a `[commandbody]` tag. If it exists, use all lines below it that
		// start with a `#` in the USAGE section.
		let mut text_lines = text.lines().collect::<Vec<&str>>();
		let mut command_body = String::new();

		if let Some(line_index) = text_lines.iter().position(|line| *line == "[commandbody]") {
			text_lines.remove(line_index);

			while text_lines
				.get(line_index)
				.is_some_and(|line| line.starts_with('#'))
			{
				command_body += if text_lines[line_index].starts_with("# ") {
					&text_lines[line_index][2..]
				} else {
					&text_lines[line_index][1..]
				};
				command_body += "[nobr]\n";
				text_lines.remove(line_index);
			}
		}

		let text = text_lines.join("\n");

		// Improve the usage section
		let text = if command_body.is_empty() {
			// Wrap the usage line in code tags
			let re = Regex::new("(?m)^USAGE:\n {4}(@conduit:.*)$").expect("Regex compilation should not fail");
			re.replace_all(&text, "USAGE:\n<code>$1</code>").to_string()
		} else {
			// Wrap the usage line in a code block, and add a yaml block example
			// This makes the usage of e.g. `register-appservice` more accurate
			let re = Regex::new("(?m)^USAGE:\n {4}(.*?)\n\n").expect("Regex compilation should not fail");
			re.replace_all(&text, "USAGE:\n<pre>$1[nobr]\n[commandbodyblock]</pre>")
				.replace("[commandbodyblock]", &command_body)
		};

		// Add HTML line-breaks

		text.replace("\n\n\n", "\n\n")
			.replace('\n', "<br>\n")
			.replace("[nobr]<br>", "")
	}

	/// Create the admin room.
	///
	/// Users in this room are considered admins by conduit, and the room can be
	/// used to issue admin commands by talking to the server user inside it.
	pub(crate) async fn create_admin_room(&self) -> Result<()> {
		let room_id = RoomId::new(services().globals.server_name());

		services().rooms.short.get_or_create_shortroomid(&room_id)?;

		let mutex_state = Arc::clone(
			services()
				.globals
				.roomid_mutex_state
				.write()
				.await
				.entry(room_id.clone())
				.or_default(),
		);
		let state_lock = mutex_state.lock().await;

		// Create a user for the server
		let conduit_user = UserId::parse_with_server_name("conduit", services().globals.server_name())
			.expect("@conduit:server_name is valid");

		services().users.create(&conduit_user, None)?;

		let room_version = services().globals.default_room_version();
		let mut content = match room_version {
			RoomVersionId::V1
			| RoomVersionId::V2
			| RoomVersionId::V3
			| RoomVersionId::V4
			| RoomVersionId::V5
			| RoomVersionId::V6
			| RoomVersionId::V7
			| RoomVersionId::V8
			| RoomVersionId::V9
			| RoomVersionId::V10 => RoomCreateEventContent::new_v1(conduit_user.clone()),
			RoomVersionId::V11 => RoomCreateEventContent::new_v11(),
			_ => {
				warn!("Unexpected or unsupported room version {}", room_version);
				return Err(Error::BadRequest(
					ErrorKind::BadJson,
					"Unexpected or unsupported room version found",
				));
			},
		};

		content.federate = true;
		content.predecessor = None;
		content.room_version = room_version;

		// 1. The room create event
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomCreate,
					content: to_raw_value(&content).expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 2. Make conduit bot join
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomMember,
					content: to_raw_value(&RoomMemberEventContent {
						membership: MembershipState::Join,
						displayname: None,
						avatar_url: None,
						is_direct: None,
						third_party_invite: None,
						blurhash: None,
						reason: None,
						join_authorized_via_users_server: None,
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(conduit_user.to_string()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 3. Power levels
		let mut users = BTreeMap::new();
		users.insert(conduit_user.clone(), 100.into());

		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomPowerLevels,
					content: to_raw_value(&RoomPowerLevelsEventContent {
						users,
						..Default::default()
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 4.1 Join Rules
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomJoinRules,
					content: to_raw_value(&RoomJoinRulesEventContent::new(JoinRule::Invite))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 4.2 History Visibility
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomHistoryVisibility,
					content: to_raw_value(&RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 4.3 Guest Access
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomGuestAccess,
					content: to_raw_value(&RoomGuestAccessEventContent::new(GuestAccess::Forbidden))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 5. Events implied by name and topic
		let room_name = format!("{} Admin Room", services().globals.server_name());
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomName,
					content: to_raw_value(&RoomNameEventContent::new(room_name))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomTopic,
					content: to_raw_value(&RoomTopicEventContent {
						topic: format!("Manage {}", services().globals.server_name()),
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 6. Room alias
		let alias: OwnedRoomAliasId = format!("#admins:{}", services().globals.server_name())
			.try_into()
			.expect("#admins:server_name is a valid alias name");

		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomCanonicalAlias,
					content: to_raw_value(&RoomCanonicalAliasEventContent {
						alias: Some(alias.clone()),
						alt_aliases: Vec::new(),
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&conduit_user,
				&room_id,
				&state_lock,
			)
			.await?;

		services().rooms.alias.set_alias(&alias, &room_id)?;

		Ok(())
	}

	/// Gets the room ID of the admin room
	///
	/// Errors are propagated from the database, and will have None if there is
	/// no admin room
	pub(crate) fn get_admin_room() -> Result<Option<OwnedRoomId>> {
		let admin_room_alias: Box<RoomAliasId> = format!("#admins:{}", services().globals.server_name())
			.try_into()
			.expect("#admins:server_name is a valid alias name");

		services()
			.rooms
			.alias
			.resolve_local_alias(&admin_room_alias)
	}

	/// Invite the user to the conduit admin room.
	///
	/// In conduit, this is equivalent to granting admin privileges.
	pub(crate) async fn make_user_admin(&self, user_id: &UserId, displayname: String) -> Result<()> {
		if let Some(room_id) = Self::get_admin_room()? {
			let mutex_state = Arc::clone(
				services()
					.globals
					.roomid_mutex_state
					.write()
					.await
					.entry(room_id.clone())
					.or_default(),
			);
			let state_lock = mutex_state.lock().await;

			// Use the server user to grant the new admin's power level
			let conduit_user = UserId::parse_with_server_name("conduit", services().globals.server_name())
				.expect("@conduit:server_name is valid");

			// Invite and join the real user
			services()
				.rooms
				.timeline
				.build_and_append_pdu(
					PduBuilder {
						event_type: TimelineEventType::RoomMember,
						content: to_raw_value(&RoomMemberEventContent {
							membership: MembershipState::Invite,
							displayname: None,
							avatar_url: None,
							is_direct: None,
							third_party_invite: None,
							blurhash: None,
							reason: None,
							join_authorized_via_users_server: None,
						})
						.expect("event is valid, we just created it"),
						unsigned: None,
						state_key: Some(user_id.to_string()),
						redacts: None,
					},
					&conduit_user,
					&room_id,
					&state_lock,
				)
				.await?;
			services()
				.rooms
				.timeline
				.build_and_append_pdu(
					PduBuilder {
						event_type: TimelineEventType::RoomMember,
						content: to_raw_value(&RoomMemberEventContent {
							membership: MembershipState::Join,
							displayname: Some(displayname),
							avatar_url: None,
							is_direct: None,
							third_party_invite: None,
							blurhash: None,
							reason: None,
							join_authorized_via_users_server: None,
						})
						.expect("event is valid, we just created it"),
						unsigned: None,
						state_key: Some(user_id.to_string()),
						redacts: None,
					},
					user_id,
					&room_id,
					&state_lock,
				)
				.await?;

			// Set power level
			let mut users = BTreeMap::new();
			users.insert(conduit_user.clone(), 100.into());
			users.insert(user_id.to_owned(), 100.into());

			services()
				.rooms
				.timeline
				.build_and_append_pdu(
					PduBuilder {
						event_type: TimelineEventType::RoomPowerLevels,
						content: to_raw_value(&RoomPowerLevelsEventContent {
							users,
							..Default::default()
						})
						.expect("event is valid, we just created it"),
						unsigned: None,
						state_key: Some(String::new()),
						redacts: None,
					},
					&conduit_user,
					&room_id,
					&state_lock,
				)
				.await?;

			// Send welcome message
			services().rooms.timeline.build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomMessage,
                content: to_raw_value(&RoomMessageEventContent::text_html(
                        format!("## Thank you for trying out conduwuit!\n\nconduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.\n\nHelpful links:\n> Git and Documentation: https://github.com/girlbossceo/conduwuit\n> Report issues: https://github.com/girlbossceo/conduwuit/issues\n\nFor a list of available commands, send the following message in this room: `@conduit:{}: --help`\n\nHere are some rooms you can join (by typing the command):\n\nconduwuit room (Ask questions and get notified on updates):\n`/join #conduwuit:puppygock.gay`", services().globals.server_name()),
                        format!("<h2>Thank you for trying out conduwuit!</h2>\n<p>conduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.</p>\n<p>Helpful links:</p>\n<blockquote>\n<p>Git and Documentation: https://github.com/girlbossceo/conduwuit<br>Report issues: https://github.com/girlbossceo/conduwuit/issues</p>\n</blockquote>\n<p>For a list of available commands, send the following message in this room: <code>@conduit:{}: --help</code></p>\n<p>Here are some rooms you can join (by typing the command):</p>\n<p>conduwuit room (Ask questions and get notified on updates):<br><code>/join #conduwuit:puppygock.gay</code></p>\n", services().globals.server_name()),
                ))
                .expect("event is valid, we just created it"),
                unsigned: None,
                state_key: None,
                redacts: None,
            },
            &conduit_user,
            &room_id,
            &state_lock,
        ).await?;

			Ok(())
		} else {
			Ok(())
		}
	}
}

fn escape_html(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
}

fn get_room_info(id: &OwnedRoomId) -> (OwnedRoomId, u64, String) {
	(
		id.clone(),
		services()
			.rooms
			.state_cache
			.room_joined_count(id)
			.ok()
			.flatten()
			.unwrap_or(0),
		services()
			.rooms
			.state_accessor
			.get_name(id)
			.ok()
			.flatten()
			.unwrap_or_else(|| id.to_string()),
	)
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn get_help_short() { get_help_inner("-h"); }

	#[test]
	fn get_help_long() { get_help_inner("--help"); }

	#[test]
	fn get_help_subcommand() { get_help_inner("help"); }

	fn get_help_inner(input: &str) {
		let error = AdminCommand::try_parse_from(["argv[0] doesn't matter", input])
			.unwrap_err()
			.to_string();

		// Search for a handful of keywords that suggest the help printed properly
		assert!(error.contains("Usage:"));
		assert!(error.contains("Commands:"));
		assert!(error.contains("Options:"));
	}
}
