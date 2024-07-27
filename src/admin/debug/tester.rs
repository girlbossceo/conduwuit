use ruma::events::room::message::RoomMessageEventContent;

use crate::{admin_command, admin_command_dispatch, Result};

#[admin_command_dispatch]
#[derive(Debug, clap::Subcommand)]
pub(crate) enum TesterCommand {
	Tester,
	Timer,
}

#[inline(never)]
#[rustfmt::skip]
#[allow(unused_variables)]
#[admin_command]
async fn tester(&self) -> Result<RoomMessageEventContent> {

	Ok(RoomMessageEventContent::notice_plain("completed"))
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
