use std::{
	mem::take,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	},
};

use async_channel::{bounded, Receiver, RecvError, Sender};
use conduit::{debug, debug_warn, defer, err, implement, result::DebugInspect, Result, Server};
use futures::channel::oneshot;
use tokio::{sync::Mutex, task::JoinSet};

use crate::{keyval::KeyBuf, Handle, Map};

pub(crate) struct Pool {
	server: Arc<Server>,
	workers: Mutex<JoinSet<()>>,
	queue: Sender<Cmd>,
	busy: AtomicUsize,
	busy_max: AtomicUsize,
	queued_max: AtomicUsize,
}

pub(crate) struct Opts {
	pub(crate) queue_size: usize,
	pub(crate) worker_num: usize,
}

#[derive(Debug)]
pub(crate) enum Cmd {
	Get(Get),
}

#[derive(Debug)]
pub(crate) struct Get {
	pub(crate) map: Arc<Map>,
	pub(crate) key: KeyBuf,
	pub(crate) res: Option<ResultSender>,
}

type ResultSender = oneshot::Sender<Result<Handle<'static>>>;

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
		busy_max: AtomicUsize::default(),
		queued_max: AtomicUsize::default(),
	});

	let worker_num = opts.worker_num.clamp(WORKER_LIMIT.0, WORKER_LIMIT.1);
	pool.spawn_until(recv, worker_num).await?;

	Ok(pool)
}

#[implement(Pool)]
pub(crate) async fn _shutdown(self: &Arc<Self>) {
	if !self.queue.is_closed() {
		self.close();
	}

	let workers = take(&mut *self.workers.lock().await);
	debug!(workers = workers.len(), "Waiting for workers to join...");

	workers.join_all().await;
	debug_assert!(self.queue.is_empty(), "channel is not empty");
}

#[implement(Pool)]
pub(crate) fn close(&self) {
	debug_assert!(!self.queue.is_closed(), "channel already closed");
	debug!(
		senders = self.queue.sender_count(),
		receivers = self.queue.receiver_count(),
		"Closing pool channel"
	);

	let closing = self.queue.close();
	debug_assert!(closing, "channel is not closing");
}

#[implement(Pool)]
async fn spawn_until(self: &Arc<Self>, recv: Receiver<Cmd>, max: usize) -> Result {
	let mut workers = self.workers.lock().await;
	while workers.len() < max {
		self.spawn_one(&mut workers, recv.clone())?;
	}

	Ok(())
}

#[implement(Pool)]
fn spawn_one(self: &Arc<Self>, workers: &mut JoinSet<()>, recv: Receiver<Cmd>) -> Result {
	let id = workers.len();

	debug!(?id, "spawning");
	let self_ = self.clone();
	let _abort = workers.spawn_blocking_on(move || self_.worker(id, recv), self.server.runtime());

	Ok(())
}

#[implement(Pool)]
#[tracing::instrument(
	level = "trace"
	skip(self, cmd),
	fields(
		task = ?tokio::task::try_id(),
		receivers = self.queue.receiver_count(),
		senders = self.queue.sender_count(),
		queued = self.queue.len(),
		queued_max = self.queued_max.load(Ordering::Relaxed),
	),
)]
pub(crate) async fn execute(&self, mut cmd: Cmd) -> Result<Handle<'_>> {
	let (send, recv) = oneshot::channel();
	Self::prepare(&mut cmd, send);

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
		.map_err(|e| err!(error!("send failed {e:?}")))?;

	recv.await
		.map(into_recv_result)
		.map_err(|e| err!(error!("recv failed {e:?}")))?
}

#[implement(Pool)]
fn prepare(cmd: &mut Cmd, send: ResultSender) {
	match cmd {
		Cmd::Get(ref mut cmd) => {
			_ = cmd.res.insert(send);
		},
	};
}

#[implement(Pool)]
#[tracing::instrument(skip(self, recv))]
fn worker(self: Arc<Self>, id: usize, recv: Receiver<Cmd>) {
	debug!("worker spawned");
	defer! {{ debug!("worker finished"); }}

	self.worker_loop(&recv);
}

#[implement(Pool)]
fn worker_loop(&self, recv: &Receiver<Cmd>) {
	// initial +1 needed prior to entering wait
	self.busy.fetch_add(1, Ordering::Relaxed);

	while let Ok(mut cmd) = self.worker_wait(recv) {
		self.worker_handle(&mut cmd);
	}
}

#[implement(Pool)]
#[tracing::instrument(
	name = "wait",
	level = "trace",
	skip_all,
	fields(
		receivers = recv.receiver_count(),
		senders = recv.sender_count(),
		queued = recv.len(),
		busy = self.busy.load(Ordering::Relaxed),
		busy_max = self.busy_max.fetch_max(
			self.busy.fetch_sub(1, Ordering::Relaxed),
			Ordering::Relaxed
		),
	),
)]
fn worker_wait(&self, recv: &Receiver<Cmd>) -> Result<Cmd, RecvError> {
	recv.recv_blocking().debug_inspect(|_| {
		self.busy.fetch_add(1, Ordering::Relaxed);
	})
}

#[implement(Pool)]
fn worker_handle(&self, cmd: &mut Cmd) {
	match cmd {
		Cmd::Get(cmd) => self.handle_get(cmd),
	}
}

#[implement(Pool)]
#[tracing::instrument(
	name = "get",
	level = "trace",
	skip_all,
	fields(%cmd.map),
)]
fn handle_get(&self, cmd: &mut Get) {
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
	let chan_result = chan.send(into_send_result(result));

	// If the future was dropped during the query this will fail acceptably.
	let _chan_sent = chan_result.is_ok();
}

fn into_send_result(result: Result<Handle<'_>>) -> Result<Handle<'static>> {
	// SAFETY: Necessary to send the Handle (rust_rocksdb::PinnableSlice) through
	// the channel. The lifetime on the handle is a device by rust-rocksdb to
	// associate a database lifetime with its assets. The Handle must be dropped
	// before the database is dropped. The handle must pass through recv_handle() on
	// the other end of the channel.
	unsafe { std::mem::transmute(result) }
}

fn into_recv_result(result: Result<Handle<'static>>) -> Result<Handle<'_>> {
	// SAFETY: This is to receive the Handle from the channel. Previously it had
	// passed through send_handle().
	unsafe { std::mem::transmute(result) }
}
