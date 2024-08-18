//! jemalloc allocator

use std::ffi::{c_char, c_void};

use tikv_jemalloc_sys as ffi;
use tikv_jemallocator as jemalloc;

#[global_allocator]
static JEMALLOC: jemalloc::Jemalloc = jemalloc::Jemalloc;

#[must_use]
#[cfg(feature = "jemalloc_stats")]
pub fn memory_usage() -> Option<String> {
	use mallctl::stats;
	use tikv_jemalloc_ctl as mallctl;

	let mibs = |input: Result<usize, mallctl::Error>| {
		let input = input.unwrap_or_default();
		let kibs = input / 1024;
		let kibs = u32::try_from(kibs).unwrap_or_default();
		let kibs = f64::from(kibs);
		kibs / 1024.0
	};

	let allocated = mibs(stats::allocated::read());
	let active = mibs(stats::active::read());
	let mapped = mibs(stats::mapped::read());
	let metadata = mibs(stats::metadata::read());
	let resident = mibs(stats::resident::read());
	let retained = mibs(stats::retained::read());
	Some(format!(
		"allocated: {allocated:.2} MiB\nactive: {active:.2} MiB\nmapped: {mapped:.2} MiB\nmetadata: {metadata:.2} \
		 MiB\nresident: {resident:.2} MiB\nretained: {retained:.2} MiB\n"
	))
}

#[must_use]
#[cfg(not(feature = "jemalloc_stats"))]
pub fn memory_usage() -> Option<String> { None }

#[must_use]
pub fn memory_stats() -> Option<String> {
	const MAX_LENGTH: usize = 65536 - 4096;

	let opts_s = "d";
	let mut str = String::new();

	let opaque = std::ptr::from_mut(&mut str).cast::<c_void>();
	let opts_p: *const c_char = std::ffi::CString::new(opts_s)
		.expect("cstring")
		.into_raw()
		.cast_const();

	// SAFETY: calls malloc_stats_print() with our string instance which must remain
	// in this frame. https://docs.rs/tikv-jemalloc-sys/latest/tikv_jemalloc_sys/fn.malloc_stats_print.html
	unsafe { ffi::malloc_stats_print(Some(malloc_stats_cb), opaque, opts_p) };

	str.truncate(MAX_LENGTH);
	Some(format!("<pre><code>{str}</code></pre>"))
}

extern "C" fn malloc_stats_cb(opaque: *mut c_void, msg: *const c_char) {
	// SAFETY: we have to trust the opaque points to our String
	let res: &mut String = unsafe { opaque.cast::<String>().as_mut().unwrap() };

	// SAFETY: we have to trust the string is null terminated.
	let msg = unsafe { std::ffi::CStr::from_ptr(msg) };

	let msg = String::from_utf8_lossy(msg.to_bytes());
	res.push_str(msg.as_ref());
}
