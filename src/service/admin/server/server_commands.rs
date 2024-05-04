use ruma::events::room::message::RoomMessageEventContent;

use crate::{services, Result};

pub(crate) async fn uptime(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let seconds = services()
		.globals
		.started
		.elapsed()
		.expect("standard duration")
		.as_secs();
	let result = format!(
		"up {} days, {} hours, {} minutes, {} seconds.",
		seconds / 86400,
		(seconds % 86400) / 60 / 60,
		(seconds % 3600) / 60,
		seconds % 60,
	);

	Ok(RoomMessageEventContent::notice_html(String::new(), result))
}

pub(crate) async fn show_config(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	// Construct and send the response
	Ok(RoomMessageEventContent::text_plain(format!("{}", services().globals.config)))
}

pub(crate) async fn memory_usage(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let response0 = services().memory_usage().await;
	let response1 = services().globals.db.memory_usage();
	let response2 = crate::alloc::memory_usage();

	Ok(RoomMessageEventContent::text_plain(format!(
		"Services:\n{response0}\n\nDatabase:\n{response1}\n{}",
		if !response2.is_empty() {
			format!("Allocator:\n {response2}")
		} else {
			String::new()
		}
	)))
}

pub(crate) async fn clear_database_caches(_body: Vec<&str>, amount: u32) -> Result<RoomMessageEventContent> {
	services().globals.db.clear_caches(amount);

	Ok(RoomMessageEventContent::text_plain("Done."))
}

pub(crate) async fn clear_service_caches(_body: Vec<&str>, amount: u32) -> Result<RoomMessageEventContent> {
	services().clear_caches(amount).await;

	Ok(RoomMessageEventContent::text_plain("Done."))
}

pub(crate) async fn list_backups(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let result = services().globals.db.backup_list()?;

	if result.is_empty() {
		Ok(RoomMessageEventContent::text_plain("No backups found."))
	} else {
		Ok(RoomMessageEventContent::text_plain(result))
	}
}

pub(crate) async fn backup_database(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if !cfg!(feature = "rocksdb") {
		return Ok(RoomMessageEventContent::text_plain(
			"Only RocksDB supports online backups in conduwuit.",
		));
	}

	let mut result = tokio::task::spawn_blocking(move || match services().globals.db.backup() {
		Ok(()) => String::new(),
		Err(e) => (*e).to_string(),
	})
	.await
	.unwrap();

	if result.is_empty() {
		result = services().globals.db.backup_list()?;
	}

	Ok(RoomMessageEventContent::text_plain(&result))
}

pub(crate) async fn list_database_files(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if !cfg!(feature = "rocksdb") {
		return Ok(RoomMessageEventContent::text_plain(
			"Only RocksDB supports listing files in conduwuit.",
		));
	}

	let result = services().globals.db.file_list()?;
	Ok(RoomMessageEventContent::notice_html(String::new(), result))
}
