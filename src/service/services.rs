use std::{any::Any, collections::BTreeMap, fmt::Write, sync::Arc};

use conduit::{debug, debug_info, info, trace, Result, Server};
use database::Database;
use tokio::sync::Mutex;

use crate::{
	account_data, admin, appservice, client, globals, key_backups,
	manager::Manager,
	media, presence, pusher, resolver, rooms, sending, service,
	service::{Args, Map, Service},
	transaction_ids, uiaa, updates, users,
};

pub struct Services {
	pub resolver: Arc<resolver::Service>,
	pub client: Arc<client::Service>,
	pub globals: Arc<globals::Service>,
	pub rooms: rooms::Service,
	pub appservice: Arc<appservice::Service>,
	pub pusher: Arc<pusher::Service>,
	pub transaction_ids: Arc<transaction_ids::Service>,
	pub uiaa: Arc<uiaa::Service>,
	pub users: Arc<users::Service>,
	pub account_data: Arc<account_data::Service>,
	pub presence: Arc<presence::Service>,
	pub admin: Arc<admin::Service>,
	pub key_backups: Arc<key_backups::Service>,
	pub media: Arc<media::Service>,
	pub sending: Arc<sending::Service>,
	pub updates: Arc<updates::Service>,

	manager: Mutex<Option<Arc<Manager>>>,
	pub(crate) service: Arc<Map>,
	pub server: Arc<Server>,
	pub db: Arc<Database>,
}

macro_rules! build_service {
	($map:ident, $server:ident, $db:ident, $tyname:ty) => {{
		let built = <$tyname>::build(Args {
			server: &$server,
			db: &$db,
			service: &$map,
		})?;

		Arc::get_mut(&mut $map)
			.expect("must have mutable reference to services collection")
			.insert(built.name().to_owned(), (built.clone(), built.clone()));

		trace!("built service #{}: {:?}", $map.len(), built.name());
		built
	}};
}

impl Services {
	#[allow(clippy::cognitive_complexity)]
	pub fn build(server: Arc<Server>, db: Arc<Database>) -> Result<Self> {
		let mut service: Arc<Map> = Arc::new(BTreeMap::new());
		macro_rules! build {
			($srv:ty) => {
				build_service!(service, server, db, $srv)
			};
		}

		Ok(Self {
			globals: build!(globals::Service),
			resolver: build!(resolver::Service),
			client: build!(client::Service),
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
			updates: build!(updates::Service),
			manager: Mutex::new(None),
			service,
			server,
			db,
		})
	}

	pub(super) async fn start(&self) -> Result<()> {
		debug_info!("Starting services...");

		globals::migrations::migrations(&self.db, &self.server.config).await?;
		self.manager
			.lock()
			.await
			.insert(Manager::new(self))
			.clone()
			.start()
			.await?;

		debug_info!("Services startup complete.");
		Ok(())
	}

	pub(super) async fn stop(&self) {
		info!("Shutting down services...");

		self.interrupt();
		if let Some(manager) = self.manager.lock().await.as_ref() {
			manager.stop().await;
		}

		debug_info!("Services shutdown complete.");
	}

	pub async fn poll(&self) -> Result<()> {
		if let Some(manager) = self.manager.lock().await.as_ref() {
			return manager.poll().await;
		}

		Ok(())
	}

	pub async fn clear_cache(&self) {
		for (service, ..) in self.service.values() {
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

	pub async fn memory_usage(&self) -> Result<String> {
		let mut out = String::new();
		for (service, ..) in self.service.values() {
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

	fn interrupt(&self) {
		debug!("Interrupting services...");

		for (name, (service, ..)) in self.service.iter() {
			trace!("Interrupting {name}");
			service.interrupt();
		}
	}

	pub fn try_get<T>(&self, name: &str) -> Result<Arc<T>>
	where
		T: Any + Send + Sync,
	{
		service::try_get::<T>(&self.service, name)
	}

	pub fn get<T>(&self, name: &str) -> Option<Arc<T>>
	where
		T: Any + Send + Sync,
	{
		service::get::<T>(&self.service, name)
	}
}
