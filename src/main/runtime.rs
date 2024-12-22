use std::{
	iter::once,
	sync::{
		atomic::{AtomicUsize, Ordering},
		OnceLock,
	},
	thread,
	time::Duration,
};

use conduwuit::{
	utils::sys::compute::{get_core_available, set_affinity},
	Result,
};
use tokio::runtime::Builder;

use crate::clap::Args;

const WORKER_NAME: &str = "conduwuit:worker";
const WORKER_MIN: usize = 2;
const WORKER_KEEPALIVE: u64 = 36;
const MAX_BLOCKING_THREADS: usize = 2048;

static WORKER_AFFINITY: OnceLock<bool> = OnceLock::new();

pub(super) fn new(args: &Args) -> Result<tokio::runtime::Runtime> {
	WORKER_AFFINITY
		.set(args.worker_affinity)
		.expect("set WORKER_AFFINITY from program argument");

	let mut builder = Builder::new_multi_thread();
	builder
		.enable_io()
		.enable_time()
		.thread_name(WORKER_NAME)
		.worker_threads(args.worker_threads.max(WORKER_MIN))
		.max_blocking_threads(MAX_BLOCKING_THREADS)
		.thread_keep_alive(Duration::from_secs(WORKER_KEEPALIVE))
		.global_queue_interval(args.global_event_interval)
		.event_interval(args.kernel_event_interval)
		.max_io_events_per_tick(args.kernel_events_per_tick)
		.on_thread_start(thread_start)
		.on_thread_stop(thread_stop)
		.on_thread_unpark(thread_unpark)
		.on_thread_park(thread_park);

	#[cfg(tokio_unstable)]
	builder
		.on_task_spawn(task_spawn)
		.on_task_terminate(task_terminate);

	#[cfg(tokio_unstable)]
	enable_histogram(&mut builder, args);

	builder.build().map_err(Into::into)
}

#[cfg(tokio_unstable)]
fn enable_histogram(builder: &mut Builder, args: &Args) {
	use tokio::runtime::HistogramConfiguration;

	let buckets = args.worker_histogram_buckets;
	let interval = Duration::from_micros(args.worker_histogram_interval);
	let linear = HistogramConfiguration::linear(interval, buckets);
	builder
		.enable_metrics_poll_time_histogram()
		.metrics_poll_time_histogram_configuration(linear);
}

#[tracing::instrument(
	name = "fork",
	level = "debug",
	skip_all,
	fields(
		id = ?thread::current().id(),
		name = %thread::current().name().unwrap_or("None"),
	),
)]
fn thread_start() {
	if WORKER_AFFINITY
		.get()
		.copied()
		.expect("WORKER_AFFINITY initialized by runtime::new()")
	{
		set_worker_affinity();
	}
}

fn set_worker_affinity() {
	static CORES_OCCUPIED: AtomicUsize = AtomicUsize::new(0);

	if thread::current().name() != Some(WORKER_NAME) {
		return;
	}

	let handle = tokio::runtime::Handle::current();
	let num_workers = handle.metrics().num_workers();
	let i = CORES_OCCUPIED.fetch_add(1, Ordering::Relaxed);
	if i >= num_workers {
		return;
	}

	let Some(id) = get_core_available(i) else {
		return;
	};

	set_affinity(once(id));
}

#[tracing::instrument(
	name = "join",
	level = "debug",
	skip_all,
	fields(
		id = ?thread::current().id(),
		name = %thread::current().name().unwrap_or("None"),
	),
)]
fn thread_stop() {}

#[tracing::instrument(
	name = "work",
	level = "trace",
	skip_all,
	fields(
		id = ?thread::current().id(),
		name = %thread::current().name().unwrap_or("None"),
	),
)]
fn thread_unpark() {}

#[tracing::instrument(
	name = "park",
	level = "trace",
	skip_all,
	fields(
		id = ?thread::current().id(),
		name = %thread::current().name().unwrap_or("None"),
	),
)]
fn thread_park() {}

#[cfg(tokio_unstable)]
#[tracing::instrument(
	name = "spawn",
	level = "trace",
	skip_all,
	fields(
		id = %meta.id(),
	),
)]
fn task_spawn(meta: &tokio::runtime::TaskMeta<'_>) {}

#[cfg(tokio_unstable)]
#[tracing::instrument(
	name = "finish",
	level = "trace",
	skip_all,
	fields(
		id = %meta.id()
	),
)]
fn task_terminate(meta: &tokio::runtime::TaskMeta<'_>) {}
