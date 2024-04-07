mod data;

use std::{sync::Arc, time::Duration};

pub use data::Data;
use futures_util::{stream::FuturesUnordered, StreamExt};
use ruma::{
	events::presence::{PresenceEvent, PresenceEventContent},
	presence::PresenceState,
	OwnedUserId, UInt, UserId,
};
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::sleep};
use tracing::{debug, error};

use crate::{services, utils, Config, Error, Result};

/// Represents data required to be kept in order to implement the presence
/// specification.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Presence {
	pub state: PresenceState,
	pub currently_active: bool,
	pub last_active_ts: u64,
	pub status_msg: Option<String>,
}

impl Presence {
	pub fn new(state: PresenceState, currently_active: bool, last_active_ts: u64, status_msg: Option<String>) -> Self {
		Self {
			state,
			currently_active,
			last_active_ts,
			status_msg,
		}
	}

	pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
		serde_json::from_slice(bytes).map_err(|_| Error::bad_database("Invalid presence data in database"))
	}

	pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
		serde_json::to_vec(self).map_err(|_| Error::bad_database("Could not serialize Presence to JSON"))
	}

	/// Creates a PresenceEvent from available data.
	pub fn to_presence_event(&self, user_id: &UserId) -> Result<PresenceEvent> {
		let now = utils::millis_since_unix_epoch();
		let last_active_ago = if self.currently_active {
			None
		} else {
			Some(UInt::new_saturating(now.saturating_sub(self.last_active_ts)))
		};

		Ok(PresenceEvent {
			sender: user_id.to_owned(),
			content: PresenceEventContent {
				presence: self.state.clone(),
				status_msg: self.status_msg.clone(),
				currently_active: Some(self.currently_active),
				last_active_ago,
				displayname: services().users.displayname(user_id)?,
				avatar_url: services().users.avatar_url(user_id)?,
			},
		})
	}
}

pub struct Service {
	pub db: &'static dyn Data,
	pub timer_sender: loole::Sender<(OwnedUserId, Duration)>,
	timer_receiver: Mutex<loole::Receiver<(OwnedUserId, Duration)>>,
	timeout_remote_users: bool,
}

impl Service {
	pub fn build(db: &'static dyn Data, config: &Config) -> Arc<Self> {
		let (timer_sender, timer_receiver) = loole::unbounded();

		Arc::new(Self {
			db,
			timer_sender,
			timer_receiver: Mutex::new(timer_receiver),
			timeout_remote_users: config.presence_timeout_remote_users,
		})
	}

	pub fn start_handler(self: &Arc<Self>) {
		let self_ = Arc::clone(self);
		tokio::spawn(async move {
			self_
				.handler()
				.await
				.expect("Failed to start presence handler");
		});
	}

	/// Returns the latest presence event for the given user.
	pub fn get_presence(&self, user_id: &UserId) -> Result<Option<PresenceEvent>> {
		if let Some((_, presence)) = self.db.get_presence(user_id)? {
			Ok(Some(presence))
		} else {
			Ok(None)
		}
	}

	/// Pings the presence of the given user in the given room, setting the
	/// specified state.
	pub fn ping_presence(&self, user_id: &UserId, new_state: &PresenceState) -> Result<()> {
		const REFRESH_TIMEOUT: u64 = 60 * 25 * 1000;

		let last_presence = self.db.get_presence(user_id)?;
		let state_changed = match last_presence {
			None => true,
			Some((_, ref presence)) => presence.content.presence != *new_state,
		};

		let last_last_active_ago = match last_presence {
			None => 0_u64,
			Some((_, ref presence)) => presence.content.last_active_ago.unwrap_or_default().into(),
		};

		if !state_changed && last_last_active_ago < REFRESH_TIMEOUT {
			return Ok(());
		}

		let status_msg = match last_presence {
			Some((_, ref presence)) => presence.content.status_msg.clone(),
			None => Some(String::new()),
		};

		let last_active_ago = UInt::new(0);
		let currently_active = *new_state == PresenceState::Online;
		self.set_presence(user_id, new_state, Some(currently_active), last_active_ago, status_msg)
	}

	/// Adds a presence event which will be saved until a new event replaces it.
	pub fn set_presence(
		&self, user_id: &UserId, presence_state: &PresenceState, currently_active: Option<bool>,
		last_active_ago: Option<UInt>, status_msg: Option<String>,
	) -> Result<()> {
		self.db
			.set_presence(user_id, presence_state, currently_active, last_active_ago, status_msg)?;

		if self.timeout_remote_users || user_id.server_name() == services().globals.server_name() {
			let timeout = match presence_state {
				PresenceState::Online => services().globals.config.presence_idle_timeout_s,
				_ => services().globals.config.presence_offline_timeout_s,
			};

			self.timer_sender
				.send((user_id.to_owned(), Duration::from_secs(timeout)))
				.map_err(|e| {
					error!("Failed to add presence timer: {}", e);
					Error::bad_database("Failed to add presence timer")
				})?;
		}

		Ok(())
	}

	/// Removes the presence record for the given user from the database.
	pub fn remove_presence(&self, user_id: &UserId) -> Result<()> { self.db.remove_presence(user_id) }

	/// Returns the most recent presence updates that happened after the event
	/// with id `since`.
	pub fn presence_since(&self, since: u64) -> Box<dyn Iterator<Item = (OwnedUserId, u64, PresenceEvent)>> {
		self.db.presence_since(since)
	}

	async fn handler(&self) -> Result<()> {
		let mut presence_timers = FuturesUnordered::new();
		let receiver = self.timer_receiver.lock().await;
		loop {
			tokio::select! {
				event = receiver.recv_async() => {

					match event {
						Ok((user_id, timeout)) => {
							debug!("Adding timer {}: {user_id} timeout:{timeout:?}", presence_timers.len());
							presence_timers.push(presence_timer(user_id, timeout));
						}
						Err(e) => {
							// generally shouldn't happen
							error!("Failed to receive presence timer through channel: {e}");
						}
					}
				}

				Some(user_id) = presence_timers.next() => {
					process_presence_timer(&user_id)?;
				}
			}
		}
	}
}

async fn presence_timer(user_id: OwnedUserId, timeout: Duration) -> OwnedUserId {
	sleep(timeout).await;

	user_id
}

fn process_presence_timer(user_id: &OwnedUserId) -> Result<()> {
	let idle_timeout = services().globals.config.presence_idle_timeout_s * 1_000;
	let offline_timeout = services().globals.config.presence_offline_timeout_s * 1_000;

	let mut presence_state = PresenceState::Offline;
	let mut last_active_ago = None;
	let mut status_msg = None;

	let presence_event = services().presence.get_presence(user_id)?;

	if let Some(presence_event) = presence_event {
		presence_state = presence_event.content.presence;
		last_active_ago = presence_event.content.last_active_ago;
		status_msg = presence_event.content.status_msg;
	}

	let new_state = match (&presence_state, last_active_ago.map(u64::from)) {
		(PresenceState::Online, Some(ago)) if ago >= idle_timeout => Some(PresenceState::Unavailable),
		(PresenceState::Unavailable, Some(ago)) if ago >= offline_timeout => Some(PresenceState::Offline),
		_ => None,
	};

	debug!("Processed presence timer for user '{user_id}': Old state = {presence_state}, New state = {new_state:?}");

	if let Some(new_state) = new_state {
		services()
			.presence
			.set_presence(user_id, &new_state, Some(false), last_active_ago, status_msg)?;
	}

	Ok(())
}
