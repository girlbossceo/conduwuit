use std::{panic::AssertUnwindSafe, time::Instant};

use clap::{CommandFactory, Parser};
use conduit::{error, trace, Error};
use futures_util::future::FutureExt;
use ruma::{
	events::{
		relation::InReplyTo,
		room::message::{Relation::Reply, RoomMessageEventContent},
	},
	OwnedEventId,
};

extern crate conduit_service as service;

use conduit::{utils::string::common_prefix, Result};
pub(crate) use service::admin::{Command, Service};
use service::admin::{CommandOutput, CommandResult, HandlerResult};

use crate::{
	appservice, appservice::AppserviceCommand, check, check::CheckCommand, debug, debug::DebugCommand, federation,
	federation::FederationCommand, media, media::MediaCommand, query, query::QueryCommand, room, room::RoomCommand,
	server, server::ServerCommand, services, user, user::UserCommand,
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
	/// - Commands for checking integrity
	Check(CheckCommand),

	#[command(subcommand)]
	/// - Commands for debugging things
	Debug(DebugCommand),

	#[command(subcommand)]
	/// - Low-level queries for database getters and iterators
	Query(QueryCommand),
}

#[must_use]
pub(crate) fn handle(command: Command) -> HandlerResult { Box::pin(handle_command(command)) }

#[must_use]
pub(crate) fn complete(line: &str) -> String { complete_admin_command(AdminCommand::command(), line) }

#[tracing::instrument(skip_all, name = "admin")]
async fn handle_command(command: Command) -> CommandResult {
	AssertUnwindSafe(process_command(&command))
		.catch_unwind()
		.await
		.map_err(Error::from_panic)
		.or_else(|error| handle_panic(&error, command))
}

async fn process_command(command: &Command) -> CommandOutput {
	process_admin_message(&command.command)
		.await
		.and_then(|content| reply(content, command.reply_id.clone()))
}

fn handle_panic(error: &Error, command: Command) -> CommandResult {
	let link = "Please submit a [bug report](https://github.com/girlbossceo/conduwuit/issues/new). 🥺";
	let msg = format!("Panic occurred while processing command:\n```\n{error:#?}\n```\n{link}");
	let content = RoomMessageEventContent::notice_markdown(msg);
	error!("Panic while processing command: {error:?}");
	Ok(reply(content, command.reply_id))
}

fn reply(mut content: RoomMessageEventContent, reply_id: Option<OwnedEventId>) -> Option<RoomMessageEventContent> {
	content.relates_to = reply_id.map(|event_id| Reply {
		in_reply_to: InReplyTo {
			event_id,
		},
	});

	Some(content)
}

// Parse and process a message from the admin room
async fn process_admin_message(msg: &str) -> CommandOutput {
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
			"Encountered an error while handling the command:\n```\n{error:#?}\n```"
		))),
	}
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
		AdminCommand::Check(command) => check::process(command, body).await?,
	};

	Ok(reply_message_content)
}

// Parse chat messages from the admin room into an AdminCommand object
fn parse_admin_command(command_line: &str) -> Result<AdminCommand, String> {
	let argv = parse_command_line(command_line);
	AdminCommand::try_parse_from(argv).map_err(|error| error.to_string())
}

fn complete_admin_command(mut cmd: clap::Command, line: &str) -> String {
	let argv = parse_command_line(line);
	let mut ret = Vec::<String>::with_capacity(argv.len().saturating_add(1));

	'token: for token in argv.into_iter().skip(1) {
		let cmd_ = cmd.clone();
		let mut choice = Vec::new();

		for sub in cmd_.get_subcommands() {
			let name = sub.get_name();
			if *name == token {
				// token already complete; recurse to subcommand
				ret.push(token);
				cmd.clone_from(sub);
				continue 'token;
			} else if name.starts_with(&token) {
				// partial match; add to choices
				choice.push(name);
			}
		}

		if choice.len() == 1 {
			// One choice. Add extra space because it's complete
			let choice = *choice.first().expect("only choice");
			ret.push(choice.to_owned());
			ret.push(String::new());
		} else if choice.is_empty() {
			// Nothing found, return original string
			ret.push(token);
		} else {
			// Find the common prefix
			ret.push(common_prefix(&choice).into());
		}

		// Return from completion
		return ret.join(" ");
	}

	// Return from no completion. Needs a space though.
	ret.push(String::new());
	ret.join(" ")
}

// Parse chat messages from the admin room into an AdminCommand object
fn parse_command_line(command_line: &str) -> Vec<String> {
	let mut argv = command_line
		.split_whitespace()
		.map(str::to_owned)
		.collect::<Vec<String>>();

	// Remove any escapes that came with a server-side escape command
	if !argv.is_empty() && argv[0].ends_with("admin") {
		argv[0] = argv[0].trim_start_matches('\\').into();
	}

	// First indice has to be "admin" but for console convenience we add it here
	let server_user = services().globals.server_user.as_str();
	if !argv.is_empty() && !argv[0].ends_with("admin") && !argv[0].starts_with(server_user) {
		argv.insert(0, "admin".to_owned());
	}

	// Replace `help command` with `command --help`
	// Clap has a help subcommand, but it omits the long help description.
	if argv.len() > 1 && argv[1] == "help" {
		argv.remove(1);
		argv.push("--help".to_owned());
	}

	// Backwards compatibility with `register_appservice`-style commands
	if argv.len() > 1 && argv[1].contains('_') {
		argv[1] = argv[1].replace('_', "-");
	}

	// Backwards compatibility with `register_appservice`-style commands
	if argv.len() > 2 && argv[2].contains('_') {
		argv[2] = argv[2].replace('_', "-");
	}

	// if the user is using the `query` command (argv[1]), replace the database
	// function/table calls with underscores to match the codebase
	if argv.len() > 3 && argv[1].eq("query") {
		argv[3] = argv[3].replace('_', "-");
	}

	trace!(?command_line, ?argv, "parse");
	argv
}
