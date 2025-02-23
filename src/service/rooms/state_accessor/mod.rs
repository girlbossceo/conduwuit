mod room_state;
mod server_can;
mod state;
mod user_can;

use std::{
	fmt::Write,
	sync::{Arc, Mutex as StdMutex, Mutex},
};

use conduwuit::{
	Result, err, utils,
	utils::math::{Expected, usize_from_f64},
};
use database::Map;
use lru_cache::LruCache;
use ruma::{
	EventEncryptionAlgorithm, JsOption, OwnedRoomAliasId, OwnedRoomId, OwnedServerName,
	OwnedUserId, RoomId, UserId,
	events::{
		StateEventType,
		room::{
			avatar::RoomAvatarEventContent,
			canonical_alias::RoomCanonicalAliasEventContent,
			create::RoomCreateEventContent,
			encryption::RoomEncryptionEventContent,
			guest_access::{GuestAccess, RoomGuestAccessEventContent},
			history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
			join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent, RoomMembership},
			member::RoomMemberEventContent,
			name::RoomNameEventContent,
			topic::RoomTopicEventContent,
		},
	},
	room::RoomType,
	space::SpaceRoomJoinRule,
};

use crate::{Dep, rooms, rooms::short::ShortStateHash};

pub struct Service {
	pub server_visibility_cache: Mutex<LruCache<(OwnedServerName, ShortStateHash), bool>>,
	pub user_visibility_cache: Mutex<LruCache<(OwnedUserId, ShortStateHash), bool>>,
	services: Services,
	db: Data,
}

struct Services {
	short: Dep<rooms::short::Service>,
	state: Dep<rooms::state::Service>,
	state_compressor: Dep<rooms::state_compressor::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

struct Data {
	shorteventid_shortstatehash: Arc<Map>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let config = &args.server.config;
		let server_visibility_cache_capacity =
			f64::from(config.server_visibility_cache_capacity) * config.cache_capacity_modifier;
		let user_visibility_cache_capacity =
			f64::from(config.user_visibility_cache_capacity) * config.cache_capacity_modifier;

		Ok(Arc::new(Self {
			server_visibility_cache: StdMutex::new(LruCache::new(usize_from_f64(
				server_visibility_cache_capacity,
			)?)),
			user_visibility_cache: StdMutex::new(LruCache::new(usize_from_f64(
				user_visibility_cache_capacity,
			)?)),
			services: Services {
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state: args.depend::<rooms::state::Service>("rooms::state"),
				state_compressor: args
					.depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
			},
			db: Data {
				shorteventid_shortstatehash: args.db["shorteventid_shortstatehash"].clone(),
			},
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result {
		use utils::bytes::pretty;

		let (svc_count, svc_bytes) = self.server_visibility_cache.lock()?.iter().fold(
			(0_usize, 0_usize),
			|(count, bytes), (key, _)| {
				(
					count.expected_add(1),
					bytes
						.expected_add(key.0.capacity())
						.expected_add(size_of_val(&key.1)),
				)
			},
		);

		let (uvc_count, uvc_bytes) = self.user_visibility_cache.lock()?.iter().fold(
			(0_usize, 0_usize),
			|(count, bytes), (key, _)| {
				(
					count.expected_add(1),
					bytes
						.expected_add(key.0.capacity())
						.expected_add(size_of_val(&key.1)),
				)
			},
		);

		writeln!(out, "server_visibility_cache: {svc_count} ({})", pretty(svc_bytes))?;
		writeln!(out, "user_visibility_cache: {uvc_count} ({})", pretty(uvc_bytes))?;

		Ok(())
	}

	fn clear_cache(&self) {
		self.server_visibility_cache.lock().expect("locked").clear();
		self.user_visibility_cache.lock().expect("locked").clear();
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub async fn get_name(&self, room_id: &RoomId) -> Result<String> {
		self.room_state_get_content(room_id, &StateEventType::RoomName, "")
			.await
			.map(|c: RoomNameEventContent| c.name)
	}

	pub async fn get_avatar(&self, room_id: &RoomId) -> JsOption<RoomAvatarEventContent> {
		let content = self
			.room_state_get_content(room_id, &StateEventType::RoomAvatar, "")
			.await
			.ok();

		JsOption::from_option(content)
	}

	pub async fn get_member(
		&self,
		room_id: &RoomId,
		user_id: &UserId,
	) -> Result<RoomMemberEventContent> {
		self.room_state_get_content(room_id, &StateEventType::RoomMember, user_id.as_str())
			.await
	}

	/// Checks if guests are able to view room content without joining
	pub async fn is_world_readable(&self, room_id: &RoomId) -> bool {
		self.room_state_get_content(room_id, &StateEventType::RoomHistoryVisibility, "")
			.await
			.map(|c: RoomHistoryVisibilityEventContent| {
				c.history_visibility == HistoryVisibility::WorldReadable
			})
			.unwrap_or(false)
	}

	/// Checks if guests are able to join a given room
	pub async fn guest_can_join(&self, room_id: &RoomId) -> bool {
		self.room_state_get_content(room_id, &StateEventType::RoomGuestAccess, "")
			.await
			.map(|c: RoomGuestAccessEventContent| c.guest_access == GuestAccess::CanJoin)
			.unwrap_or(false)
	}

	/// Gets the primary alias from canonical alias event
	pub async fn get_canonical_alias(&self, room_id: &RoomId) -> Result<OwnedRoomAliasId> {
		self.room_state_get_content(room_id, &StateEventType::RoomCanonicalAlias, "")
			.await
			.and_then(|c: RoomCanonicalAliasEventContent| {
				c.alias
					.ok_or_else(|| err!(Request(NotFound("No alias found in event content."))))
			})
	}

	/// Gets the room topic
	pub async fn get_room_topic(&self, room_id: &RoomId) -> Result<String> {
		self.room_state_get_content(room_id, &StateEventType::RoomTopic, "")
			.await
			.map(|c: RoomTopicEventContent| c.topic)
	}

	/// Returns the join rule (`SpaceRoomJoinRule`) for a given room
	pub async fn get_join_rule(
		&self,
		room_id: &RoomId,
	) -> Result<(SpaceRoomJoinRule, Vec<OwnedRoomId>)> {
		self.room_state_get_content(room_id, &StateEventType::RoomJoinRules, "")
			.await
			.map(|c: RoomJoinRulesEventContent| {
				(c.join_rule.clone().into(), self.allowed_room_ids(c.join_rule))
			})
			.or_else(|_| Ok((SpaceRoomJoinRule::Invite, vec![])))
	}

	/// Returns an empty vec if not a restricted room
	pub fn allowed_room_ids(&self, join_rule: JoinRule) -> Vec<OwnedRoomId> {
		let mut room_ids = Vec::with_capacity(1);
		if let JoinRule::Restricted(r) | JoinRule::KnockRestricted(r) = join_rule {
			for rule in r.allow {
				if let AllowRule::RoomMembership(RoomMembership { room_id: membership }) = rule {
					room_ids.push(membership.clone());
				}
			}
		}
		room_ids
	}

	pub async fn get_room_type(&self, room_id: &RoomId) -> Result<RoomType> {
		self.room_state_get_content(room_id, &StateEventType::RoomCreate, "")
			.await
			.and_then(|content: RoomCreateEventContent| {
				content
					.room_type
					.ok_or_else(|| err!(Request(NotFound("No type found in event content"))))
			})
	}

	/// Gets the room's encryption algorithm if `m.room.encryption` state event
	/// is found
	pub async fn get_room_encryption(
		&self,
		room_id: &RoomId,
	) -> Result<EventEncryptionAlgorithm> {
		self.room_state_get_content(room_id, &StateEventType::RoomEncryption, "")
			.await
			.map(|content: RoomEncryptionEventContent| content.algorithm)
	}

	pub async fn is_encrypted_room(&self, room_id: &RoomId) -> bool {
		self.room_state_get(room_id, &StateEventType::RoomEncryption, "")
			.await
			.is_ok()
	}
}
