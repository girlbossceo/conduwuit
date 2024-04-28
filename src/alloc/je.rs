use std::ffi::{c_char, c_void};

use tikv_jemalloc_ctl as mallctl;
use tikv_jemalloc_sys as ffi;
use tikv_jemallocator as jemalloc;

#[global_allocator]
static JEMALLOC: jemalloc::Jemalloc = jemalloc::Jemalloc;

pub(crate) fn memory_usage() -> String {
	use mallctl::stats;
	let allocated = stats::allocated::read().unwrap_or_default() as f64 / 1024.0 / 1024.0;
	let active = stats::active::read().unwrap_or_default() as f64 / 1024.0 / 1024.0;
	let mapped = stats::mapped::read().unwrap_or_default() as f64 / 1024.0 / 1024.0;
	let metadata = stats::metadata::read().unwrap_or_default() as f64 / 1024.0 / 1024.0;
	let resident = stats::resident::read().unwrap_or_default() as f64 / 1024.0 / 1024.0;
	let retained = stats::retained::read().unwrap_or_default() as f64 / 1024.0 / 1024.0;
	format!(
		" allocated: {allocated:.2} MiB\n active: {active:.2} MiB\n mapped: {mapped:.2} MiB\n metadata: {metadata:.2} \
		 MiB\n resident: {resident:.2} MiB\n retained: {retained:.2} MiB\n "
	)
}

#[allow(clippy::ptr_as_ptr)]
pub(crate) fn memory_stats() -> String {
	const MAX_LENGTH: usize = 65536 - 4096;

	let opts_s = "d";
	let mut str = String::new();

	let opaque: *mut c_void = &mut str as *mut _ as *mut c_void;
	let opts_p: *const c_char = std::ffi::CString::new(opts_s).expect("cstring").into_raw() as *const c_char;

	// SAFETY: calls malloc_stats_print() with our string instance which must remain
	// in this frame. https://docs.rs/tikv-jemalloc-sys/latest/tikv_jemalloc_sys/fn.malloc_stats_print.html
	unsafe { ffi::malloc_stats_print(Some(malloc_stats_cb), opaque, opts_p) };

	str.truncate(MAX_LENGTH);
	format!("<pre><code>{str}</code></pre>")
}

extern "C" fn malloc_stats_cb(opaque: *mut c_void, msg: *const c_char) {
	// SAFETY: we have to trust the opaque points to our String
	let res: &mut String = unsafe { &mut *opaque.cast::<String>() };

	// SAFETY: we have to trust the string is null terminated.
	let msg = unsafe { std::ffi::CStr::from_ptr(msg) };

	let msg = String::from_utf8_lossy(msg.to_bytes());
	res.push_str(msg.as_ref());
}
