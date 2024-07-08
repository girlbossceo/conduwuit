use std::{collections::BTreeMap, fmt::Write, sync::Arc};

use conduit::{debug, debug_info, info, trace, Result, Server};
use database::Database;

use crate::{
	account_data, admin, appservice, globals, key_backups, media, presence, pusher, rooms, sending,
	service::{Args, Map, Service},
	transaction_ids, uiaa, users,
};

pub struct Services {
	pub rooms: rooms::Service,
	pub appservice: Arc<appservice::Service>,
	pub pusher: Arc<pusher::Service>,
	pub transaction_ids: Arc<transaction_ids::Service>,
	pub uiaa: Arc<uiaa::Service>,
	pub users: Arc<users::Service>,
	pub account_data: Arc<account_data::Service>,
	pub presence: Arc<presence::Service>,
	pub admin: Arc<admin::Service>,
	pub globals: Arc<globals::Service>,
	pub key_backups: Arc<key_backups::Service>,
	pub media: Arc<media::Service>,
	pub sending: Arc<sending::Service>,

	pub(crate) service: Map,
	pub server: Arc<Server>,
	pub db: Arc<Database>,
}

impl Services {
	pub fn build(server: Arc<Server>, db: Arc<Database>) -> Result<Self> {
		let mut service: Map = BTreeMap::new();
		macro_rules! build {
			($tyname:ty) => {{
				let built = <$tyname>::build(Args {
					server: &server,
					db: &db,
					_service: &service,
				})?;
				service.insert(built.name().to_owned(), built.clone());
				built
			}};
		}

		Ok(Self {
			rooms: rooms::Service {
				alias: build!(rooms::alias::Service),
				auth_chain: build!(rooms::auth_chain::Service),
				directory: build!(rooms::directory::Service),
				event_handler: build!(rooms::event_handler::Service),
				lazy_loading: build!(rooms::lazy_loading::Service),
				metadata: build!(rooms::metadata::Service),
				outlier: build!(rooms::outlier::Service),
				pdu_metadata: build!(rooms::pdu_metadata::Service),
				read_receipt: build!(rooms::read_receipt::Service),
				search: build!(rooms::search::Service),
				short: build!(rooms::short::Service),
				state: build!(rooms::state::Service),
				state_accessor: build!(rooms::state_accessor::Service),
				state_cache: build!(rooms::state_cache::Service),
				state_compressor: build!(rooms::state_compressor::Service),
				timeline: build!(rooms::timeline::Service),
				threads: build!(rooms::threads::Service),
				typing: build!(rooms::typing::Service),
				spaces: build!(rooms::spaces::Service),
				user: build!(rooms::user::Service),
			},
			appservice: build!(appservice::Service),
			pusher: build!(pusher::Service),
			transaction_ids: build!(transaction_ids::Service),
			uiaa: build!(uiaa::Service),
			users: build!(users::Service),
			account_data: build!(account_data::Service),
			presence: build!(presence::Service),
			admin: build!(admin::Service),
			key_backups: build!(key_backups::Service),
			media: build!(media::Service),
			sending: build!(sending::Service),
			globals: build!(globals::Service),
			service,
			server,
			db,
		})
	}

	pub async fn memory_usage(&self) -> Result<String> {
		let mut out = String::new();
		for service in self.service.values() {
			service.memory_usage(&mut out)?;
		}

		//TODO
		let roomid_spacehierarchy_cache = self
			.rooms
			.spaces
			.roomid_spacehierarchy_cache
			.lock()
			.await
			.len();
		writeln!(out, "roomid_spacehierarchy_cache: {roomid_spacehierarchy_cache}")?;

		Ok(out)
	}

	pub async fn clear_cache(&self) {
		for service in self.service.values() {
			service.clear_cache();
		}

		//TODO
		self.rooms
			.spaces
			.roomid_spacehierarchy_cache
			.lock()
			.await
			.clear();
	}

	pub async fn start(&self) -> Result<()> {
		debug_info!("Starting services");

		self.media.create_media_dir().await?;
		globals::migrations::migrations(&self.db, &self.globals.config).await?;
		globals::emerg_access::init_emergency_access();

		for (name, service) in &self.service {
			debug!("Starting {name}");
			service.clone().start().await?;
		}

		if self.globals.allow_check_for_updates() {
			let handle = globals::updates::start_check_for_updates_task();
			_ = self.globals.updates_handle.lock().await.insert(handle);
		}

		debug_info!("Services startup complete.");
		Ok(())
	}

	pub fn interrupt(&self) {
		debug!("Interrupting services...");

		for (name, service) in &self.service {
			trace!("Interrupting {name}");
			service.interrupt();
		}
	}

	pub async fn stop(&self) {
		info!("Shutting down services");
		self.interrupt();

		debug!("Waiting for update worker...");
		if let Some(updates_handle) = self.globals.updates_handle.lock().await.take() {
			updates_handle.abort();
			_ = updates_handle.await;
		}

		for (name, service) in &self.service {
			debug!("Waiting for {name} ...");
			service.stop().await;
		}

		debug_info!("Services shutdown complete.");
	}
}
