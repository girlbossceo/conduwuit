mod commands;

use clap::Subcommand;
use conduit::Result;
use ruma::events::room::message::RoomMessageEventContent;

use self::commands::*;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(super) enum FsckCommand {
	CheckAllUsers,
}

pub(super) async fn process(command: FsckCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		FsckCommand::CheckAllUsers => check_all_users(body).await?,
	})
}
