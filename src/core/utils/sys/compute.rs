//! System utilities related to compute/processing

use std::{cell::Cell, fmt::Debug, sync::LazyLock};

use crate::is_equal_to;

/// The list of cores available to the process (at startup)
static CORES_AVAILABLE: LazyLock<u128> = LazyLock::new(|| {
	core_affinity::get_core_ids()
		.unwrap_or_default()
		.into_iter()
		.map(|core_id| core_id.id)
		.inspect(|&id| debug_assert!(id < 128, "Core ID must be < 128 at least for now"))
		.fold(0_u128, |mask, id| mask | (1 << id))
});

thread_local! {
	/// Tracks the affinity for this thread. This is updated when affinities
	/// are set via our set_affinity() interface.
	static CORE_AFFINITY: Cell<u128> = Cell::default();
}

/// Set the core affinity for this thread. The ID should be listed in
/// CORES_AVAILABLE. Empty input is a no-op; prior affinity unchanged.
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		id = ?std::thread::current().id(),
		name = %std::thread::current().name().unwrap_or("None"),
		set = ?ids.by_ref().collect::<Vec<_>>(),
		CURRENT = %format!("[b{:b}]", CORE_AFFINITY.get()),
		AVAILABLE = %format!("[b{:b}]", *CORES_AVAILABLE),
	),
)]
pub fn set_affinity<I>(mut ids: I)
where
	I: Iterator<Item = usize> + Clone + Debug,
{
	use core_affinity::{set_each_for_current, set_for_current, CoreId};

	let n = ids.clone().count();
	let mask: u128 = ids.clone().fold(0, |mask, id| {
		debug_assert!(is_core_available(id), "setting affinity to unavailable core");
		mask | (1 << id)
	});

	if n > 1 {
		set_each_for_current(ids.map(|id| CoreId { id }));
	} else if n > 0 {
		set_for_current(CoreId { id: ids.next().expect("n > 0") });
	}

	if mask.count_ones() > 0 {
		CORE_AFFINITY.replace(mask);
	}
}

/// Get the core affinity for this thread.
pub fn get_affinity() -> impl Iterator<Item = usize> { iter_bits(CORE_AFFINITY.get()) }

/// Gets the ID of the nth core available. This bijects our sequence of cores to
/// actual ID's which may have gaps for cores which are not available.
#[inline]
#[must_use]
pub fn nth_core_available(i: usize) -> Option<usize> { cores_available().nth(i) }

/// Determine if core (by id) is available to the process.
#[inline]
#[must_use]
pub fn is_core_available(id: usize) -> bool { cores_available().any(is_equal_to!(id)) }

/// Get the list of cores available. The values were recorded at program start.
#[inline]
pub fn cores_available() -> impl Iterator<Item = usize> { iter_bits(*CORES_AVAILABLE) }

/// Get the number of threads which could execute in parallel based on the
/// hardware and administrative constraints of this system. This value should be
/// used to hint the size of thread-pools and divide-and-conquer algorithms.
///
/// * <https://doc.rust-lang.org/std/thread/fn.available_parallelism.html>
#[must_use]
pub fn parallelism() -> usize {
	std::thread::available_parallelism()
		.expect("Unable to query for available parallelism.")
		.get()
}

fn iter_bits(v: u128) -> impl Iterator<Item = usize> {
	(0..128).filter(move |&i| (v & (1 << i)) != 0)
}
