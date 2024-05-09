use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use self::fsck_commands::check_all_users;
use crate::Result;

pub(crate) mod fsck_commands;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum FsckCommand {
	CheckAllUsers,
}

pub(crate) async fn process(command: FsckCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		FsckCommand::CheckAllUsers => check_all_users(body).await?,
	})
}
