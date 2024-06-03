use std::time::Duration;

use ruma::events::room::message::RoomMessageEventContent;
use serde::Deserialize;
use tokio::{task::JoinHandle, time::interval};
use tracing::{error, warn};

use crate::{
	conduit::{Error, Result},
	services,
};

const CHECK_FOR_UPDATES_URL: &str = "https://pupbrain.dev/check-for-updates/stable";
const CHECK_FOR_UPDATES_INTERVAL: u64 = 7200; // 2 hours

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
pub fn start_check_for_updates_task() -> JoinHandle<()> {
	let timer_interval = Duration::from_secs(CHECK_FOR_UPDATES_INTERVAL);

	services().server.runtime().spawn(async move {
		let mut i = interval(timer_interval);

		loop {
			i.tick().await;

			if let Err(e) = try_handle_updates().await {
				warn!(%e, "Failed to check for updates");
			}
		}
	})
}

#[tracing::instrument(skip_all)]
async fn try_handle_updates() -> Result<()> {
	let response = services()
		.globals
		.client
		.default
		.get(CHECK_FOR_UPDATES_URL)
		.send()
		.await?;

	let response = serde_json::from_str::<CheckForUpdatesResponse>(&response.text().await?)
		.map_err(|e| Error::Err(format!("Bad check for updates response: {e}")))?;

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
