use std::sync::Arc;

use clap::Parser;
use regex::Regex;
use ruma::{
	events::{
		relation::InReplyTo,
		room::message::{Relation::Reply, RoomMessageEventContent},
		TimelineEventType,
	},
	OwnedRoomId, OwnedUserId, ServerName, UserId,
};
use serde_json::value::to_raw_value;
use tokio::sync::MutexGuard;
use tracing::error;

extern crate conduit_service as service;

use conduit::{Error, Result};
pub(crate) use service::admin::{AdminRoomEvent, Service};
use service::{admin::HandlerResult, pdu::PduBuilder};

use self::{fsck::FsckCommand, tester::TesterCommands};
use crate::{
	appservice, appservice::AppserviceCommand, debug, debug::DebugCommand, escape_html, federation,
	federation::FederationCommand, fsck, media, media::MediaCommand, query, query::QueryCommand, room,
	room::RoomCommand, server, server::ServerCommand, services, tester, user, user::UserCommand,
};
pub(crate) const PAGE_SIZE: usize = 100;

#[cfg_attr(test, derive(Debug))]
#[derive(Parser)]
#[command(name = "@conduit:server.name:", version = env!("CARGO_PKG_VERSION"))]
pub(crate) enum AdminCommand {
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

	#[command(subcommand)]
	/// - Query all the database getters and iterators
	Fsck(FsckCommand),

	#[command(subcommand)]
	Tester(TesterCommands),
}

#[must_use]
pub fn handle(event: AdminRoomEvent, room: OwnedRoomId, user: OwnedUserId) -> HandlerResult {
	Box::pin(handle_event(event, room, user))
}

async fn handle_event(event: AdminRoomEvent, admin_room: OwnedRoomId, server_user: OwnedUserId) -> Result<()> {
	let (mut message_content, reply) = match event {
		AdminRoomEvent::SendMessage(content) => (content, None),
		AdminRoomEvent::ProcessMessage(room_message, reply_id) => {
			(process_admin_message(room_message).await, Some(reply_id))
		},
	};

	let mutex_state = Arc::clone(
		services()
			.globals
			.roomid_mutex_state
			.write()
			.await
			.entry(admin_room.clone())
			.or_default(),
	);
	let state_lock = mutex_state.lock().await;

	if let Some(reply) = reply {
		message_content.relates_to = Some(Reply {
			in_reply_to: InReplyTo {
				event_id: reply.into(),
			},
		});
	}

	let response_pdu = PduBuilder {
		event_type: TimelineEventType::RoomMessage,
		content: to_raw_value(&message_content).expect("event is valid, we just created it"),
		unsigned: None,
		state_key: None,
		redacts: None,
	};

	if let Err(e) = services()
		.rooms
		.timeline
		.build_and_append_pdu(response_pdu, &server_user, &admin_room, &state_lock)
		.await
	{
		handle_response_error(&e, &admin_room, &server_user, &state_lock).await?;
	}

	Ok(())
}

async fn handle_response_error(
	e: &Error, admin_room: &OwnedRoomId, server_user: &UserId, state_lock: &MutexGuard<'_, ()>,
) -> Result<()> {
	error!("Failed to build and append admin room response PDU: \"{e}\"");
	let error_room_message = RoomMessageEventContent::text_plain(format!(
		"Failed to build and append admin room PDU: \"{e}\"\n\nThe original admin command may have finished \
		 successfully, but we could not return the output."
	));

	let response_pdu = PduBuilder {
		event_type: TimelineEventType::RoomMessage,
		content: to_raw_value(&error_room_message).expect("event is valid, we just created it"),
		unsigned: None,
		state_key: None,
		redacts: None,
	};

	services()
		.rooms
		.timeline
		.build_and_append_pdu(response_pdu, server_user, admin_room, state_lock)
		.await?;

	Ok(())
}

// Parse and process a message from the admin room
async fn process_admin_message(room_message: String) -> RoomMessageEventContent {
	let mut lines = room_message.lines().filter(|l| !l.trim().is_empty());
	let command_line = lines.next().expect("each string has at least one line");
	let body = lines.collect::<Vec<_>>();

	let admin_command = match parse_admin_command(command_line) {
		Ok(command) => command,
		Err(error) => {
			let server_name = services().globals.server_name();
			let message = error.replace("server.name", server_name.as_str());
			let html_message = usage_to_html(&message, server_name);

			return RoomMessageEventContent::text_html(message, html_message);
		},
	};

	match process_admin_command(admin_command, body).await {
		Ok(reply_message) => reply_message,
		Err(error) => {
			let markdown_message = format!("Encountered an error while handling the command:\n```\n{error}\n```",);
			let html_message = format!("Encountered an error while handling the command:\n<pre>\n{error}\n</pre>",);

			RoomMessageEventContent::text_html(markdown_message, html_message)
		},
	}
}

// Parse chat messages from the admin room into an AdminCommand object
fn parse_admin_command(command_line: &str) -> Result<AdminCommand, String> {
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

async fn process_admin_command(command: AdminCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let reply_message_content = match command {
		AdminCommand::Appservices(command) => appservice::process(command, body).await?,
		AdminCommand::Media(command) => media::process(command, body).await?,
		AdminCommand::Users(command) => user::process(command, body).await?,
		AdminCommand::Rooms(command) => room::process(command, body).await?,
		AdminCommand::Federation(command) => federation::process(command, body).await?,
		AdminCommand::Server(command) => server::process(command, body).await?,
		AdminCommand::Debug(command) => debug::process(command, body).await?,
		AdminCommand::Query(command) => query::process(command, body).await?,
		AdminCommand::Fsck(command) => fsck::process(command, body).await?,
		AdminCommand::Tester(command) => tester::process(command, body).await?,
	};

	Ok(reply_message_content)
}

// Utility to turn clap's `--help` text to HTML.
fn usage_to_html(text: &str, server_name: &ServerName) -> String {
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
