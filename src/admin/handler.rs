use std::{panic::AssertUnwindSafe, sync::Arc, time::Instant};

use clap::{CommandFactory, Parser};
use conduit::{checked, error, trace, utils::string::common_prefix, Error, Result};
use futures_util::future::FutureExt;
use ruma::{
	events::{
		relation::InReplyTo,
		room::message::{Relation::Reply, RoomMessageEventContent},
	},
	OwnedEventId,
};
use service::{
	admin::{CommandInput, CommandOutput, HandlerFuture, HandlerResult},
	Services,
};

use crate::{admin, admin::AdminCommand, Command};

#[must_use]
pub(super) fn complete(line: &str) -> String { complete_command(AdminCommand::command(), line) }

#[must_use]
pub(super) fn handle(services: Arc<Services>, command: CommandInput) -> HandlerFuture {
	Box::pin(handle_command(services, command))
}

#[tracing::instrument(skip_all, name = "admin")]
async fn handle_command(services: Arc<Services>, command: CommandInput) -> HandlerResult {
	AssertUnwindSafe(Box::pin(process_command(services, &command)))
		.catch_unwind()
		.await
		.map_err(Error::from_panic)
		.or_else(|error| handle_panic(&error, command))
}

async fn process_command(services: Arc<Services>, command: &CommandInput) -> CommandOutput {
	process(services, &command.command)
		.await
		.and_then(|content| reply(content, command.reply_id.clone()))
}

fn handle_panic(error: &Error, command: CommandInput) -> HandlerResult {
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
async fn process(services: Arc<Services>, msg: &str) -> CommandOutput {
	let lines = msg.lines().filter(|l| !l.trim().is_empty());
	let command = lines
		.clone()
		.next()
		.expect("each string has at least one line");
	let (parsed, body) = match parse_command(command) {
		Ok(parsed) => parsed,
		Err(error) => {
			let server_name = services.globals.server_name();
			let message = error.replace("server.name", server_name.as_str());
			return Some(RoomMessageEventContent::notice_markdown(message));
		},
	};

	let body = parse_body(AdminCommand::command(), &body, lines.skip(1).collect()).expect("trailing body parsed");
	let context = Command {
		services: &services,
		body: &body,
	};
	let timer = Instant::now();
	let result = Box::pin(admin::process(parsed, &context)).await;
	let elapsed = timer.elapsed();
	conduit::debug!(?command, ok = result.is_ok(), "command processed in {elapsed:?}");
	match result {
		Ok(reply) => Some(reply),
		Err(error) => Some(RoomMessageEventContent::notice_markdown(format!(
			"Encountered an error while handling the command:\n```\n{error:#?}\n```"
		))),
	}
}

// Parse chat messages from the admin room into an AdminCommand object
fn parse_command(command_line: &str) -> Result<(AdminCommand, Vec<String>), String> {
	let argv = parse_line(command_line);
	let com = AdminCommand::try_parse_from(&argv).map_err(|error| error.to_string())?;
	Ok((com, argv))
}

fn parse_body<'a>(mut cmd: clap::Command, body: &'a [String], lines: Vec<&'a str>) -> Result<Vec<&'a str>> {
	let mut start = 1;
	'token: for token in body.iter().skip(1) {
		let cmd_ = cmd.clone();
		for sub in cmd_.get_subcommands() {
			if sub.get_name() == *token {
				start = checked!(start + 1)?;
				cmd = sub.clone();
				continue 'token;
			}
		}

		// positional arguments have to be skipped too
		let num_posargs = cmd_.get_positionals().count();
		start = checked!(start + num_posargs)?;
		break;
	}

	Ok(body
		.iter()
		.skip(start)
		.map(String::as_str)
		.chain(lines)
		.collect::<Vec<&'a str>>())
}

fn complete_command(mut cmd: clap::Command, line: &str) -> String {
	let argv = parse_line(line);
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
fn parse_line(command_line: &str) -> Vec<String> {
	let mut argv = command_line
		.split_whitespace()
		.map(str::to_owned)
		.collect::<Vec<String>>();

	// Remove any escapes that came with a server-side escape command
	if !argv.is_empty() && argv[0].ends_with("admin") {
		argv[0] = argv[0].trim_start_matches('\\').into();
	}

	// First indice has to be "admin" but for console convenience we add it here
	if !argv.is_empty() && !argv[0].ends_with("admin") && !argv[0].starts_with('@') {
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
