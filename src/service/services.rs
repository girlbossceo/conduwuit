use std::{
	collections::{BTreeMap, HashMap},
	sync::{atomic, Arc, Mutex as StdMutex},
};

use conduit::{debug_info, Result, Server};
use database::KeyValueDatabase;
use lru_cache::LruCache;
use tokio::{
	fs,
	sync::{broadcast, Mutex, RwLock},
};
use tracing::{debug, info, trace};

use crate::{
	account_data, admin, appservice, globals, key_backups, media, presence, pusher, rooms, sending, transaction_ids,
	uiaa, users,
};

pub struct Services {
	pub appservice: appservice::Service,
	pub pusher: pusher::Service,
	pub rooms: rooms::Service,
	pub transaction_ids: transaction_ids::Service,
	pub uiaa: uiaa::Service,
	pub users: users::Service,
	pub account_data: account_data::Service,
	pub presence: Arc<presence::Service>,
	pub admin: Arc<admin::Service>,
	pub globals: globals::Service,
	pub key_backups: key_backups::Service,
	pub media: media::Service,
	pub sending: Arc<sending::Service>,
	pub server: Arc<Server>,
	pub db: Arc<KeyValueDatabase>,
}

impl Services {
	pub async fn build(server: Arc<Server>, db: Arc<KeyValueDatabase>) -> Result<Self> {
		let config = &server.config;
		Ok(Self {
			appservice: appservice::Service::build(db.clone())?,
			pusher: pusher::Service {
				db: db.clone(),
			},
			rooms: rooms::Service {
				alias: rooms::alias::Service {
					db: db.clone(),
				},
				auth_chain: rooms::auth_chain::Service {
					db: db.clone(),
				},
				directory: rooms::directory::Service {
					db: db.clone(),
				},
				event_handler: rooms::event_handler::Service,
				lazy_loading: rooms::lazy_loading::Service {
					db: db.clone(),
					lazy_load_waiting: Mutex::new(HashMap::new()),
				},
				metadata: rooms::metadata::Service {
					db: db.clone(),
				},
				outlier: rooms::outlier::Service {
					db: db.clone(),
				},
				pdu_metadata: rooms::pdu_metadata::Service {
					db: db.clone(),
				},
				read_receipt: rooms::read_receipt::Service {
					db: db.clone(),
				},
				search: rooms::search::Service {
					db: db.clone(),
				},
				short: rooms::short::Service {
					db: db.clone(),
				},
				state: rooms::state::Service {
					db: db.clone(),
				},
				state_accessor: rooms::state_accessor::Service {
					db: db.clone(),
					server_visibility_cache: StdMutex::new(LruCache::new(
						(f64::from(config.server_visibility_cache_capacity) * config.conduit_cache_capacity_modifier)
							as usize,
					)),
					user_visibility_cache: StdMutex::new(LruCache::new(
						(f64::from(config.user_visibility_cache_capacity) * config.conduit_cache_capacity_modifier)
							as usize,
					)),
				},
				state_cache: rooms::state_cache::Service {
					db: db.clone(),
				},
				state_compressor: rooms::state_compressor::Service {
					db: db.clone(),
					stateinfo_cache: StdMutex::new(LruCache::new(
						(f64::from(config.stateinfo_cache_capacity) * config.conduit_cache_capacity_modifier) as usize,
					)),
				},
				timeline: rooms::timeline::Service {
					db: db.clone(),
					lasttimelinecount_cache: Mutex::new(HashMap::new()),
				},
				threads: rooms::threads::Service {
					db: db.clone(),
				},
				typing: rooms::typing::Service {
					typing: RwLock::new(BTreeMap::new()),
					last_typing_update: RwLock::new(BTreeMap::new()),
					typing_update_sender: broadcast::channel(100).0,
				},
				spaces: rooms::spaces::Service {
					roomid_spacehierarchy_cache: Mutex::new(LruCache::new(
						(f64::from(config.roomid_spacehierarchy_cache_capacity)
							* config.conduit_cache_capacity_modifier) as usize,
					)),
				},
				user: rooms::user::Service {
					db: db.clone(),
				},
			},
			transaction_ids: transaction_ids::Service {
				db: db.clone(),
			},
			uiaa: uiaa::Service {
				db: db.clone(),
			},
			users: users::Service {
				db: db.clone(),
				connections: StdMutex::new(BTreeMap::new()),
			},
			account_data: account_data::Service {
				db: db.clone(),
			},
			presence: presence::Service::build(db.clone(), config),
			admin: admin::Service::build(),
			key_backups: key_backups::Service {
				db: db.clone(),
			},
			media: media::Service {
				db: db.clone(),
				url_preview_mutex: RwLock::new(HashMap::new()),
			},
			sending: sending::Service::build(db.clone(), config),
			globals: globals::Service::load(db.clone(), config)?,
			server,
			db,
		})
	}

	pub async fn memory_usage(&self) -> String {
		let lazy_load_waiting = self.rooms.lazy_loading.lazy_load_waiting.lock().await.len();
		let server_visibility_cache = self
			.rooms
			.state_accessor
			.server_visibility_cache
			.lock()
			.unwrap()
			.len();
		let user_visibility_cache = self
			.rooms
			.state_accessor
			.user_visibility_cache
			.lock()
			.unwrap()
			.len();
		let stateinfo_cache = self
			.rooms
			.state_compressor
			.stateinfo_cache
			.lock()
			.unwrap()
			.len();
		let lasttimelinecount_cache = self
			.rooms
			.timeline
			.lasttimelinecount_cache
			.lock()
			.await
			.len();
		let roomid_spacehierarchy_cache = self
			.rooms
			.spaces
			.roomid_spacehierarchy_cache
			.lock()
			.await
			.len();
		let resolver_overrides_cache = self.globals.resolver.overrides.read().unwrap().len();
		let resolver_destinations_cache = self.globals.resolver.destinations.read().await.len();
		let bad_event_ratelimiter = self.globals.bad_event_ratelimiter.read().await.len();
		let bad_query_ratelimiter = self.globals.bad_query_ratelimiter.read().await.len();
		let bad_signature_ratelimiter = self.globals.bad_signature_ratelimiter.read().await.len();

		format!(
			"\
lazy_load_waiting: {lazy_load_waiting}
server_visibility_cache: {server_visibility_cache}
user_visibility_cache: {user_visibility_cache}
stateinfo_cache: {stateinfo_cache}
lasttimelinecount_cache: {lasttimelinecount_cache}
roomid_spacehierarchy_cache: {roomid_spacehierarchy_cache}
resolver_overrides_cache: {resolver_overrides_cache}
resolver_destinations_cache: {resolver_destinations_cache}
bad_event_ratelimiter: {bad_event_ratelimiter}
bad_query_ratelimiter: {bad_query_ratelimiter}
bad_signature_ratelimiter: {bad_signature_ratelimiter}
"
		)
	}

	pub async fn clear_caches(&self, amount: u32) {
		if amount > 0 {
			self.rooms
				.lazy_loading
				.lazy_load_waiting
				.lock()
				.await
				.clear();
		}
		if amount > 1 {
			self.rooms
				.state_accessor
				.server_visibility_cache
				.lock()
				.unwrap()
				.clear();
		}
		if amount > 2 {
			self.rooms
				.state_accessor
				.user_visibility_cache
				.lock()
				.unwrap()
				.clear();
		}
		if amount > 3 {
			self.rooms
				.state_compressor
				.stateinfo_cache
				.lock()
				.unwrap()
				.clear();
		}
		if amount > 4 {
			self.rooms
				.timeline
				.lasttimelinecount_cache
				.lock()
				.await
				.clear();
		}
		if amount > 5 {
			self.rooms
				.spaces
				.roomid_spacehierarchy_cache
				.lock()
				.await
				.clear();
		}
		if amount > 6 {
			self.globals.resolver.overrides.write().unwrap().clear();
			self.globals.resolver.destinations.write().await.clear();
		}
		if amount > 7 {
			self.globals.resolver.resolver.clear_cache();
		}
		if amount > 8 {
			self.globals.bad_event_ratelimiter.write().await.clear();
		}
		if amount > 9 {
			self.globals.bad_query_ratelimiter.write().await.clear();
		}
		if amount > 10 {
			self.globals.bad_signature_ratelimiter.write().await.clear();
		}
	}

	pub async fn start(&self) -> Result<()> {
		debug_info!("Starting services");
		globals::migrations::migrations(&self.db, &self.globals.config).await?;

		self.admin.start_handler().await;

		globals::emerg_access::init_emergency_access().await;

		self.sending.start_handler().await;

		if self.globals.config.allow_local_presence {
			self.presence.start_handler().await;
		}

		if self.globals.allow_check_for_updates() {
			let handle = globals::updates::start_check_for_updates_task().await?;
			_ = self.globals.updates_handle.lock().await.insert(handle);
		}

		debug_info!("Services startup complete.");
		Ok(())
	}

	pub async fn interrupt(&self) {
		trace!("Interrupting services...");
		self.server.interrupt.store(true, atomic::Ordering::Release);

		self.globals.rotate.fire();
		self.sending.interrupt();
		self.presence.interrupt();
		self.admin.interrupt();

		trace!("Services interrupt complete.");
	}

	#[tracing::instrument(skip_all)]
	pub async fn shutdown(&self) {
		info!("Shutting down services");
		self.interrupt().await;

		debug!("Removing unix socket file.");
		if let Some(path) = self.globals.unix_socket_path().as_ref() {
			_ = fs::remove_file(path).await;
		}

		debug!("Waiting for update worker...");
		if let Some(updates_handle) = self.globals.updates_handle.lock().await.take() {
			updates_handle.abort();
			_ = updates_handle.await;
		}

		debug!("Waiting for admin worker...");
		self.admin.close().await;

		debug!("Waiting for presence worker...");
		self.presence.close().await;

		debug!("Waiting for sender...");
		self.sending.close().await;

		debug_info!("Services shutdown complete.");
	}
}
