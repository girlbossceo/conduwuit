mod data;

use std::{collections::HashSet, sync::Arc};

use conduit::{
	err,
	utils::{stream::TryIgnore, ReadyExt, StreamTools},
	warn, Result,
};
use data::Data;
use database::{Deserialized, Ignore, Interfix};
use futures::{Stream, StreamExt};
use itertools::Itertools;
use ruma::{
	events::{
		direct::DirectEvent,
		ignored_user_list::IgnoredUserListEvent,
		room::{
			create::RoomCreateEventContent,
			member::{MembershipState, RoomMemberEventContent},
			power_levels::RoomPowerLevelsEventContent,
		},
		AnyStrippedStateEvent, AnySyncStateEvent, GlobalAccountDataEventType, RoomAccountDataEventType, StateEventType,
	},
	int,
	serde::Raw,
	OwnedRoomId, OwnedServerName, RoomId, ServerName, UserId,
};

use crate::{account_data, appservice::RegistrationInfo, globals, rooms, users, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	account_data: Dep<account_data::Service>,
	globals: Dep<globals::Service>,
	state_accessor: Dep<rooms::state_accessor::Service>,
	users: Dep<users::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				account_data: args.depend::<account_data::Service>("account_data"),
				globals: args.depend::<globals::Service>("globals"),
				state_accessor: args.depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
				users: args.depend::<users::Service>("users"),
			},
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Update current membership data.
	#[tracing::instrument(skip(self, last_state))]
	#[allow(clippy::too_many_arguments)]
	pub async fn update_membership(
		&self, room_id: &RoomId, user_id: &UserId, membership_event: RoomMemberEventContent, sender: &UserId,
		last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>, invite_via: Option<Vec<OwnedServerName>>,
		update_joined_count: bool,
	) -> Result<()> {
		let membership = membership_event.membership;

		// Keep track what remote users exist by adding them as "deactivated" users
		//
		// TODO: use futures to update remote profiles without blocking the membership
		// update
		#[allow(clippy::collapsible_if)]
		if !self.services.globals.user_is_local(user_id) {
			if !self.services.users.exists(user_id).await {
				self.services.users.create(user_id, None)?;
			}

			/*
			// Try to update our local copy of the user if ours does not match
			if ((self.services.users.displayname(user_id)? != membership_event.displayname)
				|| (self.services.users.avatar_url(user_id)? != membership_event.avatar_url)
				|| (self.services.users.blurhash(user_id)? != membership_event.blurhash))
				&& (membership != MembershipState::Leave)
			{
				let response = self.services
					.sending
					.send_federation_request(
						user_id.server_name(),
						federation::query::get_profile_information::v1::Request {
							user_id: user_id.into(),
							field: None, // we want the full user's profile to update locally too
						},
					)
					.await;

				self.services.users.set_displayname(user_id, response.displayname.clone()).await?;
				self.services.users.set_avatar_url(user_id, response.avatar_url).await?;
				self.services.users.set_blurhash(user_id, response.blurhash).await?;
			};
			*/
		}

		match &membership {
			MembershipState::Join => {
				// Check if the user never joined this room
				if !self.once_joined(user_id, room_id).await {
					// Add the user ID to the join list then
					self.db.mark_as_once_joined(user_id, room_id);

					// Check if the room has a predecessor
					if let Ok(Some(predecessor)) = self
						.services
						.state_accessor
						.room_state_get_content(room_id, &StateEventType::RoomCreate, "")
						.await
						.map(|content: RoomCreateEventContent| content.predecessor)
					{
						// Copy user settings from predecessor to the current room:
						// - Push rules
						//
						// TODO: finish this once push rules are implemented.
						//
						// let mut push_rules_event_content: PushRulesEvent = account_data
						//     .get(
						//         None,
						//         user_id,
						//         EventType::PushRules,
						//     )?;
						//
						// NOTE: find where `predecessor.room_id` match
						//       and update to `room_id`.
						//
						// account_data
						//     .update(
						//         None,
						//         user_id,
						//         EventType::PushRules,
						//         &push_rules_event_content,
						//         globals,
						//     )
						//     .ok();

						// Copy old tags to new room
						if let Ok(tag_event) = self
							.services
							.account_data
							.get(Some(&predecessor.room_id), user_id, RoomAccountDataEventType::Tag)
							.await
							.and_then(|event| {
								serde_json::from_str(event.get())
									.map_err(|e| err!(Database(warn!("Invalid account data event in db: {e:?}"))))
							}) {
							self.services
								.account_data
								.update(Some(room_id), user_id, RoomAccountDataEventType::Tag, &tag_event)
								.await
								.ok();
						};

						// Copy direct chat flag
						if let Ok(mut direct_event) = self
							.services
							.account_data
							.get(None, user_id, GlobalAccountDataEventType::Direct.to_string().into())
							.await
							.and_then(|event| {
								serde_json::from_str::<DirectEvent>(event.get())
									.map_err(|e| err!(Database(warn!("Invalid account data event in db: {e:?}"))))
							}) {
							let mut room_ids_updated = false;
							for room_ids in direct_event.content.0.values_mut() {
								if room_ids.iter().any(|r| r == &predecessor.room_id) {
									room_ids.push(room_id.to_owned());
									room_ids_updated = true;
								}
							}

							if room_ids_updated {
								self.services
									.account_data
									.update(
										None,
										user_id,
										GlobalAccountDataEventType::Direct.to_string().into(),
										&serde_json::to_value(&direct_event).expect("to json always works"),
									)
									.await?;
							}
						};
					}
				}

				self.db.mark_as_joined(user_id, room_id);
			},
			MembershipState::Invite => {
				// We want to know if the sender is ignored by the receiver
				let is_ignored = self
					.services
					.account_data
					.get(
						None,    // Ignored users are in global account data
						user_id, // Receiver
						GlobalAccountDataEventType::IgnoredUserList
							.to_string()
							.into(),
					)
					.await
					.and_then(|event| {
						serde_json::from_str::<IgnoredUserListEvent>(event.get())
							.map_err(|e| err!(Database(warn!("Invalid account data event in db: {e:?}"))))
					})
					.map_or(false, |ignored| {
						ignored
							.content
							.ignored_users
							.iter()
							.any(|(user, _details)| user == sender)
					});

				if is_ignored {
					return Ok(());
				}

				self.mark_as_invited(user_id, room_id, last_state, invite_via)
					.await;
			},
			MembershipState::Leave | MembershipState::Ban => {
				self.db.mark_as_left(user_id, room_id);
			},
			_ => {},
		}

		if update_joined_count {
			self.update_joined_count(room_id).await;
		}

		Ok(())
	}

	#[tracing::instrument(skip(self, room_id, appservice), level = "debug")]
	pub async fn appservice_in_room(&self, room_id: &RoomId, appservice: &RegistrationInfo) -> bool {
		let maybe = self
			.db
			.appservice_in_room_cache
			.read()
			.unwrap()
			.get(room_id)
			.and_then(|map| map.get(&appservice.registration.id))
			.copied();

		if let Some(b) = maybe {
			b
		} else {
			let bridge_user_id = UserId::parse_with_server_name(
				appservice.registration.sender_localpart.as_str(),
				self.services.globals.server_name(),
			)
			.ok();

			let in_room = if let Some(id) = &bridge_user_id {
				self.is_joined(id, room_id).await
			} else {
				false
			};

			let in_room = in_room
				|| self
					.room_members(room_id)
					.ready_any(|userid| appservice.users.is_match(userid.as_str()))
					.await;

			self.db
				.appservice_in_room_cache
				.write()
				.unwrap()
				.entry(room_id.to_owned())
				.or_default()
				.insert(appservice.registration.id.clone(), in_room);

			in_room
		}
	}

	/// Direct DB function to directly mark a user as left. It is not
	/// recommended to use this directly. You most likely should use
	/// `update_membership` instead
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn mark_as_left(&self, user_id: &UserId, room_id: &RoomId) { self.db.mark_as_left(user_id, room_id); }

	/// Direct DB function to directly mark a user as joined. It is not
	/// recommended to use this directly. You most likely should use
	/// `update_membership` instead
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn mark_as_joined(&self, user_id: &UserId, room_id: &RoomId) { self.db.mark_as_joined(user_id, room_id); }

	/// Makes a user forget a room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn forget(&self, room_id: &RoomId, user_id: &UserId) { self.db.forget(room_id, user_id); }

	/// Returns an iterator of all servers participating in this room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn room_servers<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &ServerName> + Send + 'a {
		let prefix = (room_id, Interfix);
		self.db
			.roomserverids
			.keys_prefix(&prefix)
			.ignore_err()
			.map(|(_, server): (Ignore, &ServerName)| server)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn server_in_room<'a>(&'a self, server: &'a ServerName, room_id: &'a RoomId) -> bool {
		let key = (server, room_id);
		self.db.serverroomids.qry(&key).await.is_ok()
	}

	/// Returns an iterator of all rooms a server participates in (as far as we
	/// know).
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn server_rooms<'a>(&'a self, server: &'a ServerName) -> impl Stream<Item = &RoomId> + Send + 'a {
		let prefix = (server, Interfix);
		self.db
			.serverroomids
			.keys_prefix(&prefix)
			.ignore_err()
			.map(|(_, room_id): (Ignore, &RoomId)| room_id)
	}

	/// Returns true if server can see user by sharing at least one room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn server_sees_user(&self, server: &ServerName, user_id: &UserId) -> bool {
		self.server_rooms(server)
			.any(|room_id| self.is_joined(user_id, room_id))
			.await
	}

	/// Returns true if user_a and user_b share at least one room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn user_sees_user(&self, user_a: &UserId, user_b: &UserId) -> bool {
		// Minimize number of point-queries by iterating user with least nr rooms
		let (a, b) = if self.rooms_joined(user_a).count().await < self.rooms_joined(user_b).count().await {
			(user_a, user_b)
		} else {
			(user_b, user_a)
		};

		self.rooms_joined(a)
			.any(|room_id| self.is_joined(b, room_id))
			.await
	}

	/// Returns an iterator of all joined members of a room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn room_members<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &UserId> + Send + 'a {
		let prefix = (room_id, Interfix);
		self.db
			.roomuserid_joined
			.keys_prefix(&prefix)
			.ignore_err()
			.map(|(_, user_id): (Ignore, &UserId)| user_id)
	}

	/// Returns the number of users which are currently in a room
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn room_joined_count(&self, room_id: &RoomId) -> Result<u64> {
		self.db.roomid_joinedcount.qry(room_id).await.deserialized()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	/// Returns an iterator of all our local users in the room, even if they're
	/// deactivated/guests
	pub fn local_users_in_room<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &UserId> + Send + 'a {
		self.room_members(room_id)
			.ready_filter(|user| self.services.globals.user_is_local(user))
	}

	#[tracing::instrument(skip(self), level = "debug")]
	/// Returns an iterator of all our local joined users in a room who are
	/// active (not deactivated, not guest)
	pub fn active_local_users_in_room<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &UserId> + Send + 'a {
		self.local_users_in_room(room_id)
			.filter(|user| self.services.users.is_active(user))
	}

	/// Returns the number of users which are currently invited to a room
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn room_invited_count(&self, room_id: &RoomId) -> Result<u64> {
		self.db
			.roomid_invitedcount
			.qry(room_id)
			.await
			.deserialized()
	}

	/// Returns an iterator over all User IDs who ever joined a room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn room_useroncejoined<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &UserId> + Send + 'a {
		let prefix = (room_id, Interfix);
		self.db
			.roomuseroncejoinedids
			.keys_prefix(&prefix)
			.ignore_err()
			.map(|(_, user_id): (Ignore, &UserId)| user_id)
	}

	/// Returns an iterator over all invited members of a room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn room_members_invited<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &UserId> + Send + 'a {
		let prefix = (room_id, Interfix);
		self.db
			.roomuserid_invitecount
			.keys_prefix(&prefix)
			.ignore_err()
			.map(|(_, user_id): (Ignore, &UserId)| user_id)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn get_invite_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
		let key = (room_id, user_id);
		self.db
			.roomuserid_invitecount
			.qry(&key)
			.await
			.deserialized()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn get_left_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
		let key = (room_id, user_id);
		self.db.roomuserid_leftcount.qry(&key).await.deserialized()
	}

	/// Returns an iterator over all rooms this user joined.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn rooms_joined<'a>(&'a self, user_id: &'a UserId) -> impl Stream<Item = &RoomId> + Send + 'a {
		self.db
			.userroomid_joined
			.keys_prefix_raw(user_id)
			.ignore_err()
			.map(|(_, room_id): (Ignore, &RoomId)| room_id)
	}

	/// Returns an iterator over all rooms a user was invited to.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn rooms_invited<'a>(
		&'a self, user_id: &'a UserId,
	) -> impl Stream<Item = (OwnedRoomId, Vec<Raw<AnyStrippedStateEvent>>)> + Send + 'a {
		self.db.rooms_invited(user_id)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn invite_state(&self, user_id: &UserId, room_id: &RoomId) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
		self.db.invite_state(user_id, room_id).await
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn left_state(&self, user_id: &UserId, room_id: &RoomId) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
		self.db.left_state(user_id, room_id).await
	}

	/// Returns an iterator over all rooms a user left.
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn rooms_left<'a>(
		&'a self, user_id: &'a UserId,
	) -> impl Stream<Item = (OwnedRoomId, Vec<Raw<AnySyncStateEvent>>)> + Send + 'a {
		self.db.rooms_left(user_id)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn once_joined(&self, user_id: &UserId, room_id: &RoomId) -> bool {
		let key = (user_id, room_id);
		self.db.roomuseroncejoinedids.qry(&key).await.is_ok()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn is_joined<'a>(&'a self, user_id: &'a UserId, room_id: &'a RoomId) -> bool {
		let key = (user_id, room_id);
		self.db.userroomid_joined.qry(&key).await.is_ok()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn is_invited(&self, user_id: &UserId, room_id: &RoomId) -> bool {
		let key = (user_id, room_id);
		self.db.userroomid_invitestate.qry(&key).await.is_ok()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn is_left(&self, user_id: &UserId, room_id: &RoomId) -> bool {
		let key = (user_id, room_id);
		self.db.userroomid_leftstate.qry(&key).await.is_ok()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn servers_invite_via<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &ServerName> + Send + 'a {
		type KeyVal<'a> = (Ignore, Vec<&'a ServerName>);

		self.db
			.roomid_inviteviaservers
			.stream_prefix_raw(room_id)
			.ignore_err()
			.map(|(_, servers): KeyVal<'_>| *servers.last().expect("at least one server"))
	}

	/// Gets up to three servers that are likely to be in the room in the
	/// distant future.
	///
	/// See <https://spec.matrix.org/v1.10/appendices/#routing>
	#[tracing::instrument(skip(self))]
	pub async fn servers_route_via(&self, room_id: &RoomId) -> Result<Vec<OwnedServerName>> {
		let most_powerful_user_server = self
			.services
			.state_accessor
			.room_state_get_content(room_id, &StateEventType::RoomPowerLevels, "")
			.await
			.map(|content: RoomPowerLevelsEventContent| {
				content
					.users
					.iter()
					.max_by_key(|(_, power)| *power)
					.and_then(|x| (x.1 >= &int!(50)).then_some(x))
					.map(|(user, _power)| user.server_name().to_owned())
			})
			.map_err(|e| err!(Database(error!(?e, "Invalid power levels event content in database."))))?;

		let mut servers: Vec<OwnedServerName> = self
			.room_members(room_id)
			.counts_by(|user| user.server_name().to_owned())
			.await
			.into_iter()
			.sorted_by_key(|(_, users)| *users)
			.map(|(server, _)| server)
			.rev()
			.take(3)
			.collect();

		if let Some(server) = most_powerful_user_server {
			servers.insert(0, server);
			servers.truncate(3);
		}

		Ok(servers)
	}

	pub fn get_appservice_in_room_cache_usage(&self) -> (usize, usize) {
		let cache = self.db.appservice_in_room_cache.read().expect("locked");
		(cache.len(), cache.capacity())
	}

	pub fn clear_appservice_in_room_cache(&self) {
		self.db
			.appservice_in_room_cache
			.write()
			.expect("locked")
			.clear();
	}

	pub async fn update_joined_count(&self, room_id: &RoomId) {
		let mut joinedcount = 0_u64;
		let mut invitedcount = 0_u64;
		let mut joined_servers = HashSet::new();

		self.room_members(room_id)
			.ready_for_each(|joined| {
				joined_servers.insert(joined.server_name().to_owned());
				joinedcount = joinedcount.saturating_add(1);
			})
			.await;

		invitedcount = invitedcount.saturating_add(
			self.room_members_invited(room_id)
				.count()
				.await
				.try_into()
				.unwrap_or(0),
		);

		self.db
			.roomid_joinedcount
			.insert(room_id.as_bytes(), &joinedcount.to_be_bytes());

		self.db
			.roomid_invitedcount
			.insert(room_id.as_bytes(), &invitedcount.to_be_bytes());

		self.room_servers(room_id)
			.ready_for_each(|old_joined_server| {
				if !joined_servers.remove(old_joined_server) {
					// Server not in room anymore
					let mut roomserver_id = room_id.as_bytes().to_vec();
					roomserver_id.push(0xFF);
					roomserver_id.extend_from_slice(old_joined_server.as_bytes());

					let mut serverroom_id = old_joined_server.as_bytes().to_vec();
					serverroom_id.push(0xFF);
					serverroom_id.extend_from_slice(room_id.as_bytes());

					self.db.roomserverids.remove(&roomserver_id);
					self.db.serverroomids.remove(&serverroom_id);
				}
			})
			.await;

		// Now only new servers are in joined_servers anymore
		for server in joined_servers {
			let mut roomserver_id = room_id.as_bytes().to_vec();
			roomserver_id.push(0xFF);
			roomserver_id.extend_from_slice(server.as_bytes());

			let mut serverroom_id = server.as_bytes().to_vec();
			serverroom_id.push(0xFF);
			serverroom_id.extend_from_slice(room_id.as_bytes());

			self.db.roomserverids.insert(&roomserver_id, &[]);
			self.db.serverroomids.insert(&serverroom_id, &[]);
		}

		self.db
			.appservice_in_room_cache
			.write()
			.unwrap()
			.remove(room_id);
	}

	pub async fn mark_as_invited(
		&self, user_id: &UserId, room_id: &RoomId, last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
		invite_via: Option<Vec<OwnedServerName>>,
	) {
		let mut roomuser_id = room_id.as_bytes().to_vec();
		roomuser_id.push(0xFF);
		roomuser_id.extend_from_slice(user_id.as_bytes());

		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		self.db.userroomid_invitestate.insert(
			&userroom_id,
			&serde_json::to_vec(&last_state.unwrap_or_default()).expect("state to bytes always works"),
		);
		self.db
			.roomuserid_invitecount
			.insert(&roomuser_id, &self.services.globals.next_count().unwrap().to_be_bytes());
		self.db.userroomid_joined.remove(&userroom_id);
		self.db.roomuserid_joined.remove(&roomuser_id);
		self.db.userroomid_leftstate.remove(&userroom_id);
		self.db.roomuserid_leftcount.remove(&roomuser_id);

		if let Some(servers) = invite_via {
			let mut prev_servers = self
				.servers_invite_via(room_id)
				.map(ToOwned::to_owned)
				.collect::<Vec<_>>()
				.await;
			#[allow(clippy::redundant_clone)] // this is a necessary clone?
			prev_servers.append(servers.clone().as_mut());
			let servers = prev_servers.iter().rev().unique().rev().collect_vec();

			let servers = servers
				.iter()
				.map(|server| server.as_bytes())
				.collect_vec()
				.join(&[0xFF][..]);

			self.db
				.roomid_inviteviaservers
				.insert(room_id.as_bytes(), &servers);
		}
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn add_servers_invite_via(&self, room_id: &RoomId, servers: &[OwnedServerName]) {
		let mut prev_servers = self
			.servers_invite_via(room_id)
			.map(ToOwned::to_owned)
			.collect::<Vec<_>>()
			.await;
		prev_servers.extend(servers.to_owned());
		prev_servers.sort_unstable();
		prev_servers.dedup();

		let servers = prev_servers
			.iter()
			.map(|server| server.as_bytes())
			.collect_vec()
			.join(&[0xFF][..]);

		self.db
			.roomid_inviteviaservers
			.insert(room_id.as_bytes(), &servers);
	}
}
