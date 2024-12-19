use std::{sync::OnceLock, thread, time::Duration};

use conduwuit::Result;
use tokio::runtime::Builder;

use crate::clap::Args;

const WORKER_NAME: &str = "conduwuit:worker";
const WORKER_MIN: usize = 2;
const WORKER_KEEPALIVE: u64 = 36;
const GLOBAL_QUEUE_INTERVAL: u32 = 192;
const KERNEL_QUEUE_INTERVAL: u32 = 256;
const KERNEL_EVENTS_PER_TICK: usize = 512;

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
		.thread_keep_alive(Duration::from_secs(WORKER_KEEPALIVE))
		.max_io_events_per_tick(KERNEL_EVENTS_PER_TICK)
		.event_interval(KERNEL_QUEUE_INTERVAL)
		.global_queue_interval(GLOBAL_QUEUE_INTERVAL)
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
	#[cfg(feature = "worker_affinity")]
	if WORKER_AFFINITY
		.get()
		.copied()
		.expect("WORKER_AFFINITY initialized by runtime::new()")
	{
		set_worker_affinity();
	}
}

#[cfg(feature = "worker_affinity")]
fn set_worker_affinity() {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		LazyLock,
	};

	static CORES_OCCUPIED: AtomicUsize = AtomicUsize::new(0);
	static CORES_AVAILABLE: LazyLock<Option<Vec<core_affinity::CoreId>>> = LazyLock::new(|| {
		core_affinity::get_core_ids().map(|mut cores| {
			cores.sort_unstable();
			cores
		})
	});

	let Some(cores) = CORES_AVAILABLE.as_ref() else {
		return;
	};

	if thread::current().name() != Some(WORKER_NAME) {
		return;
	}

	let handle = tokio::runtime::Handle::current();
	let num_workers = handle.metrics().num_workers();
	let i = CORES_OCCUPIED.fetch_add(1, Ordering::Relaxed);
	if i >= num_workers {
		return;
	}

	let Some(id) = cores.get(i) else {
		return;
	};

	let _set = core_affinity::set_for_current(*id);
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
