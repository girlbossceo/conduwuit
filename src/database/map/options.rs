use rocksdb::{ReadOptions, ReadTier, WriteOptions};

#[inline]
pub(crate) fn iter_options_default() -> ReadOptions {
	let mut options = read_options_default();
	options.set_background_purge_on_iterator_cleanup(true);
	//options.set_pin_data(true);
	options
}

#[inline]
pub(crate) fn cache_iter_options_default() -> ReadOptions {
	let mut options = cache_read_options_default();
	options.set_background_purge_on_iterator_cleanup(true);
	//options.set_pin_data(true);
	options
}

#[inline]
pub(crate) fn cache_read_options_default() -> ReadOptions {
	let mut options = read_options_default();
	options.set_read_tier(ReadTier::BlockCache);
	options.fill_cache(false);
	options
}

#[inline]
pub(crate) fn read_options_default() -> ReadOptions {
	let mut options = ReadOptions::default();
	options.set_total_order_seek(true);
	options
}

#[inline]
pub(crate) fn write_options_default() -> WriteOptions { WriteOptions::default() }
