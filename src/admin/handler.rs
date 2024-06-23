use std::time::Instant;

use clap::Parser;
use conduit::trace;
use ruma::events::{
	relation::InReplyTo,
	room::message::{Relation::Reply, RoomMessageEventContent},
};

extern crate conduit_service as service;

use conduit::Result;
pub(crate) use service::admin::{Command, Service};
use service::admin::{CommandOutput, CommandResult, HandlerResult};

use crate::{
	appservice, appservice::AppserviceCommand, debug, debug::DebugCommand, federation, federation::FederationCommand,
	fsck, fsck::FsckCommand, media, media::MediaCommand, query, query::QueryCommand, room, room::RoomCommand, server,
	server::ServerCommand, services, user, user::UserCommand,
};
pub(crate) const PAGE_SIZE: usize = 100;

#[cfg_attr(test, derive(Debug))]
#[derive(Parser)]
#[command(name = "admin", version = env!("CARGO_PKG_VERSION"))]
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
}

#[must_use]
pub fn handle(command: Command) -> HandlerResult { Box::pin(handle_command(command)) }

#[tracing::instrument(skip_all, name = "admin")]
async fn handle_command(command: Command) -> CommandResult {
	let Some(mut content) = process_admin_message(command.command).await else {
		return Ok(None);
	};

	content.relates_to = command.reply_id.map(|event_id| Reply {
		in_reply_to: InReplyTo {
			event_id,
		},
	});

	Ok(Some(content))
}

// Parse and process a message from the admin room
async fn process_admin_message(msg: String) -> CommandOutput {
	let mut lines = msg.lines().filter(|l| !l.trim().is_empty());
	let command = lines.next().expect("each string has at least one line");
	let body = lines.collect::<Vec<_>>();
	let parsed = match parse_admin_command(command) {
		Ok(parsed) => parsed,
		Err(error) => {
			let server_name = services().globals.server_name();
			let message = error.replace("server.name", server_name.as_str());
			return Some(RoomMessageEventContent::notice_markdown(message));
		},
	};

	let timer = Instant::now();
	let result = process_admin_command(parsed, body).await;
	let elapsed = timer.elapsed();
	conduit::debug!(?command, ok = result.is_ok(), "command processed in {elapsed:?}");
	match result {
		Ok(reply) => Some(reply),
		Err(error) => Some(RoomMessageEventContent::notice_markdown(format!(
			"Encountered an error while handling the command:\n```\n{error}\n```"
		))),
	}
}

// Parse chat messages from the admin room into an AdminCommand object
fn parse_admin_command(command_line: &str) -> Result<AdminCommand, String> {
	let mut argv = command_line.split_whitespace().collect::<Vec<_>>();

	// Remove any escapes that came with a server-side escape command
	if !argv.is_empty() && argv[0].ends_with("admin") {
		argv[0] = argv[0].trim_start_matches('\\');
	}

	// First indice has to be "admin" but for console convenience we add it here
	let server_user = services().globals.server_user.as_str();
	if !argv.is_empty() && !argv[0].ends_with("admin") && !argv[0].starts_with(server_user) {
		argv.insert(0, "admin");
	}

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

	trace!(?command_line, ?argv, "parse");
	AdminCommand::try_parse_from(argv).map_err(|error| error.to_string())
}

#[tracing::instrument(skip_all, name = "command")]
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
	};

	Ok(reply_message_content)
}
