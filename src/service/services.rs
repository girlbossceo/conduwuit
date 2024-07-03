use std::sync::Arc;

use conduit::{debug_info, Result, Server};
use database::Database;
use tracing::{debug, info, trace};

use crate::{
	account_data, admin, appservice, globals, key_backups, media, presence, pusher, rooms, sending, transaction_ids,
	uiaa, users,
};

pub struct Services {
	pub rooms: rooms::Service,
	pub appservice: appservice::Service,
	pub pusher: pusher::Service,
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
	pub db: Arc<Database>,
}

impl Services {
	pub async fn build(server: Arc<Server>, db: Arc<Database>) -> Result<Self> {
		Ok(Self {
			rooms: rooms::Service {
				alias: rooms::alias::Service::build(&server, &db)?,
				auth_chain: rooms::auth_chain::Service::build(&server, &db)?,
				directory: rooms::directory::Service::build(&server, &db)?,
				event_handler: rooms::event_handler::Service::build(&server, &db)?,
				lazy_loading: rooms::lazy_loading::Service::build(&server, &db)?,
				metadata: rooms::metadata::Service::build(&server, &db)?,
				outlier: rooms::outlier::Service::build(&server, &db)?,
				pdu_metadata: rooms::pdu_metadata::Service::build(&server, &db)?,
				read_receipt: rooms::read_receipt::Service::build(&server, &db)?,
				search: rooms::search::Service::build(&server, &db)?,
				short: rooms::short::Service::build(&server, &db)?,
				state: rooms::state::Service::build(&server, &db)?,
				state_accessor: rooms::state_accessor::Service::build(&server, &db)?,
				state_cache: rooms::state_cache::Service::build(&server, &db)?,
				state_compressor: rooms::state_compressor::Service::build(&server, &db)?,
				timeline: rooms::timeline::Service::build(&server, &db)?,
				threads: rooms::threads::Service::build(&server, &db)?,
				typing: rooms::typing::Service::build(&server, &db)?,
				spaces: rooms::spaces::Service::build(&server, &db)?,
				user: rooms::user::Service::build(&server, &db)?,
			},
			appservice: appservice::Service::build(&server, &db)?,
			pusher: pusher::Service::build(&server, &db)?,
			transaction_ids: transaction_ids::Service::build(&server, &db)?,
			uiaa: uiaa::Service::build(&server, &db)?,
			users: users::Service::build(&server, &db)?,
			account_data: account_data::Service::build(&server, &db)?,
			presence: presence::Service::build(&server, &db)?,
			admin: admin::Service::build(&server, &db)?,
			key_backups: key_backups::Service::build(&server, &db)?,
			media: media::Service::build(&server, &db)?,
			sending: sending::Service::build(&server, &db)?,
			globals: globals::Service::build(&server, &db)?,
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
		let resolver_overrides_cache = self
			.globals
			.resolver
			.overrides
			.read()
			.expect("locked for reading")
			.len();
		let resolver_destinations_cache = self
			.globals
			.resolver
			.destinations
			.read()
			.expect("locked for reading")
			.len();
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
			self.globals
				.resolver
				.overrides
				.write()
				.expect("locked for writing")
				.clear();
			self.globals
				.resolver
				.destinations
				.write()
				.expect("locked for writing")
				.clear();
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

		self.media.create_media_dir().await?;
		globals::migrations::migrations(&self.db, &self.globals.config).await?;
		globals::emerg_access::init_emergency_access();

		self.admin.start_handler().await;
		self.sending.start_handler().await;
		if self.globals.config.allow_local_presence {
			self.presence.start_handler().await;
		}

		if self.globals.allow_check_for_updates() {
			let handle = globals::updates::start_check_for_updates_task();

			#[allow(clippy::let_underscore_must_use)] // needed for shutdown
			{
				_ = self.globals.updates_handle.lock().await.insert(handle);
			}
		}

		debug_info!("Services startup complete.");
		Ok(())
	}

	pub async fn interrupt(&self) {
		trace!("Interrupting services...");
		self.sending.interrupt();
		self.presence.interrupt();
		self.admin.interrupt();

		trace!("Services interrupt complete.");
	}

	pub async fn stop(&self) {
		info!("Shutting down services");
		self.interrupt().await;

		debug!("Waiting for update worker...");
		if let Some(updates_handle) = self.globals.updates_handle.lock().await.take() {
			updates_handle.abort();

			#[allow(clippy::let_underscore_must_use)]
			{
				_ = updates_handle.await;
			}
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
