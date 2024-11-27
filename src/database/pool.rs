use std::{
	convert::identity,
	mem::take,
	sync::{Arc, Mutex},
	thread::JoinHandle,
};

use arrayvec::ArrayVec;
use async_channel::{bounded, Receiver, Sender};
use conduit::{debug, defer, err, implement, Result};
use futures::channel::oneshot;

use crate::{Handle, Map};

pub(crate) struct Pool {
	workers: Mutex<Vec<JoinHandle<()>>>,
	recv: Receiver<Cmd>,
	send: Sender<Cmd>,
}

#[derive(Default)]
pub(crate) struct Opts {
	queue_size: Option<usize>,
	worker_num: Option<usize>,
}

const WORKER_THREAD_NAME: &str = "conduwuit:db";
const DEFAULT_QUEUE_SIZE: usize = 1024;
const DEFAULT_WORKER_NUM: usize = 32;
const KEY_MAX_BYTES: usize = 384;

#[derive(Debug)]
pub(crate) enum Cmd {
	Get(Get),
}

#[derive(Debug)]
pub(crate) struct Get {
	pub(crate) map: Arc<Map>,
	pub(crate) key: ArrayVec<u8, KEY_MAX_BYTES>,
	pub(crate) res: Option<oneshot::Sender<Result<Handle<'static>>>>,
}

#[implement(Pool)]
pub(crate) fn new(opts: &Opts) -> Result<Arc<Self>> {
	let queue_size = opts.queue_size.unwrap_or(DEFAULT_QUEUE_SIZE);

	let (send, recv) = bounded(queue_size);
	let pool = Arc::new(Self {
		workers: Vec::new().into(),
		recv,
		send,
	});

	let worker_num = opts.worker_num.unwrap_or(DEFAULT_WORKER_NUM);
	pool.spawn_until(worker_num)?;

	Ok(pool)
}

#[implement(Pool)]
fn spawn_until(self: &Arc<Self>, max: usize) -> Result {
	let mut workers = self.workers.lock()?;

	while workers.len() < max {
		self.clone().spawn_one(&mut workers)?;
	}

	Ok(())
}

#[implement(Pool)]
fn spawn_one(self: Arc<Self>, workers: &mut Vec<JoinHandle<()>>) -> Result<usize> {
	use std::thread::Builder;

	let id = workers.len();

	debug!(?id, "spawning {WORKER_THREAD_NAME}...");
	let thread = Builder::new()
		.name(WORKER_THREAD_NAME.into())
		.spawn(move || self.worker(id))?;

	workers.push(thread);

	Ok(id)
}

#[implement(Pool)]
pub(crate) fn close(self: &Arc<Self>) {
	debug!(
		senders = %self.send.sender_count(),
		receivers = %self.send.receiver_count(),
		"Closing pool channel"
	);

	let closing = self.send.close();
	debug_assert!(closing, "channel is not closing");

	debug!("Shutting down pool...");
	let mut workers = self.workers.lock().expect("locked");

	debug!(
		workers = %workers.len(),
		"Waiting for workers to join..."
	);
	take(&mut *workers)
		.into_iter()
		.map(JoinHandle::join)
		.try_for_each(identity)
		.expect("failed to join worker threads");

	debug_assert!(self.send.is_empty(), "channel is not empty");
}

#[implement(Pool)]
#[tracing::instrument(skip(self, cmd), level = "trace")]
pub(crate) async fn execute(&self, mut cmd: Cmd) -> Result<Handle<'_>> {
	let (send, recv) = oneshot::channel();
	match &mut cmd {
		Cmd::Get(ref mut cmd) => {
			_ = cmd.res.insert(send);
		},
	};

	self.send
		.send(cmd)
		.await
		.map_err(|e| err!(error!("send failed {e:?}")))?;

	recv.await
		.map(into_recv_result)
		.map_err(|e| err!(error!("recv failed {e:?}")))?
}

#[implement(Pool)]
#[tracing::instrument(skip(self))]
fn worker(self: Arc<Self>, id: usize) {
	debug!(?id, "worker spawned");
	defer! {{ debug!(?id, "worker finished"); }}
	self.worker_loop(id);
}

#[implement(Pool)]
fn worker_loop(&self, id: usize) {
	while let Ok(mut cmd) = self.recv.recv_blocking() {
		self.handle(id, &mut cmd);
	}
}

#[implement(Pool)]
fn handle(&self, id: usize, cmd: &mut Cmd) {
	match cmd {
		Cmd::Get(get) => self.handle_get(id, get),
	}
}

#[implement(Pool)]
#[tracing::instrument(skip(self, cmd), fields(%cmd.map), level = "trace")]
fn handle_get(&self, id: usize, cmd: &mut Get) {
	let chan = cmd.res.take().expect("missing result channel");

	// If the future was dropped while the command was queued then we can bail
	// without any query. This makes it more efficient to use select() variants and
	// pessimistic parallel queries.
	if chan.is_canceled() {
		return;
	}

	let result = cmd.map.get_blocking(&cmd.key);
	let _sent = chan.send(into_send_result(result)).is_ok();
}

fn into_send_result(result: Result<Handle<'_>>) -> Result<Handle<'static>> {
	// SAFETY: Necessary to send the Handle (rust_rocksdb::PinnableSlice) through
	// the channel. The lifetime on the handle is a device by rust-rocksdb to
	// associate a database lifetime with its assets, not a function of rocksdb or
	// the asset. The Handle must be dropped before the database is dropped. The
	// handle must pass through recv_handle() on the other end of the channel.
	unsafe { std::mem::transmute(result) }
}

fn into_recv_result(result: Result<Handle<'static>>) -> Result<Handle<'_>> {
	// SAFETY: This is to receive the Handle from the channel. Previously it had
	// passed through send_handle().
	unsafe { std::mem::transmute(result) }
}
