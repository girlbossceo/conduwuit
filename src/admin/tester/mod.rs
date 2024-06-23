use ruma::events::room::message::RoomMessageEventContent;

use crate::Result;

#[derive(clap::Subcommand)]
#[cfg_attr(test, derive(Debug))]
pub(super) enum TesterCommands {
	Tester,
	Timer,
}

pub(super) async fn process(command: TesterCommands, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		TesterCommands::Tester => tester(body).await,
		TesterCommands::Timer => timer(body).await,
	}
}

#[inline(never)]
#[rustfmt::skip]
#[allow(unused_variables)]
async fn tester(body: Vec<&str>) -> Result<RoomMessageEventContent> {

	Ok(RoomMessageEventContent::notice_plain("completed"))
}

#[inline(never)]
#[rustfmt::skip]
async fn timer(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let started = std::time::Instant::now();
	timed(&body);

	let elapsed = started.elapsed();
	Ok(RoomMessageEventContent::notice_plain(format!("completed in {elapsed:#?}")))
}

#[inline(never)]
#[rustfmt::skip]
#[allow(unused_variables)]
fn timed(body: &[&str]) {

}
