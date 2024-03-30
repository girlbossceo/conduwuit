use std::collections::BTreeMap;

use ruma::{
	api::federation::transactions::edu::{Edu, TypingContent},
	events::SyncEphemeralRoomEvent,
	OwnedRoomId, OwnedUserId, RoomId, UserId,
};
use tokio::sync::{broadcast, RwLock};
use tracing::debug;

use crate::{services, utils, Result};

pub struct Service {
	pub typing: RwLock<BTreeMap<OwnedRoomId, BTreeMap<OwnedUserId, u64>>>, // u64 is unix timestamp of timeout
	pub last_typing_update: RwLock<BTreeMap<OwnedRoomId, u64>>,            /* timestamp of the last change to typing
	                                                                        * users */
	pub typing_update_sender: broadcast::Sender<OwnedRoomId>,
}

impl Service {
	/// Sets a user as typing until the timeout timestamp is reached or
	/// roomtyping_remove is called.
	pub async fn typing_add(&self, user_id: &UserId, room_id: &RoomId, timeout: u64) -> Result<()> {
		debug!("typing add {:?} in {:?} timeout:{:?}", user_id, room_id, timeout);
		// update clients
		self.typing
			.write()
			.await
			.entry(room_id.to_owned())
			.or_default()
			.insert(user_id.to_owned(), timeout);
		self.last_typing_update
			.write()
			.await
			.insert(room_id.to_owned(), services().globals.next_count()?);
		_ = self.typing_update_sender.send(room_id.to_owned());

		// update federation
		if user_id.server_name() == services().globals.server_name() {
			self.federation_send(room_id, user_id, true)?;
		}

		Ok(())
	}

	/// Removes a user from typing before the timeout is reached.
	pub async fn typing_remove(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
		debug!("typing remove {:?} in {:?}", user_id, room_id);
		// update clients
		self.typing
			.write()
			.await
			.entry(room_id.to_owned())
			.or_default()
			.remove(user_id);
		self.last_typing_update
			.write()
			.await
			.insert(room_id.to_owned(), services().globals.next_count()?);
		_ = self.typing_update_sender.send(room_id.to_owned());

		// update federation
		if user_id.server_name() == services().globals.server_name() {
			self.federation_send(room_id, user_id, false)?;
		}

		Ok(())
	}

	pub async fn wait_for_update(&self, room_id: &RoomId) -> Result<()> {
		let mut receiver = self.typing_update_sender.subscribe();
		while let Ok(next) = receiver.recv().await {
			if next == room_id {
				break;
			}
		}

		Ok(())
	}

	/// Makes sure that typing events with old timestamps get removed.
	async fn typings_maintain(&self, room_id: &RoomId) -> Result<()> {
		let current_timestamp = utils::millis_since_unix_epoch();
		let mut removable = Vec::new();

		{
			let typing = self.typing.read().await;
			let Some(room) = typing.get(room_id) else {
				return Ok(());
			};

			for (user, timeout) in room {
				if *timeout < current_timestamp {
					removable.push(user.clone());
				}
			}

			drop(typing);
		};

		if !removable.is_empty() {
			let typing = &mut self.typing.write().await;
			let room = typing.entry(room_id.to_owned()).or_default();
			for user in &removable {
				debug!("typing maintain remove {:?} in {:?}", &user, room_id);
				room.remove(user);
			}
			// update clients
			self.last_typing_update
				.write()
				.await
				.insert(room_id.to_owned(), services().globals.next_count()?);
			_ = self.typing_update_sender.send(room_id.to_owned());

			// update federation
			for user in removable {
				if user.server_name() == services().globals.server_name() {
					self.federation_send(room_id, &user, false)?;
				}
			}
		}

		Ok(())
	}

	/// Returns the count of the last typing update in this room.
	pub async fn last_typing_update(&self, room_id: &RoomId) -> Result<u64> {
		self.typings_maintain(room_id).await?;
		Ok(self
			.last_typing_update
			.read()
			.await
			.get(room_id)
			.copied()
			.unwrap_or(0))
	}

	/// Returns a new typing EDU.
	pub async fn typings_all(
		&self, room_id: &RoomId,
	) -> Result<SyncEphemeralRoomEvent<ruma::events::typing::TypingEventContent>> {
		Ok(SyncEphemeralRoomEvent {
			content: ruma::events::typing::TypingEventContent {
				user_ids: self
					.typing
					.read()
					.await
					.get(room_id)
					.map(|m| m.keys().cloned().collect())
					.unwrap_or_default(),
			},
		})
	}

	fn federation_send(&self, room_id: &RoomId, user_id: &UserId, typing: bool) -> Result<()> {
		debug_assert!(
			user_id.server_name() == services().globals.server_name(),
			"tried to broadcast typing status of remote user",
		);
		if !services().globals.config.allow_outgoing_typing {
			return Ok(());
		}

		let edu = Edu::Typing(TypingContent::new(room_id.to_owned(), user_id.to_owned(), typing));

		services()
			.sending
			.send_edu_room(room_id, serde_json::to_vec(&edu).expect("Serialized Edu::Typing"))?;

		Ok(())
	}
}
