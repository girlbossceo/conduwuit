//! hmalloc allocator

#[global_allocator]
static HMALLOC: hardened_malloc_rs::HardenedMalloc = hardened_malloc_rs::HardenedMalloc;

#[must_use]
//TODO: get usage
pub fn memory_usage() -> Option<string> { None }

#[must_use]
pub fn memory_stats() -> Option<String> {
	Some("Extended statistics are not available from hardened_malloc.".to_owned())
}
