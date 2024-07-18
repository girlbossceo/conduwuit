use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use conduit::{err, info, utils, warn, Error, Result};
use database::Map;
use ruma::events::room::message::RoomMessageEventContent;
use serde::Deserialize;
use tokio::{sync::Notify, time::interval};

use crate::{admin, client, Dep};

pub struct Service {
	services: Services,
	db: Arc<Map>,
	interrupt: Notify,
	interval: Duration,
}

struct Services {
	admin: Dep<admin::Service>,
	client: Dep<client::Service>,
}

#[derive(Deserialize)]
struct CheckForUpdatesResponse {
	updates: Vec<CheckForUpdatesResponseEntry>,
}

#[derive(Deserialize)]
struct CheckForUpdatesResponseEntry {
	id: u64,
	date: String,
	message: String,
}

const CHECK_FOR_UPDATES_URL: &str = "https://pupbrain.dev/check-for-updates/stable";
const CHECK_FOR_UPDATES_INTERVAL: u64 = 7200; // 2 hours
const LAST_CHECK_FOR_UPDATES_COUNT: &[u8] = b"u";

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				admin: args.depend::<admin::Service>("admin"),
				client: args.depend::<client::Service>("client"),
			},
			db: args.db["global"].clone(),
			interrupt: Notify::new(),
			interval: Duration::from_secs(CHECK_FOR_UPDATES_INTERVAL),
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		let mut i = interval(self.interval);
		loop {
			tokio::select! {
				() = self.interrupt.notified() => return Ok(()),
				_ = i.tick() => (),
			}

			if let Err(e) = self.handle_updates().await {
				warn!(%e, "Failed to check for updates");
			}
		}
	}

	fn interrupt(&self) { self.interrupt.notify_waiters(); }

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	#[tracing::instrument(skip_all)]
	async fn handle_updates(&self) -> Result<()> {
		let response = self
			.services
			.client
			.default
			.get(CHECK_FOR_UPDATES_URL)
			.send()
			.await?;

		let response = serde_json::from_str::<CheckForUpdatesResponse>(&response.text().await?)
			.map_err(|e| err!("Bad check for updates response: {e}"))?;

		let mut last_update_id = self.last_check_for_updates_id()?;
		for update in response.updates {
			last_update_id = last_update_id.max(update.id);
			if update.id > self.last_check_for_updates_id()? {
				info!("{:#}", update.message);
				self.services
					.admin
					.send_message(RoomMessageEventContent::text_markdown(format!(
						"### the following is a message from the conduwuit puppy\n\nit was sent on `{}`:\n\n@room: {}",
						update.date, update.message
					)))
					.await;
			}
		}
		self.update_check_for_updates_id(last_update_id)?;

		Ok(())
	}

	#[inline]
	pub fn update_check_for_updates_id(&self, id: u64) -> Result<()> {
		self.db
			.insert(LAST_CHECK_FOR_UPDATES_COUNT, &id.to_be_bytes())?;

		Ok(())
	}

	pub fn last_check_for_updates_id(&self) -> Result<u64> {
		self.db
			.get(LAST_CHECK_FOR_UPDATES_COUNT)?
			.map_or(Ok(0_u64), |bytes| {
				utils::u64_from_bytes(&bytes)
					.map_err(|_| Error::bad_database("last check for updates count has invalid bytes."))
			})
	}
}
