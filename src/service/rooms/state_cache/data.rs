use std::{collections::HashSet, sync::Arc};

use conduit::{utils, Error, Result};
use database::{Database, Map};
use itertools::Itertools;
use ruma::{
	events::{AnyStrippedStateEvent, AnySyncStateEvent},
	serde::Raw,
	OwnedRoomId, OwnedServerName, OwnedUserId, RoomId, ServerName, UserId,
};

use crate::{appservice::RegistrationInfo, services, user_is_local};

type StrippedStateEventIter<'a> = Box<dyn Iterator<Item = Result<(OwnedRoomId, Vec<Raw<AnyStrippedStateEvent>>)>> + 'a>;
type AnySyncStateEventIter<'a> = Box<dyn Iterator<Item = Result<(OwnedRoomId, Vec<Raw<AnySyncStateEvent>>)>> + 'a>;

pub(super) struct Data {
	userroomid_joined: Arc<Map>,
	roomuserid_joined: Arc<Map>,
	userroomid_invitestate: Arc<Map>,
	roomuserid_invitecount: Arc<Map>,
	userroomid_leftstate: Arc<Map>,
	roomuserid_leftcount: Arc<Map>,
	roomid_inviteviaservers: Arc<Map>,
	roomuseroncejoinedids: Arc<Map>,
	roomid_joinedcount: Arc<Map>,
	roomid_invitedcount: Arc<Map>,
	roomserverids: Arc<Map>,
	serverroomids: Arc<Map>,
	db: Arc<Database>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			userroomid_joined: db["userroomid_joined"].clone(),
			roomuserid_joined: db["roomuserid_joined"].clone(),
			userroomid_invitestate: db["userroomid_invitestate"].clone(),
			roomuserid_invitecount: db["roomuserid_invitecount"].clone(),
			userroomid_leftstate: db["userroomid_leftstate"].clone(),
			roomuserid_leftcount: db["roomuserid_leftcount"].clone(),
			roomid_inviteviaservers: db["roomid_inviteviaservers"].clone(),
			roomuseroncejoinedids: db["roomuseroncejoinedids"].clone(),
			roomid_joinedcount: db["roomid_joinedcount"].clone(),
			roomid_invitedcount: db["roomid_invitedcount"].clone(),
			roomserverids: db["roomserverids"].clone(),
			serverroomids: db["serverroomids"].clone(),
			db: db.clone(),
		}
	}

	pub(super) fn mark_as_once_joined(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());
		self.roomuseroncejoinedids.insert(&userroom_id, &[])
	}

	pub(super) fn mark_as_joined(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
		let roomid = room_id.as_bytes().to_vec();

		let mut roomuser_id = roomid.clone();
		roomuser_id.push(0xFF);
		roomuser_id.extend_from_slice(user_id.as_bytes());

		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		self.userroomid_joined.insert(&userroom_id, &[])?;
		self.roomuserid_joined.insert(&roomuser_id, &[])?;
		self.userroomid_invitestate.remove(&userroom_id)?;
		self.roomuserid_invitecount.remove(&roomuser_id)?;
		self.userroomid_leftstate.remove(&userroom_id)?;
		self.roomuserid_leftcount.remove(&roomuser_id)?;

		self.roomid_inviteviaservers.remove(&roomid)?;

		Ok(())
	}

	pub(super) fn mark_as_invited(
		&self, user_id: &UserId, room_id: &RoomId, last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
		invite_via: Option<Vec<OwnedServerName>>,
	) -> Result<()> {
		let mut roomuser_id = room_id.as_bytes().to_vec();
		roomuser_id.push(0xFF);
		roomuser_id.extend_from_slice(user_id.as_bytes());

		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		self.userroomid_invitestate.insert(
			&userroom_id,
			&serde_json::to_vec(&last_state.unwrap_or_default()).expect("state to bytes always works"),
		)?;
		self.roomuserid_invitecount
			.insert(&roomuser_id, &services().globals.next_count()?.to_be_bytes())?;
		self.userroomid_joined.remove(&userroom_id)?;
		self.roomuserid_joined.remove(&roomuser_id)?;
		self.userroomid_leftstate.remove(&userroom_id)?;
		self.roomuserid_leftcount.remove(&roomuser_id)?;

		if let Some(servers) = invite_via {
			let mut prev_servers = self
				.servers_invite_via(room_id)
				.filter_map(Result::ok)
				.collect_vec();
			#[allow(clippy::redundant_clone)] // this is a necessary clone?
			prev_servers.append(servers.clone().as_mut());
			let servers = prev_servers.iter().rev().unique().rev().collect_vec();

			let servers = servers
				.iter()
				.map(|server| server.as_bytes())
				.collect_vec()
				.join(&[0xFF][..]);

			self.roomid_inviteviaservers
				.insert(room_id.as_bytes(), &servers)?;
		}

		Ok(())
	}

	pub(super) fn mark_as_left(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
		let roomid = room_id.as_bytes().to_vec();

		let mut roomuser_id = roomid.clone();
		roomuser_id.push(0xFF);
		roomuser_id.extend_from_slice(user_id.as_bytes());

		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		self.userroomid_leftstate.insert(
			&userroom_id,
			&serde_json::to_vec(&Vec::<Raw<AnySyncStateEvent>>::new()).unwrap(),
		)?; // TODO
		self.roomuserid_leftcount
			.insert(&roomuser_id, &services().globals.next_count()?.to_be_bytes())?;
		self.userroomid_joined.remove(&userroom_id)?;
		self.roomuserid_joined.remove(&roomuser_id)?;
		self.userroomid_invitestate.remove(&userroom_id)?;
		self.roomuserid_invitecount.remove(&roomuser_id)?;

		self.roomid_inviteviaservers.remove(&roomid)?;

		Ok(())
	}

	pub(super) fn update_joined_count(&self, room_id: &RoomId) -> Result<()> {
		let mut joinedcount = 0_u64;
		let mut invitedcount = 0_u64;
		let mut joined_servers = HashSet::new();

		for joined in self.room_members(room_id).filter_map(Result::ok) {
			joined_servers.insert(joined.server_name().to_owned());
			joinedcount = joinedcount.saturating_add(1);
		}

		for _invited in self.room_members_invited(room_id).filter_map(Result::ok) {
			invitedcount = invitedcount.saturating_add(1);
		}

		self.roomid_joinedcount
			.insert(room_id.as_bytes(), &joinedcount.to_be_bytes())?;

		self.roomid_invitedcount
			.insert(room_id.as_bytes(), &invitedcount.to_be_bytes())?;

		for old_joined_server in self.room_servers(room_id).filter_map(Result::ok) {
			if !joined_servers.remove(&old_joined_server) {
				// Server not in room anymore
				let mut roomserver_id = room_id.as_bytes().to_vec();
				roomserver_id.push(0xFF);
				roomserver_id.extend_from_slice(old_joined_server.as_bytes());

				let mut serverroom_id = old_joined_server.as_bytes().to_vec();
				serverroom_id.push(0xFF);
				serverroom_id.extend_from_slice(room_id.as_bytes());

				self.roomserverids.remove(&roomserver_id)?;
				self.serverroomids.remove(&serverroom_id)?;
			}
		}

		// Now only new servers are in joined_servers anymore
		for server in joined_servers {
			let mut roomserver_id = room_id.as_bytes().to_vec();
			roomserver_id.push(0xFF);
			roomserver_id.extend_from_slice(server.as_bytes());

			let mut serverroom_id = server.as_bytes().to_vec();
			serverroom_id.push(0xFF);
			serverroom_id.extend_from_slice(room_id.as_bytes());

			self.roomserverids.insert(&roomserver_id, &[])?;
			self.serverroomids.insert(&serverroom_id, &[])?;
		}

		self.db
			.appservice_in_room_cache
			.write()
			.unwrap()
			.remove(room_id);

		Ok(())
	}

	#[tracing::instrument(skip(self, room_id, appservice))]
	pub(super) fn appservice_in_room(&self, room_id: &RoomId, appservice: &RegistrationInfo) -> Result<bool> {
		let maybe = self
			.db
			.appservice_in_room_cache
			.read()
			.unwrap()
			.get(room_id)
			.and_then(|map| map.get(&appservice.registration.id))
			.copied();

		if let Some(b) = maybe {
			Ok(b)
		} else {
			let bridge_user_id = UserId::parse_with_server_name(
				appservice.registration.sender_localpart.as_str(),
				services().globals.server_name(),
			)
			.ok();

			let in_room = bridge_user_id.map_or(false, |id| self.is_joined(&id, room_id).unwrap_or(false))
				|| self
					.room_members(room_id)
					.any(|userid| userid.map_or(false, |userid| appservice.users.is_match(userid.as_str())));

			self.db
				.appservice_in_room_cache
				.write()
				.unwrap()
				.entry(room_id.to_owned())
				.or_default()
				.insert(appservice.registration.id.clone(), in_room);

			Ok(in_room)
		}
	}

	/// Makes a user forget a room.
	#[tracing::instrument(skip(self))]
	pub(super) fn forget(&self, room_id: &RoomId, user_id: &UserId) -> Result<()> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		let mut roomuser_id = room_id.as_bytes().to_vec();
		roomuser_id.push(0xFF);
		roomuser_id.extend_from_slice(user_id.as_bytes());

		self.userroomid_leftstate.remove(&userroom_id)?;
		self.roomuserid_leftcount.remove(&roomuser_id)?;

		Ok(())
	}

	/// Returns an iterator of all servers participating in this room.
	#[tracing::instrument(skip(self))]
	pub(super) fn room_servers<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedServerName>> + 'a> {
		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(self.roomserverids.scan_prefix(prefix).map(|(key, _)| {
			ServerName::parse(
				utils::string_from_bytes(
					key.rsplit(|&b| b == 0xFF)
						.next()
						.expect("rsplit always returns an element"),
				)
				.map_err(|_| Error::bad_database("Server name in roomserverids is invalid unicode."))?,
			)
			.map_err(|_| Error::bad_database("Server name in roomserverids is invalid."))
		}))
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn server_in_room(&self, server: &ServerName, room_id: &RoomId) -> Result<bool> {
		let mut key = server.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());

		self.serverroomids.get(&key).map(|o| o.is_some())
	}

	/// Returns an iterator of all rooms a server participates in (as far as we
	/// know).
	#[tracing::instrument(skip(self))]
	pub(super) fn server_rooms<'a>(
		&'a self, server: &ServerName,
	) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
		let mut prefix = server.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(self.serverroomids.scan_prefix(prefix).map(|(key, _)| {
			RoomId::parse(
				utils::string_from_bytes(
					key.rsplit(|&b| b == 0xFF)
						.next()
						.expect("rsplit always returns an element"),
				)
				.map_err(|_| Error::bad_database("RoomId in serverroomids is invalid unicode."))?,
			)
			.map_err(|_| Error::bad_database("RoomId in serverroomids is invalid."))
		}))
	}

	/// Returns an iterator of all joined members of a room.
	#[tracing::instrument(skip(self))]
	pub(super) fn room_members<'a>(&'a self, room_id: &RoomId) -> Box<dyn Iterator<Item = Result<OwnedUserId>> + 'a> {
		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(self.roomuserid_joined.scan_prefix(prefix).map(|(key, _)| {
			UserId::parse(
				utils::string_from_bytes(
					key.rsplit(|&b| b == 0xFF)
						.next()
						.expect("rsplit always returns an element"),
				)
				.map_err(|_| Error::bad_database("User ID in roomuserid_joined is invalid unicode."))?,
			)
			.map_err(|_| Error::bad_database("User ID in roomuserid_joined is invalid."))
		}))
	}

	/// Returns an iterator of all our local users in the room, even if they're
	/// deactivated/guests
	pub(super) fn local_users_in_room<'a>(&'a self, room_id: &RoomId) -> Box<dyn Iterator<Item = OwnedUserId> + 'a> {
		Box::new(
			self.room_members(room_id)
				.filter_map(Result::ok)
				.filter(|user| user_is_local(user)),
		)
	}

	/// Returns an iterator of all our local joined users in a room who are
	/// active (not deactivated, not guest)
	#[tracing::instrument(skip(self))]
	pub(super) fn active_local_users_in_room<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = OwnedUserId> + 'a> {
		Box::new(
			self.local_users_in_room(room_id)
				.filter(|user| !services().users.is_deactivated(user).unwrap_or(true)),
		)
	}

	/// Returns the number of users which are currently in a room
	#[tracing::instrument(skip(self))]
	pub(super) fn room_joined_count(&self, room_id: &RoomId) -> Result<Option<u64>> {
		self.roomid_joinedcount
			.get(room_id.as_bytes())?
			.map(|b| utils::u64_from_bytes(&b).map_err(|_| Error::bad_database("Invalid joinedcount in db.")))
			.transpose()
	}

	/// Returns the number of users which are currently invited to a room
	#[tracing::instrument(skip(self))]
	pub(super) fn room_invited_count(&self, room_id: &RoomId) -> Result<Option<u64>> {
		self.roomid_invitedcount
			.get(room_id.as_bytes())?
			.map(|b| utils::u64_from_bytes(&b).map_err(|_| Error::bad_database("Invalid joinedcount in db.")))
			.transpose()
	}

	/// Returns an iterator over all User IDs who ever joined a room.
	#[tracing::instrument(skip(self))]
	pub(super) fn room_useroncejoined<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedUserId>> + 'a> {
		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(
			self.roomuseroncejoinedids
				.scan_prefix(prefix)
				.map(|(key, _)| {
					UserId::parse(
						utils::string_from_bytes(
							key.rsplit(|&b| b == 0xFF)
								.next()
								.expect("rsplit always returns an element"),
						)
						.map_err(|_| Error::bad_database("User ID in room_useroncejoined is invalid unicode."))?,
					)
					.map_err(|_| Error::bad_database("User ID in room_useroncejoined is invalid."))
				}),
		)
	}

	/// Returns an iterator over all invited members of a room.
	#[tracing::instrument(skip(self))]
	pub(super) fn room_members_invited<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedUserId>> + 'a> {
		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(
			self.roomuserid_invitecount
				.scan_prefix(prefix)
				.map(|(key, _)| {
					UserId::parse(
						utils::string_from_bytes(
							key.rsplit(|&b| b == 0xFF)
								.next()
								.expect("rsplit always returns an element"),
						)
						.map_err(|_| Error::bad_database("User ID in roomuserid_invited is invalid unicode."))?,
					)
					.map_err(|_| Error::bad_database("User ID in roomuserid_invited is invalid."))
				}),
		)
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn get_invite_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<u64>> {
		let mut key = room_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(user_id.as_bytes());

		self.roomuserid_invitecount
			.get(&key)?
			.map_or(Ok(None), |bytes| {
				Ok(Some(
					utils::u64_from_bytes(&bytes).map_err(|_| Error::bad_database("Invalid invitecount in db."))?,
				))
			})
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn get_left_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<u64>> {
		let mut key = room_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(user_id.as_bytes());

		self.roomuserid_leftcount
			.get(&key)?
			.map(|bytes| utils::u64_from_bytes(&bytes).map_err(|_| Error::bad_database("Invalid leftcount in db.")))
			.transpose()
	}

	/// Returns an iterator over all rooms this user joined.
	#[tracing::instrument(skip(self))]
	pub(super) fn rooms_joined(&self, user_id: &UserId) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + '_> {
		Box::new(
			self.userroomid_joined
				.scan_prefix(user_id.as_bytes().to_vec())
				.map(|(key, _)| {
					RoomId::parse(
						utils::string_from_bytes(
							key.rsplit(|&b| b == 0xFF)
								.next()
								.expect("rsplit always returns an element"),
						)
						.map_err(|_| Error::bad_database("Room ID in userroomid_joined is invalid unicode."))?,
					)
					.map_err(|_| Error::bad_database("Room ID in userroomid_joined is invalid."))
				}),
		)
	}

	/// Returns an iterator over all rooms a user was invited to.
	#[tracing::instrument(skip(self))]
	pub(super) fn rooms_invited<'a>(&'a self, user_id: &UserId) -> StrippedStateEventIter<'a> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(
			self.userroomid_invitestate
				.scan_prefix(prefix)
				.map(|(key, state)| {
					let room_id = RoomId::parse(
						utils::string_from_bytes(
							key.rsplit(|&b| b == 0xFF)
								.next()
								.expect("rsplit always returns an element"),
						)
						.map_err(|_| Error::bad_database("Room ID in userroomid_invited is invalid unicode."))?,
					)
					.map_err(|_| Error::bad_database("Room ID in userroomid_invited is invalid."))?;

					let state = serde_json::from_slice(&state)
						.map_err(|_| Error::bad_database("Invalid state in userroomid_invitestate."))?;

					Ok((room_id, state))
				}),
		)
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn invite_state(
		&self, user_id: &UserId, room_id: &RoomId,
	) -> Result<Option<Vec<Raw<AnyStrippedStateEvent>>>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());

		self.userroomid_invitestate
			.get(&key)?
			.map(|state| {
				let state = serde_json::from_slice(&state)
					.map_err(|_| Error::bad_database("Invalid state in userroomid_invitestate."))?;

				Ok(state)
			})
			.transpose()
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn left_state(
		&self, user_id: &UserId, room_id: &RoomId,
	) -> Result<Option<Vec<Raw<AnyStrippedStateEvent>>>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());

		self.userroomid_leftstate
			.get(&key)?
			.map(|state| {
				let state = serde_json::from_slice(&state)
					.map_err(|_| Error::bad_database("Invalid state in userroomid_leftstate."))?;

				Ok(state)
			})
			.transpose()
	}

	/// Returns an iterator over all rooms a user left.
	#[tracing::instrument(skip(self))]
	pub(super) fn rooms_left<'a>(&'a self, user_id: &UserId) -> AnySyncStateEventIter<'a> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(
			self.userroomid_leftstate
				.scan_prefix(prefix)
				.map(|(key, state)| {
					let room_id = RoomId::parse(
						utils::string_from_bytes(
							key.rsplit(|&b| b == 0xFF)
								.next()
								.expect("rsplit always returns an element"),
						)
						.map_err(|_| Error::bad_database("Room ID in userroomid_invited is invalid unicode."))?,
					)
					.map_err(|_| Error::bad_database("Room ID in userroomid_invited is invalid."))?;

					let state = serde_json::from_slice(&state)
						.map_err(|_| Error::bad_database("Invalid state in userroomid_leftstate."))?;

					Ok((room_id, state))
				}),
		)
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn once_joined(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		Ok(self.roomuseroncejoinedids.get(&userroom_id)?.is_some())
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn is_joined(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		Ok(self.userroomid_joined.get(&userroom_id)?.is_some())
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn is_invited(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		Ok(self.userroomid_invitestate.get(&userroom_id)?.is_some())
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn is_left(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		Ok(self.userroomid_leftstate.get(&userroom_id)?.is_some())
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn servers_invite_via<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedServerName>> + 'a> {
		let key = room_id.as_bytes().to_vec();

		Box::new(
			self.roomid_inviteviaservers
				.scan_prefix(key)
				.map(|(_, servers)| {
					ServerName::parse(
						utils::string_from_bytes(
							servers
								.rsplit(|&b| b == 0xFF)
								.next()
								.expect("rsplit always returns an element"),
						)
						.map_err(|_| {
							Error::bad_database("Server name in roomid_inviteviaservers is invalid unicode.")
						})?,
					)
					.map_err(|_| Error::bad_database("Server name in roomid_inviteviaservers is invalid."))
				}),
		)
	}

	#[tracing::instrument(skip(self))]
	pub(super) fn add_servers_invite_via(&self, room_id: &RoomId, servers: &[OwnedServerName]) -> Result<()> {
		let mut prev_servers = self
			.servers_invite_via(room_id)
			.filter_map(Result::ok)
			.collect_vec();
		prev_servers.extend(servers.to_owned());
		prev_servers.sort_unstable();
		prev_servers.dedup();

		let servers = prev_servers
			.iter()
			.map(|server| server.as_bytes())
			.collect_vec()
			.join(&[0xFF][..]);

		self.roomid_inviteviaservers
			.insert(room_id.as_bytes(), &servers)?;

		Ok(())
	}
}
