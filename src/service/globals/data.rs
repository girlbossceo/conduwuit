use std::sync::{Arc, RwLock};

use conduit::{trace, utils, Result, Server};
use database::{Database, Deserialized, Map};
use futures::{pin_mut, stream::FuturesUnordered, FutureExt, StreamExt};
use ruma::{DeviceId, UserId};

use crate::{rooms, Dep};

pub struct Data {
	global: Arc<Map>,
	todeviceid_events: Arc<Map>,
	userroomid_joined: Arc<Map>,
	userroomid_invitestate: Arc<Map>,
	userroomid_leftstate: Arc<Map>,
	userroomid_notificationcount: Arc<Map>,
	userroomid_highlightcount: Arc<Map>,
	pduid_pdu: Arc<Map>,
	keychangeid_userid: Arc<Map>,
	roomusertype_roomuserdataid: Arc<Map>,
	readreceiptid_readreceipt: Arc<Map>,
	userid_lastonetimekeyupdate: Arc<Map>,
	counter: RwLock<u64>,
	pub(super) db: Arc<Database>,
	services: Services,
}

struct Services {
	server: Arc<Server>,
	short: Dep<rooms::short::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
	typing: Dep<rooms::typing::Service>,
}

const COUNTER: &[u8] = b"c";

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			global: db["global"].clone(),
			todeviceid_events: db["todeviceid_events"].clone(),
			userroomid_joined: db["userroomid_joined"].clone(),
			userroomid_invitestate: db["userroomid_invitestate"].clone(),
			userroomid_leftstate: db["userroomid_leftstate"].clone(),
			userroomid_notificationcount: db["userroomid_notificationcount"].clone(),
			userroomid_highlightcount: db["userroomid_highlightcount"].clone(),
			pduid_pdu: db["pduid_pdu"].clone(),
			keychangeid_userid: db["keychangeid_userid"].clone(),
			roomusertype_roomuserdataid: db["roomusertype_roomuserdataid"].clone(),
			readreceiptid_readreceipt: db["readreceiptid_readreceipt"].clone(),
			userid_lastonetimekeyupdate: db["userid_lastonetimekeyupdate"].clone(),
			counter: RwLock::new(Self::stored_count(&db["global"]).expect("initialized global counter")),
			db: args.db.clone(),
			services: Services {
				server: args.server.clone(),
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
				typing: args.depend::<rooms::typing::Service>("rooms::typing"),
			},
		}
	}

	pub fn next_count(&self) -> Result<u64> {
		let _cork = self.db.cork();
		let mut lock = self.counter.write().expect("locked");
		let counter: &mut u64 = &mut lock;
		debug_assert!(
			*counter == Self::stored_count(&self.global).expect("database failure"),
			"counter mismatch"
		);

		*counter = counter
			.checked_add(1)
			.expect("counter must not overflow u64");

		self.global.insert(COUNTER, counter.to_be_bytes());

		Ok(*counter)
	}

	#[inline]
	pub fn current_count(&self) -> u64 {
		let lock = self.counter.read().expect("locked");
		let counter: &u64 = &lock;
		debug_assert!(
			*counter == Self::stored_count(&self.global).expect("database failure"),
			"counter mismatch"
		);

		*counter
	}

	fn stored_count(global: &Arc<Map>) -> Result<u64> {
		global
			.get_blocking(COUNTER)
			.as_deref()
			.map_or(Ok(0_u64), utils::u64_from_bytes)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result<()> {
		let userid_bytes = user_id.as_bytes().to_vec();
		let mut userid_prefix = userid_bytes.clone();
		userid_prefix.push(0xFF);

		let mut userdeviceid_prefix = userid_prefix.clone();
		userdeviceid_prefix.extend_from_slice(device_id.as_bytes());
		userdeviceid_prefix.push(0xFF);

		let mut futures = FuturesUnordered::new();

		// Return when *any* user changed their key
		// TODO: only send for user they share a room with
		futures.push(self.todeviceid_events.watch_prefix(&userdeviceid_prefix));

		futures.push(self.userroomid_joined.watch_prefix(&userid_prefix));
		futures.push(self.userroomid_invitestate.watch_prefix(&userid_prefix));
		futures.push(self.userroomid_leftstate.watch_prefix(&userid_prefix));
		futures.push(
			self.userroomid_notificationcount
				.watch_prefix(&userid_prefix),
		);
		futures.push(self.userroomid_highlightcount.watch_prefix(&userid_prefix));

		// Events for rooms we are in
		let rooms_joined = self.services.state_cache.rooms_joined(user_id);

		pin_mut!(rooms_joined);
		while let Some(room_id) = rooms_joined.next().await {
			let Ok(short_roomid) = self.services.short.get_shortroomid(room_id).await else {
				continue;
			};

			let roomid_bytes = room_id.as_bytes().to_vec();
			let mut roomid_prefix = roomid_bytes.clone();
			roomid_prefix.push(0xFF);

			// Key changes
			futures.push(self.keychangeid_userid.watch_prefix(&roomid_prefix));

			// Room account data
			let mut roomuser_prefix = roomid_prefix.clone();
			roomuser_prefix.extend_from_slice(&userid_prefix);

			futures.push(
				self.roomusertype_roomuserdataid
					.watch_prefix(&roomuser_prefix),
			);

			// PDUs
			let short_roomid = short_roomid.to_be_bytes().to_vec();
			futures.push(self.pduid_pdu.watch_prefix(&short_roomid));

			// EDUs
			let typing_room_id = room_id.to_owned();
			let typing_wait_for_update = async move {
				self.services.typing.wait_for_update(&typing_room_id).await;
			};

			futures.push(typing_wait_for_update.boxed());
			futures.push(self.readreceiptid_readreceipt.watch_prefix(&roomid_prefix));
		}

		let mut globaluserdata_prefix = vec![0xFF];
		globaluserdata_prefix.extend_from_slice(&userid_prefix);

		futures.push(
			self.roomusertype_roomuserdataid
				.watch_prefix(&globaluserdata_prefix),
		);

		// More key changes (used when user is not joined to any rooms)
		futures.push(self.keychangeid_userid.watch_prefix(&userid_prefix));

		// One time keys
		futures.push(self.userid_lastonetimekeyupdate.watch_prefix(&userid_bytes));

		// Server shutdown
		let server_shutdown = async move {
			while self.services.server.running() {
				self.services.server.signal.subscribe().recv().await.ok();
			}
		};

		futures.push(server_shutdown.boxed());
		if !self.services.server.running() {
			return Ok(());
		}

		// Wait until one of them finds something
		trace!(futures = futures.len(), "watch started");
		futures.next().await;
		trace!(futures = futures.len(), "watch finished");

		Ok(())
	}

	pub async fn database_version(&self) -> u64 {
		self.global
			.get(b"version")
			.await
			.deserialized()
			.unwrap_or(0)
	}

	#[inline]
	pub fn bump_database_version(&self, new_version: u64) -> Result<()> {
		self.global.raw_put(b"version", new_version);
		Ok(())
	}

	#[inline]
	pub fn backup(&self) -> Result<(), Box<dyn std::error::Error>> { self.db.db.backup() }

	#[inline]
	pub fn backup_list(&self) -> Result<String> { self.db.db.backup_list() }

	#[inline]
	pub fn file_list(&self) -> Result<String> { self.db.db.file_list() }
}
