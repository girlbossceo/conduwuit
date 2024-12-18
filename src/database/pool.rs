use std::{
	mem::take,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc, Mutex,
	},
};

use async_channel::{bounded, Receiver, RecvError, Sender};
use conduwuit::{debug, debug_warn, defer, err, implement, result::DebugInspect, Result, Server};
use futures::{channel::oneshot, TryFutureExt};
use oneshot::Sender as ResultSender;
use rocksdb::Direction;
use tokio::task::JoinSet;

use crate::{keyval::KeyBuf, stream, Handle, Map};

pub(crate) struct Pool {
	server: Arc<Server>,
	workers: Mutex<JoinSet<()>>,
	queue: Sender<Cmd>,
	busy: AtomicUsize,
	queued_max: AtomicUsize,
}

pub(crate) struct Opts {
	pub(crate) queue_size: usize,
	pub(crate) worker_num: usize,
}

pub(crate) enum Cmd {
	Get(Get),
	Iter(Seek),
}

pub(crate) struct Get {
	pub(crate) map: Arc<Map>,
	pub(crate) key: KeyBuf,
	pub(crate) res: Option<ResultSender<Result<Handle<'static>>>>,
}

pub(crate) struct Seek {
	pub(crate) map: Arc<Map>,
	pub(crate) state: stream::State<'static>,
	pub(crate) dir: Direction,
	pub(crate) key: Option<KeyBuf>,
	pub(crate) res: Option<ResultSender<stream::State<'static>>>,
}

const QUEUE_LIMIT: (usize, usize) = (1, 3072);
const WORKER_LIMIT: (usize, usize) = (1, 512);

impl Drop for Pool {
	fn drop(&mut self) {
		debug_assert!(self.queue.is_empty(), "channel must be empty on drop");
		debug_assert!(self.queue.is_closed(), "channel should be closed on drop");
	}
}

#[implement(Pool)]
pub(crate) async fn new(server: &Arc<Server>, opts: &Opts) -> Result<Arc<Self>> {
	let queue_size = opts.queue_size.clamp(QUEUE_LIMIT.0, QUEUE_LIMIT.1);
	let (send, recv) = bounded(queue_size);
	let pool = Arc::new(Self {
		server: server.clone(),
		workers: JoinSet::new().into(),
		queue: send,
		busy: AtomicUsize::default(),
		queued_max: AtomicUsize::default(),
	});

	let worker_num = opts.worker_num.clamp(WORKER_LIMIT.0, WORKER_LIMIT.1);
	pool.spawn_until(recv, worker_num).await?;

	Ok(pool)
}

#[implement(Pool)]
pub(crate) async fn shutdown(self: &Arc<Self>) {
	self.close();

	let workers = take(&mut *self.workers.lock().expect("locked"));
	debug!(workers = workers.len(), "Waiting for workers to join...");

	workers.join_all().await;
	debug_assert!(self.queue.is_empty(), "channel is not empty");
}

#[implement(Pool)]
pub(crate) fn close(&self) -> bool {
	if !self.queue.close() {
		return false;
	}

	let mut workers = take(&mut *self.workers.lock().expect("locked"));
	debug!(workers = workers.len(), "Waiting for workers to join...");
	workers.abort_all();
	drop(workers);

	std::thread::yield_now();
	debug_assert!(self.queue.is_empty(), "channel is not empty");
	debug!(
		senders = self.queue.sender_count(),
		receivers = self.queue.receiver_count(),
		"Closed pool channel"
	);

	true
}

#[implement(Pool)]
async fn spawn_until(self: &Arc<Self>, recv: Receiver<Cmd>, max: usize) -> Result {
	let mut workers = self.workers.lock().expect("locked");
	while workers.len() < max {
		self.spawn_one(&mut workers, recv.clone())?;
	}

	Ok(())
}

#[implement(Pool)]
#[tracing::instrument(
	name = "spawn",
	level = "trace",
	skip_all,
	fields(id = %workers.len())
)]
fn spawn_one(self: &Arc<Self>, workers: &mut JoinSet<()>, recv: Receiver<Cmd>) -> Result {
	let id = workers.len();
	let self_ = self.clone();

	#[cfg(not(tokio_unstable))]
	let _abort = workers.spawn_blocking_on(move || self_.worker(id, recv), self.server.runtime());

	#[cfg(tokio_unstable)]
	let _abort = workers
		.build_task()
		.name("conduwuit:dbpool")
		.spawn_blocking_on(move || self_.worker(id, recv), self.server.runtime());

	Ok(())
}

#[implement(Pool)]
#[tracing::instrument(level = "trace", name = "get", skip(self, cmd))]
pub(crate) async fn execute_get(&self, mut cmd: Get) -> Result<Handle<'_>> {
	let (send, recv) = oneshot::channel();
	_ = cmd.res.insert(send);
	self.execute(Cmd::Get(cmd))
		.and_then(|()| {
			recv.map_ok(into_recv_get_result)
				.map_err(|e| err!(error!("recv failed {e:?}")))
		})
		.await?
}

#[implement(Pool)]
#[tracing::instrument(level = "trace", name = "iter", skip(self, cmd))]
pub(crate) async fn execute_iter(&self, mut cmd: Seek) -> Result<stream::State<'_>> {
	let (send, recv) = oneshot::channel();
	_ = cmd.res.insert(send);
	self.execute(Cmd::Iter(cmd))
		.and_then(|()| {
			recv.map_ok(into_recv_seek)
				.map_err(|e| err!(error!("recv failed {e:?}")))
		})
		.await
}

#[implement(Pool)]
#[tracing::instrument(
	level = "trace",
	name = "execute",
	skip(self, cmd),
	fields(
		task = ?tokio::task::try_id(),
		receivers = self.queue.receiver_count(),
		queued = self.queue.len(),
		queued_max = self.queued_max.load(Ordering::Relaxed),
	),
)]
async fn execute(&self, cmd: Cmd) -> Result {
	if cfg!(debug_assertions) {
		self.queued_max
			.fetch_max(self.queue.len(), Ordering::Relaxed);
	}

	if self.queue.is_full() {
		debug_warn!(
			capacity = ?self.queue.capacity(),
			"pool queue is full"
		);
	}

	self.queue
		.send(cmd)
		.await
		.map_err(|e| err!(error!("send failed {e:?}")))
}

#[implement(Pool)]
#[tracing::instrument(
	parent = None,
	level = "debug",
	skip(self, recv),
	fields(
		tid = ?std::thread::current().id(),
	),
)]
fn worker(self: Arc<Self>, id: usize, recv: Receiver<Cmd>) {
	debug!("worker spawned");
	defer! {{ debug!("worker finished"); }}

	self.worker_loop(&recv);
}

#[implement(Pool)]
fn worker_loop(&self, recv: &Receiver<Cmd>) {
	// initial +1 needed prior to entering wait
	self.busy.fetch_add(1, Ordering::Relaxed);

	while let Ok(cmd) = self.worker_wait(recv) {
		self.worker_handle(cmd);
	}
}

#[implement(Pool)]
#[tracing::instrument(
	name = "wait",
	level = "trace",
	skip_all,
	fields(
		receivers = recv.receiver_count(),
		queued = recv.len(),
		busy = self.busy.fetch_sub(1, Ordering::Relaxed) - 1,
	),
)]
fn worker_wait(&self, recv: &Receiver<Cmd>) -> Result<Cmd, RecvError> {
	recv.recv_blocking().debug_inspect(|_| {
		self.busy.fetch_add(1, Ordering::Relaxed);
	})
}

#[implement(Pool)]
fn worker_handle(&self, cmd: Cmd) {
	match cmd {
		| Cmd::Get(cmd) => self.handle_get(cmd),
		| Cmd::Iter(cmd) => self.handle_iter(cmd),
	}
}

#[implement(Pool)]
#[tracing::instrument(
	name = "iter",
	level = "trace",
	skip_all,
	fields(%cmd.map),
)]
fn handle_iter(&self, mut cmd: Seek) {
	let chan = cmd.res.take().expect("missing result channel");

	if chan.is_canceled() {
		return;
	}

	let from = cmd.key.as_deref().map(Into::into);
	let result = match cmd.dir {
		| Direction::Forward => cmd.state.init_fwd(from),
		| Direction::Reverse => cmd.state.init_rev(from),
	};

	let chan_result = chan.send(into_send_seek(result));
	let _chan_sent = chan_result.is_ok();
}

#[implement(Pool)]
#[tracing::instrument(
	name = "get",
	level = "trace",
	skip_all,
	fields(%cmd.map),
)]
fn handle_get(&self, mut cmd: Get) {
	debug_assert!(!cmd.key.is_empty(), "querying for empty key");

	// Obtain the result channel.
	let chan = cmd.res.take().expect("missing result channel");

	// It is worth checking if the future was dropped while the command was queued
	// so we can bail without paying for any query.
	if chan.is_canceled() {
		return;
	}

	// Perform the actual database query. We reuse our database::Map interface but
	// limited to the blocking calls, rather than creating another surface directly
	// with rocksdb here.
	let result = cmd.map.get_blocking(&cmd.key);

	// Send the result back to the submitter.
	let chan_result = chan.send(into_send_get_result(result));

	// If the future was dropped during the query this will fail acceptably.
	let _chan_sent = chan_result.is_ok();
}

fn into_send_get_result(result: Result<Handle<'_>>) -> Result<Handle<'static>> {
	// SAFETY: Necessary to send the Handle (rust_rocksdb::PinnableSlice) through
	// the channel. The lifetime on the handle is a device by rust-rocksdb to
	// associate a database lifetime with its assets. The Handle must be dropped
	// before the database is dropped.
	unsafe { std::mem::transmute(result) }
}

fn into_recv_get_result(result: Result<Handle<'static>>) -> Result<Handle<'_>> {
	// SAFETY: This is to receive the Handle from the channel.
	unsafe { std::mem::transmute(result) }
}

pub(crate) fn into_send_seek(result: stream::State<'_>) -> stream::State<'static> {
	// SAFETY: Necessary to send the State through the channel; see above.
	unsafe { std::mem::transmute(result) }
}

fn into_recv_seek(result: stream::State<'static>) -> stream::State<'_> {
	// SAFETY: This is to receive the State from the channel; see above.
	unsafe { std::mem::transmute(result) }
}
