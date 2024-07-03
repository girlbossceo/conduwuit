use conduit::{warn, Error, Result};
use ruma::events::room::message::RoomMessageEventContent;

use crate::services;

pub(super) async fn uptime(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let seconds = services()
		.server
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

	Ok(RoomMessageEventContent::notice_plain(result))
}

pub(super) async fn show_config(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	// Construct and send the response
	Ok(RoomMessageEventContent::text_plain(format!("{}", services().globals.config)))
}

pub(super) async fn memory_usage(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let response0 = services().memory_usage().await;
	let response1 = services().globals.db.memory_usage();
	let response2 = conduit::alloc::memory_usage();

	Ok(RoomMessageEventContent::text_plain(format!(
		"Services:\n{response0}\n\nDatabase:\n{response1}\n{}",
		if !response2.is_empty() {
			format!("Allocator:\n {response2}")
		} else {
			String::new()
		}
	)))
}

pub(super) async fn clear_database_caches(_body: Vec<&str>, amount: u32) -> Result<RoomMessageEventContent> {
	services().globals.db.clear_caches(amount);

	Ok(RoomMessageEventContent::text_plain("Done."))
}

pub(super) async fn clear_service_caches(_body: Vec<&str>, amount: u32) -> Result<RoomMessageEventContent> {
	services().clear_caches(amount).await;

	Ok(RoomMessageEventContent::text_plain("Done."))
}

pub(super) async fn list_backups(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let result = services().globals.db.backup_list()?;

	if result.is_empty() {
		Ok(RoomMessageEventContent::text_plain("No backups found."))
	} else {
		Ok(RoomMessageEventContent::text_plain(result))
	}
}

pub(super) async fn backup_database(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let mut result = services()
		.server
		.runtime()
		.spawn_blocking(move || match services().globals.db.backup() {
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

pub(super) async fn list_database_files(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let result = services().globals.db.file_list()?;

	Ok(RoomMessageEventContent::notice_markdown(result))
}

pub(super) async fn admin_notice(_body: Vec<&str>, message: Vec<String>) -> Result<RoomMessageEventContent> {
	let message = message.join(" ");
	services().admin.send_text(&message).await;

	Ok(RoomMessageEventContent::notice_plain("Notice was sent to #admins"))
}

#[cfg(conduit_mods)]
pub(super) async fn reload(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	services().server.reload()?;

	Ok(RoomMessageEventContent::notice_plain("Reloading server..."))
}

#[cfg(unix)]
pub(super) async fn restart(_body: Vec<&str>, force: bool) -> Result<RoomMessageEventContent> {
	use conduit::utils::sys::current_exe_deleted;

	if !force && current_exe_deleted() {
		return Err(Error::Err(
			"The server cannot be restarted because the executable was tampered with. If this is expected use --force \
			 to override."
				.to_owned(),
		));
	}

	services().server.restart()?;

	Ok(RoomMessageEventContent::notice_plain("Restarting server..."))
}

pub(super) async fn shutdown(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	warn!("shutdown command");
	services().server.shutdown()?;

	Ok(RoomMessageEventContent::notice_plain("Shutting down server..."))
}
