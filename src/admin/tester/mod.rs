use ruma::events::room::message::RoomMessageEventContent;

use crate::Result;

#[cfg_attr(test, derive(Debug))]
#[derive(clap::Subcommand)]
pub(crate) enum TesterCommands {
	Tester,
}
pub(crate) async fn process(command: TesterCommands, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		TesterCommands::Tester => RoomMessageEventContent::notice_plain(String::from("completed")),
	})
}
