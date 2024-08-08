use std::{
	collections::{BTreeMap, BTreeSet},
	sync::{Arc, Mutex, Mutex as StdMutex},
};

use conduit::Result;
use ruma::{
	api::client::sync::sync_events::{
		self,
		v4::{ExtensionsConfig, SyncRequestList},
	},
	OwnedDeviceId, OwnedRoomId, OwnedUserId,
};

pub struct Service {
	connections: DbConnections,
}

struct SlidingSyncCache {
	lists: BTreeMap<String, SyncRequestList>,
	subscriptions: BTreeMap<OwnedRoomId, sync_events::v4::RoomSubscription>,
	known_rooms: BTreeMap<String, BTreeMap<OwnedRoomId, u64>>, // For every room, the roomsince number
	extensions: ExtensionsConfig,
}

type DbConnections = Mutex<BTreeMap<DbConnectionsKey, DbConnectionsVal>>;
type DbConnectionsKey = (OwnedUserId, OwnedDeviceId, String);
type DbConnectionsVal = Arc<Mutex<SlidingSyncCache>>;

impl crate::Service for Service {
	fn build(_args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			connections: StdMutex::new(BTreeMap::new()),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub fn remembered(&self, user_id: OwnedUserId, device_id: OwnedDeviceId, conn_id: String) -> bool {
		self.connections
			.lock()
			.unwrap()
			.contains_key(&(user_id, device_id, conn_id))
	}

	pub fn forget_sync_request_connection(&self, user_id: OwnedUserId, device_id: OwnedDeviceId, conn_id: String) {
		self.connections
			.lock()
			.expect("locked")
			.remove(&(user_id, device_id, conn_id));
	}

	pub fn update_sync_request_with_cache(
		&self, user_id: OwnedUserId, device_id: OwnedDeviceId, request: &mut sync_events::v4::Request,
	) -> BTreeMap<String, BTreeMap<OwnedRoomId, u64>> {
		let Some(conn_id) = request.conn_id.clone() else {
			return BTreeMap::new();
		};

		let mut cache = self.connections.lock().expect("locked");
		let cached = Arc::clone(
			cache
				.entry((user_id, device_id, conn_id))
				.or_insert_with(|| {
					Arc::new(Mutex::new(SlidingSyncCache {
						lists: BTreeMap::new(),
						subscriptions: BTreeMap::new(),
						known_rooms: BTreeMap::new(),
						extensions: ExtensionsConfig::default(),
					}))
				}),
		);
		let cached = &mut cached.lock().expect("locked");
		drop(cache);

		for (list_id, list) in &mut request.lists {
			if let Some(cached_list) = cached.lists.get(list_id) {
				if list.sort.is_empty() {
					list.sort.clone_from(&cached_list.sort);
				};
				if list.room_details.required_state.is_empty() {
					list.room_details
						.required_state
						.clone_from(&cached_list.room_details.required_state);
				};
				list.room_details.timeline_limit = list
					.room_details
					.timeline_limit
					.or(cached_list.room_details.timeline_limit);
				list.include_old_rooms = list
					.include_old_rooms
					.clone()
					.or_else(|| cached_list.include_old_rooms.clone());
				match (&mut list.filters, cached_list.filters.clone()) {
					(Some(list_filters), Some(cached_filters)) => {
						list_filters.is_dm = list_filters.is_dm.or(cached_filters.is_dm);
						if list_filters.spaces.is_empty() {
							list_filters.spaces = cached_filters.spaces;
						}
						list_filters.is_encrypted = list_filters.is_encrypted.or(cached_filters.is_encrypted);
						list_filters.is_invite = list_filters.is_invite.or(cached_filters.is_invite);
						if list_filters.room_types.is_empty() {
							list_filters.room_types = cached_filters.room_types;
						}
						if list_filters.not_room_types.is_empty() {
							list_filters.not_room_types = cached_filters.not_room_types;
						}
						list_filters.room_name_like = list_filters
							.room_name_like
							.clone()
							.or(cached_filters.room_name_like);
						if list_filters.tags.is_empty() {
							list_filters.tags = cached_filters.tags;
						}
						if list_filters.not_tags.is_empty() {
							list_filters.not_tags = cached_filters.not_tags;
						}
					},
					(_, Some(cached_filters)) => list.filters = Some(cached_filters),
					(Some(list_filters), _) => list.filters = Some(list_filters.clone()),
					(..) => {},
				}
				if list.bump_event_types.is_empty() {
					list.bump_event_types
						.clone_from(&cached_list.bump_event_types);
				};
			}
			cached.lists.insert(list_id.clone(), list.clone());
		}

		cached
			.subscriptions
			.extend(request.room_subscriptions.clone());
		request
			.room_subscriptions
			.extend(cached.subscriptions.clone());

		request.extensions.e2ee.enabled = request
			.extensions
			.e2ee
			.enabled
			.or(cached.extensions.e2ee.enabled);

		request.extensions.to_device.enabled = request
			.extensions
			.to_device
			.enabled
			.or(cached.extensions.to_device.enabled);

		request.extensions.account_data.enabled = request
			.extensions
			.account_data
			.enabled
			.or(cached.extensions.account_data.enabled);
		request.extensions.account_data.lists = request
			.extensions
			.account_data
			.lists
			.clone()
			.or_else(|| cached.extensions.account_data.lists.clone());
		request.extensions.account_data.rooms = request
			.extensions
			.account_data
			.rooms
			.clone()
			.or_else(|| cached.extensions.account_data.rooms.clone());

		cached.extensions = request.extensions.clone();

		cached.known_rooms.clone()
	}

	pub fn update_sync_subscriptions(
		&self, user_id: OwnedUserId, device_id: OwnedDeviceId, conn_id: String,
		subscriptions: BTreeMap<OwnedRoomId, sync_events::v4::RoomSubscription>,
	) {
		let mut cache = self.connections.lock().expect("locked");
		let cached = Arc::clone(
			cache
				.entry((user_id, device_id, conn_id))
				.or_insert_with(|| {
					Arc::new(Mutex::new(SlidingSyncCache {
						lists: BTreeMap::new(),
						subscriptions: BTreeMap::new(),
						known_rooms: BTreeMap::new(),
						extensions: ExtensionsConfig::default(),
					}))
				}),
		);
		let cached = &mut cached.lock().expect("locked");
		drop(cache);

		cached.subscriptions = subscriptions;
	}

	pub fn update_sync_known_rooms(
		&self, user_id: OwnedUserId, device_id: OwnedDeviceId, conn_id: String, list_id: String,
		new_cached_rooms: BTreeSet<OwnedRoomId>, globalsince: u64,
	) {
		let mut cache = self.connections.lock().expect("locked");
		let cached = Arc::clone(
			cache
				.entry((user_id, device_id, conn_id))
				.or_insert_with(|| {
					Arc::new(Mutex::new(SlidingSyncCache {
						lists: BTreeMap::new(),
						subscriptions: BTreeMap::new(),
						known_rooms: BTreeMap::new(),
						extensions: ExtensionsConfig::default(),
					}))
				}),
		);
		let cached = &mut cached.lock().expect("locked");
		drop(cache);

		for (roomid, lastsince) in cached
			.known_rooms
			.entry(list_id.clone())
			.or_default()
			.iter_mut()
		{
			if !new_cached_rooms.contains(roomid) {
				*lastsince = 0;
			}
		}
		let list = cached.known_rooms.entry(list_id).or_default();
		for roomid in new_cached_rooms {
			list.insert(roomid, globalsince);
		}
	}
}
