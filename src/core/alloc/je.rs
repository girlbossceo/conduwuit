//! jemalloc allocator

use std::{
	cell::OnceCell,
	ffi::{c_char, c_void},
	fmt::{Debug, Write},
};

use arrayvec::ArrayVec;
use tikv_jemalloc_ctl as mallctl;
use tikv_jemalloc_sys as ffi;
use tikv_jemallocator as jemalloc;

use crate::{err, is_equal_to, utils::math::Tried, Result};

#[cfg(feature = "jemalloc_conf")]
#[no_mangle]
pub static malloc_conf: &[u8] = b"\
metadata_thp:always\
,percpu_arena:percpu\
,background_thread:true\
,max_background_threads:-1\
,lg_extent_max_active_fit:4\
,oversize_threshold:33554432\
,tcache_max:2097152\
,dirty_decay_ms:16000\
,muzzy_decay_ms:144000\
\0";

#[global_allocator]
static JEMALLOC: jemalloc::Jemalloc = jemalloc::Jemalloc;

type Key = ArrayVec<usize, KEY_SEGS>;
type Name = ArrayVec<u8, NAME_MAX>;

const KEY_SEGS: usize = 8;
const NAME_MAX: usize = 128;

#[must_use]
#[cfg(feature = "jemalloc_stats")]
pub fn memory_usage() -> Option<String> {
	use mallctl::stats;

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
		"allocated: {allocated:.2} MiB\nactive: {active:.2} MiB\nmapped: {mapped:.2} \
		 MiB\nmetadata: {metadata:.2} MiB\nresident: {resident:.2} MiB\nretained: {retained:.2} \
		 MiB\n"
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

unsafe extern "C" fn malloc_stats_cb(opaque: *mut c_void, msg: *const c_char) {
	// SAFETY: we have to trust the opaque points to our String
	let res: &mut String = unsafe {
		opaque
			.cast::<String>()
			.as_mut()
			.expect("failed to cast void* to &mut String")
	};

	// SAFETY: we have to trust the string is null terminated.
	let msg = unsafe { std::ffi::CStr::from_ptr(msg) };

	let msg = String::from_utf8_lossy(msg.to_bytes());
	res.push_str(msg.as_ref());
}

macro_rules! mallctl {
	($name:literal) => {{
		thread_local! {
			static KEY: OnceCell<Key> = OnceCell::default();
		};

		KEY.with(|once| {
			once.get_or_init(move || key($name).expect("failed to translate name into mib key"))
				.clone()
		})
	}};
}

pub fn trim() -> Result { set(&mallctl!("arena.4096.purge"), ()) }

pub fn decay() -> Result { set(&mallctl!("arena.4096.purge"), ()) }

pub fn set_by_name<T: Copy + Debug>(name: &str, val: T) -> Result { set(&key(name)?, val) }

pub fn get_by_name<T: Copy + Debug>(name: &str) -> Result<T> { get(&key(name)?) }

pub mod this_thread {
	use super::{get, key, set, Key, OnceCell, Result};

	pub fn trim() -> Result {
		let mut key = mallctl!("arena.0.purge");
		key[1] = arena_id()?.try_into()?;
		set(&key, ())
	}

	pub fn decay() -> Result {
		let mut key = mallctl!("arena.0.decay");
		key[1] = arena_id()?.try_into()?;
		set(&key, ())
	}

	pub fn cache(enable: bool) -> Result {
		set(&mallctl!("thread.tcache.enabled"), u8::from(enable))
	}

	pub fn flush() -> Result { set(&mallctl!("thread.tcache.flush"), ()) }

	pub fn allocated() -> Result<u64> { get::<u64>(&mallctl!("thread.allocated")) }

	pub fn deallocated() -> Result<u64> { get::<u64>(&mallctl!("thread.deallocated")) }

	pub fn arena_id() -> Result<u32> { get::<u32>(&mallctl!("thread.arena")) }
}

fn set<T>(key: &Key, val: T) -> Result
where
	T: Copy + Debug,
{
	// SAFETY: T must be the exact expected type.
	unsafe { mallctl::raw::write_mib(key.as_slice(), val) }.map_err(map_err)
}

fn get<T>(key: &Key) -> Result<T>
where
	T: Copy + Debug,
{
	// SAFETY: T must be perfectly valid to receive value.
	unsafe { mallctl::raw::read_mib(key.as_slice()) }.map_err(map_err)
}

fn key(name: &str) -> Result<Key> {
	// tikv asserts the output buffer length is tight to the number of required mibs
	// so we slice that down here.
	let segs = name.chars().filter(is_equal_to!(&'.')).count().try_add(1)?;

	let name = self::name(name)?;
	let mut buf = [0_usize; KEY_SEGS];
	mallctl::raw::name_to_mib(name.as_slice(), &mut buf[0..segs])
		.map_err(map_err)
		.map(move |()| buf.into_iter().take(segs).collect())
}

fn name(name: &str) -> Result<Name> {
	let mut buf = Name::new();
	buf.try_extend_from_slice(name.as_bytes())?;
	buf.try_extend_from_slice(b"\0")?;

	Ok(buf)
}

fn map_err(error: tikv_jemalloc_ctl::Error) -> crate::Error {
	err!("mallctl: {}", error.to_string())
}
