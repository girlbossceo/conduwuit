//! System utilities related to compute/processing

use std::{cell::Cell, sync::LazyLock};

use crate::is_equal_to;

/// The list of cores available to the process (at startup)
static CORES_AVAILABLE: LazyLock<u128> = LazyLock::new(|| {
	core_affinity::get_core_ids()
		.unwrap_or_default()
		.into_iter()
		.map(|core_id| core_id.id)
		.inspect(|&id| assert!(id < 128, "Core ID must be < 128 at least for now"))
		.fold(0_u128, |mask, id| mask | (1 << id))
});

thread_local! {
	/// Tracks the affinity for this thread. This is updated when affinities
	/// are set via our set_affinity() interface.
	static CORE_AFFINITY: Cell<u128> = Cell::default();
}

/// Set the core affinity for this thread. The ID should be listed in
/// CORES_AVAILABLE. Empty input is a no-op; prior affinity unchanged.
pub fn set_affinity<I>(ids: I)
where
	I: Iterator<Item = usize>,
{
	use core_affinity::{set_for_current, CoreId};

	let mask: u128 = ids.fold(0, |mask, id| {
		debug_assert!(is_core_available(id), "setting affinity to unavailable core");
		set_for_current(CoreId { id });
		mask | (1 << id)
	});

	if mask.count_ones() > 0 {
		CORE_AFFINITY.replace(mask);
	}
}

/// Get the core affinity for this thread.
pub fn get_affinity() -> impl Iterator<Item = usize> {
	(0..128).filter(|&i| ((CORE_AFFINITY.get() & (1 << i)) != 0))
}

/// Gets the ID of the nth core available. This bijects our sequence of cores to
/// actual ID's which may have gaps for cores which are not available.
#[inline]
#[must_use]
pub fn get_core_available(i: usize) -> Option<usize> { cores_available().nth(i) }

/// Determine if core (by id) is available to the process.
#[inline]
#[must_use]
pub fn is_core_available(id: usize) -> bool { cores_available().any(is_equal_to!(id)) }

/// Get the list of cores available. The values were recorded at program start.
#[inline]
pub fn cores_available() -> impl Iterator<Item = usize> {
	(0..128).filter(|&i| ((*CORES_AVAILABLE & (1 << i)) != 0))
}

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
