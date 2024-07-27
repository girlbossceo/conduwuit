use clap::Subcommand;
use conduit::Result;
use ruma::events::room::message::RoomMessageEventContent;

use crate::Command;

#[derive(Debug, Subcommand)]
/// All the getters and iterators from src/database/key_value/users.rs
pub(crate) enum UsersCommand {
	Iter,
}

/// All the getters and iterators in key_value/users.rs
pub(super) async fn process(subcommand: UsersCommand, context: &Command<'_>) -> Result<RoomMessageEventContent> {
	let services = context.services;

	match subcommand {
		UsersCommand::Iter => {
			let timer = tokio::time::Instant::now();
			let results = services.users.db.iter();
			let users = results.collect::<Vec<_>>();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{users:#?}\n```"
			)))
		},
	}
}
