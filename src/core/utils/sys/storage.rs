//! System utilities related to devices/peripherals

use std::{
	ffi::{OsStr, OsString},
	fs,
	fs::{read_to_string, FileType},
	iter::IntoIterator,
	path::Path,
};

use crate::{result::FlatOk, Result};

/// Device characteristics useful for random access throughput
#[derive(Clone, Debug, Default)]
pub struct Parallelism {
	/// Number of requests for the device.
	pub nr_requests: Option<usize>,

	/// Individual queue characteristics.
	pub mq: Vec<Queue>,
}

/// Device queue characteristics
#[derive(Clone, Debug, Default)]
pub struct Queue {
	/// Queue's indice.
	pub id: usize,

	/// Number of requests for the queue.
	pub nr_tags: Option<usize>,

	/// CPU affinities for the queue.
	pub cpu_list: Vec<usize>,
}

/// Get device characteristics useful for random access throughput by name.
#[must_use]
pub fn parallelism(name: &OsStr) -> Parallelism {
	let name = name
		.to_str()
		.expect("device name expected to be utf-8 representable");

	let block_path = Path::new("/").join("sys/").join("block/");

	let mq_path = Path::new(&block_path).join(format!("{name}/mq/"));

	let nr_requests_path = Path::new(&block_path).join(format!("{name}/queue/nr_requests"));

	Parallelism {
		nr_requests: read_to_string(&nr_requests_path)
			.ok()
			.as_deref()
			.map(str::trim)
			.map(str::parse)
			.flat_ok(),

		mq: fs::read_dir(&mq_path)
			.into_iter()
			.flat_map(IntoIterator::into_iter)
			.filter_map(Result::ok)
			.filter(|entry| entry.file_type().as_ref().is_ok_and(FileType::is_dir))
			.map(|dir| queue_parallelism(&dir.path()))
			.collect(),
	}
}

/// Get device queue characteristics by mq path on sysfs(5)
fn queue_parallelism(dir: &Path) -> Queue {
	let queue_id = dir.file_name();

	let nr_tags_path = dir.join("nr_tags");

	let cpu_list_path = dir.join("cpu_list");

	Queue {
		id: queue_id
			.and_then(OsStr::to_str)
			.map(str::parse)
			.flat_ok()
			.expect("queue has some numerical identifier"),

		nr_tags: read_to_string(&nr_tags_path)
			.ok()
			.as_deref()
			.map(str::trim)
			.map(str::parse)
			.flat_ok(),

		cpu_list: read_to_string(&cpu_list_path)
			.iter()
			.flat_map(|list| list.trim().split(','))
			.map(str::trim)
			.map(str::parse)
			.filter_map(Result::ok)
			.collect(),
	}
}

/// Get the name of the device on which Path is mounted.
#[must_use]
pub fn name_from_path(path: &Path) -> Option<OsString> {
	sysinfo::Disks::new_with_refreshed_list()
		.into_iter()
		.filter(|disk| path.starts_with(disk.mount_point()))
		.max_by(|a, b| {
			let a = a.mount_point().ancestors().count();
			let b = b.mount_point().ancestors().count();
			a.cmp(&b)
		})
		.map(|disk| Path::new(disk.name()))
		.and_then(|path| path.file_name().map(ToOwned::to_owned))
}
