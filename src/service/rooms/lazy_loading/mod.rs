use std::{
	collections::{HashMap, HashSet},
	fmt::Write,
	sync::{Arc, Mutex},
};

use conduit::{
	implement,
	utils::{stream::TryIgnore, ReadyExt},
	PduCount, Result,
};
use database::{Interfix, Map};
use ruma::{DeviceId, OwnedDeviceId, OwnedRoomId, OwnedUserId, RoomId, UserId};

pub struct Service {
	lazy_load_waiting: Mutex<LazyLoadWaiting>,
	db: Data,
}

struct Data {
	lazyloadedids: Arc<Map>,
}

type LazyLoadWaiting = HashMap<LazyLoadWaitingKey, LazyLoadWaitingVal>;
type LazyLoadWaitingKey = (OwnedUserId, OwnedDeviceId, OwnedRoomId, PduCount);
type LazyLoadWaitingVal = HashSet<OwnedUserId>;

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			lazy_load_waiting: LazyLoadWaiting::new().into(),
			db: Data {
				lazyloadedids: args.db["lazyloadedids"].clone(),
			},
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result<()> {
		let lazy_load_waiting = self.lazy_load_waiting.lock().expect("locked").len();
		writeln!(out, "lazy_load_waiting: {lazy_load_waiting}")?;

		Ok(())
	}

	fn clear_cache(&self) { self.lazy_load_waiting.lock().expect("locked").clear(); }

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
#[inline]
pub async fn lazy_load_was_sent_before(
	&self, user_id: &UserId, device_id: &DeviceId, room_id: &RoomId, ll_user: &UserId,
) -> bool {
	let key = (user_id, device_id, room_id, ll_user);
	self.db.lazyloadedids.qry(&key).await.is_ok()
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn lazy_load_mark_sent(
	&self, user_id: &UserId, device_id: &DeviceId, room_id: &RoomId, lazy_load: HashSet<OwnedUserId>, count: PduCount,
) {
	let key = (user_id.to_owned(), device_id.to_owned(), room_id.to_owned(), count);

	self.lazy_load_waiting
		.lock()
		.expect("locked")
		.insert(key, lazy_load);
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn lazy_load_confirm_delivery(&self, user_id: &UserId, device_id: &DeviceId, room_id: &RoomId, since: PduCount) {
	let key = (user_id.to_owned(), device_id.to_owned(), room_id.to_owned(), since);

	let Some(user_ids) = self.lazy_load_waiting.lock().expect("locked").remove(&key) else {
		return;
	};

	let mut prefix = user_id.as_bytes().to_vec();
	prefix.push(0xFF);
	prefix.extend_from_slice(device_id.as_bytes());
	prefix.push(0xFF);
	prefix.extend_from_slice(room_id.as_bytes());
	prefix.push(0xFF);

	for ll_id in &user_ids {
		let mut key = prefix.clone();
		key.extend_from_slice(ll_id.as_bytes());
		self.db.lazyloadedids.insert(&key, &[]);
	}
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn lazy_load_reset(&self, user_id: &UserId, device_id: &DeviceId, room_id: &RoomId) {
	let prefix = (user_id, device_id, room_id, Interfix);
	self.db
		.lazyloadedids
		.keys_raw_prefix(&prefix)
		.ignore_err()
		.ready_for_each(|key| self.db.lazyloadedids.remove(key))
		.await;
}
