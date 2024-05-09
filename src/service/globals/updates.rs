use std::time::Duration;

use ruma::events::room::message::RoomMessageEventContent;
use serde::Deserialize;
use tokio::{task::JoinHandle, time::interval};
use tracing::{debug, error};

use crate::{
	conduit::{Error, Result},
	services,
};

#[derive(Deserialize)]
struct CheckForUpdatesResponseEntry {
	id: u64,
	date: String,
	message: String,
}
#[derive(Deserialize)]
struct CheckForUpdatesResponse {
	updates: Vec<CheckForUpdatesResponseEntry>,
}

#[tracing::instrument]
pub async fn start_check_for_updates_task() -> Result<JoinHandle<()>> {
	let timer_interval = Duration::from_secs(7200); // 2 hours

	Ok(services().server.runtime().spawn(async move {
		let mut i = interval(timer_interval);

		loop {
			tokio::select! {
				_ = i.tick() => {
					debug!(target: "start_check_for_updates_task", "Timer ticked");
				},
			}

			_ = try_handle_updates().await;
		}
	}))
}

async fn try_handle_updates() -> Result<()> {
	let response = services()
		.globals
		.client
		.default
		.get("https://pupbrain.dev/check-for-updates/stable")
		.send()
		.await?;

	let response = serde_json::from_str::<CheckForUpdatesResponse>(&response.text().await?).map_err(|e| {
		error!("Bad check for updates response: {e}");
		Error::BadServerResponse("Bad version check response")
	})?;

	let mut last_update_id = services().globals.last_check_for_updates_id()?;
	for update in response.updates {
		last_update_id = last_update_id.max(update.id);
		if update.id > services().globals.last_check_for_updates_id()? {
			error!("{}", update.message);
			services()
				.admin
				.send_message(RoomMessageEventContent::text_plain(format!(
					"@room: the following is a message from the conduwuit puppy. it was sent on '{}':\n\n{}",
					update.date, update.message
				)))
				.await;
		}
	}
	services()
		.globals
		.update_check_for_updates_id(last_update_id)?;

	Ok(())
}
