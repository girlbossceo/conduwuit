use conduwuit::utils::string::EMPTY;
use rocksdb::{
	DBCompactionPri as CompactionPri, DBCompactionStyle as CompactionStyle,
	DBCompressionType as CompressionType,
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum CacheDisp {
	Unique,
	Shared,
	SharedWith(&'static str),
}

#[derive(Debug, Clone)]
pub(crate) struct Descriptor {
	pub(crate) name: &'static str,
	pub(crate) dropped: bool,
	pub(crate) cache_disp: CacheDisp,
	pub(crate) key_size_hint: Option<usize>,
	pub(crate) val_size_hint: Option<usize>,
	pub(crate) block_size: usize,
	pub(crate) index_size: usize,
	pub(crate) write_size: usize,
	pub(crate) cache_size: usize,
	pub(crate) level_size: u64,
	pub(crate) level_shape: [i32; 7],
	pub(crate) file_size: u64,
	pub(crate) file_shape: [i32; 1],
	pub(crate) level0_width: i32,
	pub(crate) merge_width: (i32, i32),
	pub(crate) ttl: u64,
	pub(crate) compaction: CompactionStyle,
	pub(crate) compaction_pri: CompactionPri,
	pub(crate) compression: CompressionType,
	pub(crate) compression_level: i32,
	pub(crate) bottommost_level: Option<i32>,
	pub(crate) block_index_hashing: bool,
	pub(crate) cache_shards: u32,
}

pub(crate) static BASE: Descriptor = Descriptor {
	name: EMPTY,
	dropped: false,
	cache_disp: CacheDisp::Shared,
	key_size_hint: None,
	val_size_hint: None,
	block_size: 1024 * 4,
	index_size: 1024 * 4,
	write_size: 1024 * 1024 * 2,
	cache_size: 1024 * 1024 * 4,
	level_size: 1024 * 1024 * 8,
	level_shape: [1, 1, 1, 3, 7, 15, 31],
	file_size: 1024 * 1024,
	file_shape: [2],
	level0_width: 2,
	merge_width: (2, 16),
	ttl: 60 * 60 * 24 * 21,
	compaction: CompactionStyle::Level,
	compaction_pri: CompactionPri::MinOverlappingRatio,
	compression: CompressionType::Zstd,
	compression_level: 32767,
	bottommost_level: Some(32767),
	block_index_hashing: false,
	cache_shards: 64,
};

pub(crate) static RANDOM: Descriptor = Descriptor {
	compaction_pri: CompactionPri::OldestSmallestSeqFirst,
	write_size: 1024 * 1024 * 32,
	..BASE
};

pub(crate) static SEQUENTIAL: Descriptor = Descriptor {
	compaction_pri: CompactionPri::OldestLargestSeqFirst,
	write_size: 1024 * 1024 * 64,
	level_size: 1024 * 1024 * 32,
	file_size: 1024 * 1024 * 2,
	..BASE
};

pub(crate) static RANDOM_SMALL: Descriptor = Descriptor {
	compaction: CompactionStyle::Universal,
	write_size: 1024 * 1024 * 16,
	level_size: 1024 * 512,
	file_size: 1024 * 128,
	index_size: 512,
	block_size: 512,
	cache_shards: 64,
	..RANDOM
};

pub(crate) static SEQUENTIAL_SMALL: Descriptor = Descriptor {
	compaction: CompactionStyle::Universal,
	write_size: 1024 * 1024 * 16,
	level_size: 1024 * 1024,
	file_size: 1024 * 512,
	block_size: 512,
	cache_shards: 64,
	..SEQUENTIAL
};
