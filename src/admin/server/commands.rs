use std::fmt::Write;

use conduit::{info, utils::time, warn, Err, Result};
use ruma::events::room::message::RoomMessageEventContent;

use crate::services;

pub(super) async fn uptime(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let elapsed = services()
		.server
		.started
		.elapsed()
		.expect("standard duration");

	let result = time::pretty(elapsed);
	Ok(RoomMessageEventContent::notice_plain(format!("{result}.")))
}

pub(super) async fn show_config(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	// Construct and send the response
	Ok(RoomMessageEventContent::text_plain(format!("{}", services().globals.config)))
}

pub(super) async fn list_features(
	_body: Vec<&str>, available: bool, enabled: bool, comma: bool,
) -> Result<RoomMessageEventContent> {
	let delim = if comma {
		","
	} else {
		" "
	};
	if enabled && !available {
		let features = info::rustc::features().join(delim);
		let out = format!("```\n{features}\n```");
		return Ok(RoomMessageEventContent::text_markdown(out));
	}

	if available && !enabled {
		let features = info::cargo::features().join(delim);
		let out = format!("```\n{features}\n```");
		return Ok(RoomMessageEventContent::text_markdown(out));
	}

	let mut features = String::new();
	let enabled = info::rustc::features();
	let available = info::cargo::features();
	for feature in available {
		let active = enabled.contains(&feature.as_str());
		let emoji = if active {
			"✅"
		} else {
			"❌"
		};
		let remark = if active {
			"[enabled]"
		} else {
			""
		};
		writeln!(features, "{emoji} {feature} {remark}")?;
	}

	Ok(RoomMessageEventContent::text_markdown(features))
}

pub(super) async fn memory_usage(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let services_usage = services().memory_usage().await?;
	let database_usage = services().db.db.memory_usage()?;
	let allocator_usage = conduit::alloc::memory_usage().map_or(String::new(), |s| format!("\nAllocator:\n{s}"));

	Ok(RoomMessageEventContent::text_plain(format!(
		"Services:\n{services_usage}\nDatabase:\n{database_usage}{allocator_usage}",
	)))
}

pub(super) async fn clear_caches(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	services().clear_cache().await;

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
		return Err!(
			"The server cannot be restarted because the executable changed. If this is expected use --force to \
			 override."
		);
	}

	services().server.restart()?;

	Ok(RoomMessageEventContent::notice_plain("Restarting server..."))
}

pub(super) async fn shutdown(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	warn!("shutdown command");
	services().server.shutdown()?;

	Ok(RoomMessageEventContent::notice_plain("Shutting down server..."))
}
