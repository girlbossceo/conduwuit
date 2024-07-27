use std::{panic::AssertUnwindSafe, time::Instant};

use clap::{CommandFactory, Parser};
use conduit::{error, trace, utils::string::common_prefix, Error, Result};
use futures_util::future::FutureExt;
use ruma::{
	events::{
		relation::InReplyTo,
		room::message::{Relation::Reply, RoomMessageEventContent},
	},
	OwnedEventId,
};
use service::{
	admin::{CommandInput, CommandOutput, CommandResult, HandlerResult},
	Services,
};

use crate::{admin, admin::AdminCommand, Command};

struct Handler {
	services: &'static Services,
}

#[must_use]
pub(super) fn complete(line: &str) -> String {
	Handler {
		services: service::services(),
	}
	.complete_command(AdminCommand::command(), line)
}

#[must_use]
pub(super) fn handle(command: CommandInput) -> HandlerResult { Box::pin(handle_command(command)) }

#[tracing::instrument(skip_all, name = "admin")]
async fn handle_command(command: CommandInput) -> CommandResult {
	AssertUnwindSafe(Box::pin(process_command(&command)))
		.catch_unwind()
		.await
		.map_err(Error::from_panic)
		.or_else(|error| handle_panic(&error, command))
}

async fn process_command(command: &CommandInput) -> CommandOutput {
	Handler {
		services: service::services(),
	}
	.process(&command.command)
	.await
	.and_then(|content| reply(content, command.reply_id.clone()))
}

fn handle_panic(error: &Error, command: CommandInput) -> CommandResult {
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

impl Handler {
	// Parse and process a message from the admin room
	async fn process(&self, msg: &str) -> CommandOutput {
		let mut lines = msg.lines().filter(|l| !l.trim().is_empty());
		let command = lines.next().expect("each string has at least one line");
		let (parsed, body) = match self.parse_command(command) {
			Ok(parsed) => parsed,
			Err(error) => {
				let server_name = self.services.globals.server_name();
				let message = error.replace("server.name", server_name.as_str());
				return Some(RoomMessageEventContent::notice_markdown(message));
			},
		};

		let timer = Instant::now();
		let body: Vec<&str> = body.iter().map(String::as_str).collect();
		let context = Command {
			services: self.services,
			body: &body,
		};
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
	fn parse_command(&self, command_line: &str) -> Result<(AdminCommand, Vec<String>), String> {
		let argv = self.parse_line(command_line);
		let com = AdminCommand::try_parse_from(&argv).map_err(|error| error.to_string())?;
		Ok((com, argv))
	}

	fn complete_command(&self, mut cmd: clap::Command, line: &str) -> String {
		let argv = self.parse_line(line);
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
	fn parse_line(&self, command_line: &str) -> Vec<String> {
		let mut argv = command_line
			.split_whitespace()
			.map(str::to_owned)
			.collect::<Vec<String>>();

		// Remove any escapes that came with a server-side escape command
		if !argv.is_empty() && argv[0].ends_with("admin") {
			argv[0] = argv[0].trim_start_matches('\\').into();
		}

		// First indice has to be "admin" but for console convenience we add it here
		let server_user = self.services.globals.server_user.as_str();
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
}
