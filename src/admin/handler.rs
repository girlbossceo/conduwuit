use std::{
	panic::AssertUnwindSafe,
	sync::{Arc, Mutex},
	time::SystemTime,
};

use clap::{CommandFactory, Parser};
use conduit::{
	debug, error,
	log::{
		capture,
		capture::Capture,
		fmt::{markdown_table, markdown_table_head},
	},
	trace,
	utils::string::{collect_stream, common_prefix},
	Error, Result,
};
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
use tracing::Level;

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

async fn process_command(services: Arc<Services>, input: &CommandInput) -> CommandOutput {
	let (command, args, body) = match parse(&services, input) {
		Err(error) => return error,
		Ok(parsed) => parsed,
	};

	let context = Command {
		services: &services,
		body: &body,
		timer: SystemTime::now(),
	};

	process(&context, command, &args)
		.await
		.and_then(|content| reply(content, input.reply_id.clone()))
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
async fn process(context: &Command<'_>, command: AdminCommand, args: &[String]) -> CommandOutput {
	let filter: &capture::Filter =
		&|data| data.level() <= Level::DEBUG && data.our_modules() && data.scope.contains(&"admin");
	let logs = Arc::new(Mutex::new(
		collect_stream(|s| markdown_table_head(s)).expect("markdown table header"),
	));

	let capture = Capture::new(
		&context.services.server.log.capture,
		Some(filter),
		capture::fmt(markdown_table, logs.clone()),
	);

	let capture_scope = capture.start();
	let result = Box::pin(admin::process(command, context)).await;
	drop(capture_scope);

	debug!(
		ok = result.is_ok(),
		elapsed = ?context.timer.elapsed(),
		command = ?args,
		"command processed"
	);

	let logs = logs.lock().expect("locked");
	let output = match result {
		Err(error) => format!("{logs}\nEncountered an error while handling the command:\n```\n{error:#?}\n```"),
		Ok(reply) => format!("{logs}\n{}", reply.body()), //TODO: content is recreated to add logs
	};

	Some(RoomMessageEventContent::notice_markdown(output))
}

// Parse chat messages from the admin room into an AdminCommand object
fn parse<'a>(
	services: &Arc<Services>, input: &'a CommandInput,
) -> Result<(AdminCommand, Vec<String>, Vec<&'a str>), CommandOutput> {
	let lines = input.command.lines().filter(|line| !line.trim().is_empty());
	let command_line = lines.clone().next().expect("command missing first line");
	let body = lines.skip(1).collect();
	match parse_command(command_line) {
		Ok((command, args)) => Ok((command, args, body)),
		Err(error) => {
			let message = error
				.to_string()
				.replace("server.name", services.globals.server_name().as_str());
			Err(Some(RoomMessageEventContent::notice_markdown(message)))
		},
	}
}

fn parse_command(line: &str) -> Result<(AdminCommand, Vec<String>)> {
	let argv = parse_line(line);
	let command = AdminCommand::try_parse_from(&argv)?;
	Ok((command, argv))
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
