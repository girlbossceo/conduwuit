use std::{collections::BTreeMap, fmt::Write, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use conduit::{debug, debug_info, error, info, trace, utils::time, warn, Error, Result, Server};
use database::Database;
use futures_util::FutureExt;
use tokio::{
	sync::{Mutex, MutexGuard},
	task::{JoinHandle, JoinSet},
	time::sleep,
};

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

	workers: Mutex<Workers>,
	manager: Mutex<Option<JoinHandle<Result<()>>>>,
	pub(crate) service: Map,
	pub server: Arc<Server>,
	pub db: Arc<Database>,
}

type Workers = JoinSet<WorkerResult>;
type WorkerResult = (Arc<dyn Service>, Result<()>);
type WorkersLocked<'a> = MutexGuard<'a, Workers>;

const RESTART_DELAY_MS: u64 = 2500;

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
			workers: Mutex::new(JoinSet::new()),
			manager: Mutex::new(None),
			service,
			server,
			db,
		})
	}

	pub async fn start(&self) -> Result<()> {
		debug_info!("Starting services...");

		self.media.create_media_dir().await?;
		globals::migrations::migrations(&self.db, &self.globals.config).await?;
		globals::emerg_access::init_emergency_access();

		let mut workers = self.workers.lock().await;
		for service in self.service.values() {
			self.start_worker(&mut workers, service).await?;
		}

		debug!("Starting service manager...");
		let manager = async move { crate::services().manager().await };
		let manager = self.server.runtime().spawn(manager);
		_ = self.manager.lock().await.insert(manager);

		if self.globals.allow_check_for_updates() {
			let handle = globals::updates::start_check_for_updates_task();
			_ = self.globals.updates_handle.lock().await.insert(handle);
		}

		debug_info!("Services startup complete.");
		Ok(())
	}

	pub async fn stop(&self) {
		info!("Shutting down services...");
		self.interrupt();

		debug!("Waiting for update worker...");
		if let Some(updates_handle) = self.globals.updates_handle.lock().await.take() {
			updates_handle.abort();
			_ = updates_handle.await;
		}

		debug!("Stopping service manager...");
		if let Some(manager) = self.manager.lock().await.take() {
			if let Err(e) = manager.await {
				error!("Manager shutdown error: {e:?}");
			}
		}

		debug_info!("Services shutdown complete.");
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

	fn interrupt(&self) {
		debug!("Interrupting services...");
		for (name, service) in &self.service {
			trace!("Interrupting {name}");
			service.interrupt();
		}
	}

	async fn manager(&self) -> Result<()> {
		loop {
			let mut workers = self.workers.lock().await;
			tokio::select! {
				result = workers.join_next() => match result {
					Some(Ok(result)) => self.handle_result(&mut workers, result).await?,
					Some(Err(error)) => self.handle_abort(&mut workers, Error::from(error)).await?,
					None => break,
				}
			}
		}

		debug!("Worker manager finished");
		Ok(())
	}

	async fn handle_abort(&self, _workers: &mut WorkersLocked<'_>, error: Error) -> Result<()> {
		// not supported until service can be associated with abort
		unimplemented!("unexpected worker task abort {error:?}");
	}

	async fn handle_result(&self, workers: &mut WorkersLocked<'_>, result: WorkerResult) -> Result<()> {
		let (service, result) = result;
		match result {
			Ok(()) => self.handle_finished(workers, &service).await,
			Err(error) => self.handle_error(workers, &service, error).await,
		}
	}

	async fn handle_finished(&self, _workers: &mut WorkersLocked<'_>, service: &Arc<dyn Service>) -> Result<()> {
		debug!("service {:?} worker finished", service.name());
		Ok(())
	}

	async fn handle_error(
		&self, workers: &mut WorkersLocked<'_>, service: &Arc<dyn Service>, error: Error,
	) -> Result<()> {
		let name = service.name();
		error!("service {name:?} worker error: {error}");

		if !error.is_panic() {
			return Ok(());
		}

		if !self.server.running() {
			return Ok(());
		}

		let delay = Duration::from_millis(RESTART_DELAY_MS);
		warn!("service {name:?} worker restarting after {} delay", time::pretty(delay));
		sleep(delay).await;

		self.start_worker(workers, service).await
	}

	/// Start the worker in a task for the service.
	async fn start_worker(&self, workers: &mut WorkersLocked<'_>, service: &Arc<dyn Service>) -> Result<()> {
		if !self.server.running() {
			return Err(Error::Err(format!(
				"Service {:?} worker not starting during server shutdown.",
				service.name()
			)));
		}

		debug!("Service {:?} worker starting...", service.name());
		workers.spawn_on(worker(service.clone()), self.server.runtime());

		Ok(())
	}
}

/// Base frame for service worker. This runs in a tokio::task. All errors and
/// panics from the worker are caught and returned cleanly. The JoinHandle
/// should never error with a panic, and if so it should propagate, but it may
/// error with an Abort which the manager should handle along with results to
/// determine if the worker should be restarted.
async fn worker(service: Arc<dyn Service>) -> WorkerResult {
	let service_ = Arc::clone(&service);
	let result = AssertUnwindSafe(service_.worker())
		.catch_unwind()
		.await
		.map_err(Error::from_panic);

	// flattens JoinError for panic into worker's Error
	(service, result.unwrap_or_else(Err))
}
