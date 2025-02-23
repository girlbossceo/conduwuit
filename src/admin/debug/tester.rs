use conduwuit::Err;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{Result, admin_command, admin_command_dispatch};

#[admin_command_dispatch]
#[derive(Debug, clap::Subcommand)]
pub(crate) enum TesterCommand {
	Panic,
	Failure,
	Tester,
	Timer,
}

#[rustfmt::skip]
#[admin_command]
async fn panic(&self) -> Result<RoomMessageEventContent> {

	panic!("panicked")
}

#[rustfmt::skip]
#[admin_command]
async fn failure(&self) -> Result<RoomMessageEventContent> {

	Err!("failed")
}

#[inline(never)]
#[rustfmt::skip]
#[admin_command]
async fn tester(&self) -> Result<RoomMessageEventContent> {

	Ok(RoomMessageEventContent::notice_plain("legacy"))
}

#[inline(never)]
#[rustfmt::skip]
#[admin_command]
async fn timer(&self) -> Result<RoomMessageEventContent> {
	let started = std::time::Instant::now();
	timed(self.body);

	let elapsed = started.elapsed();
	Ok(RoomMessageEventContent::notice_plain(format!("completed in {elapsed:#?}")))
}

#[inline(never)]
#[rustfmt::skip]
#[allow(unused_variables)]
fn timed(body: &[&str]) {

}
