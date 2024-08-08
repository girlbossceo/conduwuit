mod data;

use std::sync::Arc;

use conduit::Result;
use futures::{pin_mut, Stream, StreamExt};
use ruma::{RoomId, UserId};

use self::data::Data;

pub struct Service {
	db: Data,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	#[inline]
	pub fn reset_notification_counts(&self, user_id: &UserId, room_id: &RoomId) {
		self.db.reset_notification_counts(user_id, room_id);
	}

	#[inline]
	pub async fn notification_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		self.db.notification_count(user_id, room_id).await
	}

	#[inline]
	pub async fn highlight_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		self.db.highlight_count(user_id, room_id).await
	}

	#[inline]
	pub async fn last_notification_read(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		self.db.last_notification_read(user_id, room_id).await
	}

	#[inline]
	pub async fn associate_token_shortstatehash(&self, room_id: &RoomId, token: u64, shortstatehash: u64) {
		self.db
			.associate_token_shortstatehash(room_id, token, shortstatehash)
			.await;
	}

	#[inline]
	pub async fn get_token_shortstatehash(&self, room_id: &RoomId, token: u64) -> Result<u64> {
		self.db.get_token_shortstatehash(room_id, token).await
	}

	#[inline]
	pub fn get_shared_rooms<'a>(
		&'a self, user_a: &'a UserId, user_b: &'a UserId,
	) -> impl Stream<Item = &RoomId> + Send + 'a {
		self.db.get_shared_rooms(user_a, user_b)
	}

	pub async fn has_shared_rooms<'a>(&'a self, user_a: &'a UserId, user_b: &'a UserId) -> bool {
		let get_shared_rooms = self.get_shared_rooms(user_a, user_b);

		pin_mut!(get_shared_rooms);
		get_shared_rooms.next().await.is_some()
	}
}
