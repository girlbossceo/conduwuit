mod commands;

use clap::Subcommand;
use conduit::Result;
use ruma::events::room::message::RoomMessageEventContent;

use crate::Command;

#[derive(Debug, Subcommand)]
pub(super) enum CheckCommand {
	AllUsers,
}

pub(super) async fn process(command: CheckCommand, context: &Command<'_>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		CheckCommand::AllUsers => context.check_all_users().await?,
	})
}
