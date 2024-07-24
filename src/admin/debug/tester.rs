use ruma::events::room::message::RoomMessageEventContent;

use crate::Result;

#[derive(Debug, clap::Subcommand)]
pub(crate) enum TesterCommand {
	Tester,
	Timer,
}

pub(super) async fn process(command: TesterCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		TesterCommand::Tester => tester(body).await,
		TesterCommand::Timer => timer(body).await,
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
