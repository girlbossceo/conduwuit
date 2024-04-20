use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{services, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum FsckCommand {
	Register,
}

pub(crate) async fn fsck(command: FsckCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		FsckCommand::Register => {
			todo!()
		},
	}
}
