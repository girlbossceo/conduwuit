use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use crate::Result;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum FsckCommand {
	Register,
}

#[allow(dead_code)]
pub(crate) async fn fsck(command: FsckCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		FsckCommand::Register => {
			todo!()
		},
	}
}
