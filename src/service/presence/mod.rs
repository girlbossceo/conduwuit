mod data;
mod presence;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use conduit::{checked, debug, error, result::LogErr, Error, Result, Server};
use futures::{stream::FuturesUnordered, Stream, StreamExt, TryFutureExt};
use ruma::{events::presence::PresenceEvent, presence::PresenceState, OwnedUserId, UInt, UserId};
use tokio::{sync::Mutex, time::sleep};

use self::{data::Data, presence::Presence};
use crate::{globals, users, Dep};

pub struct Service {
	timer_sender: loole::Sender<TimerType>,
	timer_receiver: Mutex<loole::Receiver<TimerType>>,
	timeout_remote_users: bool,
	idle_timeout: u64,
	offline_timeout: u64,
	pub db: Data,
	services: Services,
}

struct Services {
	server: Arc<Server>,
	globals: Dep<globals::Service>,
	users: Dep<users::Service>,
}

type TimerType = (OwnedUserId, Duration);

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let config = &args.server.config;
		let idle_timeout_s = config.presence_idle_timeout_s;
		let offline_timeout_s = config.presence_offline_timeout_s;
		let (timer_sender, timer_receiver) = loole::unbounded();
		Ok(Arc::new(Self {
			timer_sender,
			timer_receiver: Mutex::new(timer_receiver),
			timeout_remote_users: config.presence_timeout_remote_users,
			idle_timeout: checked!(idle_timeout_s * 1_000)?,
			offline_timeout: checked!(offline_timeout_s * 1_000)?,
			db: Data::new(&args),
			services: Services {
				server: args.server.clone(),
				globals: args.depend::<globals::Service>("globals"),
				users: args.depend::<users::Service>("users"),
			},
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		let mut presence_timers = FuturesUnordered::new();
		let receiver = self.timer_receiver.lock().await;
		while !receiver.is_closed() {
			tokio::select! {
				Some(user_id) = presence_timers.next() => {
					self.process_presence_timer(&user_id).await.log_err().ok();
				},
				event = receiver.recv_async() => match event {
					Err(_) => break,
					Ok((user_id, timeout)) => {
						debug!("Adding timer {}: {user_id} timeout:{timeout:?}", presence_timers.len());
						presence_timers.push(presence_timer(user_id, timeout));
					},
				},
			}
		}

		Ok(())
	}

	fn interrupt(&self) {
		if !self.timer_sender.is_closed() {
			self.timer_sender.close();
		}
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Returns the latest presence event for the given user.
	#[inline]
	pub async fn get_presence(&self, user_id: &UserId) -> Result<PresenceEvent> {
		self.db
			.get_presence(user_id)
			.map_ok(|(_, presence)| presence)
			.await
	}

	/// Pings the presence of the given user in the given room, setting the
	/// specified state.
	pub async fn ping_presence(&self, user_id: &UserId, new_state: &PresenceState) -> Result<()> {
		const REFRESH_TIMEOUT: u64 = 60 * 25 * 1000;

		let last_presence = self.db.get_presence(user_id).await;
		let state_changed = match last_presence {
			Err(_) => true,
			Ok((_, ref presence)) => presence.content.presence != *new_state,
		};

		let last_last_active_ago = match last_presence {
			Err(_) => 0_u64,
			Ok((_, ref presence)) => presence.content.last_active_ago.unwrap_or_default().into(),
		};

		if !state_changed && last_last_active_ago < REFRESH_TIMEOUT {
			return Ok(());
		}

		let status_msg = match last_presence {
			Ok((_, ref presence)) => presence.content.status_msg.clone(),
			Err(_) => Some(String::new()),
		};

		let last_active_ago = UInt::new(0);
		let currently_active = *new_state == PresenceState::Online;
		self.set_presence(user_id, new_state, Some(currently_active), last_active_ago, status_msg)
			.await
	}

	/// Adds a presence event which will be saved until a new event replaces it.
	pub async fn set_presence(
		&self, user_id: &UserId, state: &PresenceState, currently_active: Option<bool>, last_active_ago: Option<UInt>,
		status_msg: Option<String>,
	) -> Result<()> {
		let presence_state = match state.as_str() {
			"" => &PresenceState::Offline, // default an empty string to 'offline'
			&_ => state,
		};

		self.db
			.set_presence(user_id, presence_state, currently_active, last_active_ago, status_msg)
			.await?;

		if self.timeout_remote_users || self.services.globals.user_is_local(user_id) {
			let timeout = match presence_state {
				PresenceState::Online => self.services.server.config.presence_idle_timeout_s,
				_ => self.services.server.config.presence_offline_timeout_s,
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
	///
	/// TODO: Why is this not used?
	#[allow(dead_code)]
	pub async fn remove_presence(&self, user_id: &UserId) { self.db.remove_presence(user_id).await }

	/// Returns the most recent presence updates that happened after the event
	/// with id `since`.
	pub fn presence_since(&self, since: u64) -> impl Stream<Item = (&UserId, u64, &[u8])> + Send + '_ {
		self.db.presence_since(since)
	}

	#[inline]
	pub async fn from_json_bytes_to_event(&self, bytes: &[u8], user_id: &UserId) -> Result<PresenceEvent> {
		let presence = Presence::from_json_bytes(bytes)?;
		let event = presence
			.to_presence_event(user_id, &self.services.users)
			.await;

		Ok(event)
	}

	async fn process_presence_timer(&self, user_id: &OwnedUserId) -> Result<()> {
		let mut presence_state = PresenceState::Offline;
		let mut last_active_ago = None;
		let mut status_msg = None;

		let presence_event = self.get_presence(user_id).await;

		if let Ok(presence_event) = presence_event {
			presence_state = presence_event.content.presence;
			last_active_ago = presence_event.content.last_active_ago;
			status_msg = presence_event.content.status_msg;
		}

		let new_state = match (&presence_state, last_active_ago.map(u64::from)) {
			(PresenceState::Online, Some(ago)) if ago >= self.idle_timeout => Some(PresenceState::Unavailable),
			(PresenceState::Unavailable, Some(ago)) if ago >= self.offline_timeout => Some(PresenceState::Offline),
			_ => None,
		};

		debug!(
			"Processed presence timer for user '{user_id}': Old state = {presence_state}, New state = {new_state:?}"
		);

		if let Some(new_state) = new_state {
			self.set_presence(user_id, &new_state, Some(false), last_active_ago, status_msg)
				.await?;
		}

		Ok(())
	}
}

async fn presence_timer(user_id: OwnedUserId, timeout: Duration) -> OwnedUserId {
	sleep(timeout).await;

	user_id
}
