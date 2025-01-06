use rocksdb::{ReadOptions, ReadTier, WriteOptions};

#[inline]
pub(crate) fn iter_options_default() -> ReadOptions {
	let mut read_options = read_options_default();
	read_options.set_background_purge_on_iterator_cleanup(true);
	//read_options.set_pin_data(true);
	read_options
}

#[inline]
pub(crate) fn cache_read_options_default() -> ReadOptions {
	let mut read_options = read_options_default();
	read_options.set_read_tier(ReadTier::BlockCache);
	read_options
}

#[inline]
pub(crate) fn read_options_default() -> ReadOptions {
	let mut read_options = ReadOptions::default();
	read_options.set_total_order_seek(true);
	read_options
}

#[inline]
pub(crate) fn write_options_default() -> WriteOptions { WriteOptions::default() }
