use conduit::{debug, info, Result};
use ruma::{events::room::message::RoomMessageEventContent, EventId, MxcUri};

use crate::admin_command;

#[admin_command]
pub(super) async fn delete(
	&self, mxc: Option<Box<MxcUri>>, event_id: Option<Box<EventId>>,
) -> Result<RoomMessageEventContent> {
	if event_id.is_some() && mxc.is_some() {
		return Ok(RoomMessageEventContent::text_plain(
			"Please specify either an MXC or an event ID, not both.",
		));
	}

	if let Some(mxc) = mxc {
		debug!("Got MXC URL: {mxc}");
		self.services.media.delete(mxc.as_ref()).await?;

		return Ok(RoomMessageEventContent::text_plain(
			"Deleted the MXC from our database and on our filesystem.",
		));
	} else if let Some(event_id) = event_id {
		debug!("Got event ID to delete media from: {event_id}");

		let mut mxc_urls = vec![];
		let mut mxc_deletion_count: usize = 0;

		// parsing the PDU for any MXC URLs begins here
		if let Some(event_json) = self.services.rooms.timeline.get_pdu_json(&event_id)? {
			if let Some(content_key) = event_json.get("content") {
				debug!("Event ID has \"content\".");
				let content_obj = content_key.as_object();

				if let Some(content) = content_obj {
					// 1. attempts to parse the "url" key
					debug!("Attempting to go into \"url\" key for main media file");
					if let Some(url) = content.get("url") {
						debug!("Got a URL in the event ID {event_id}: {url}");

						if url.to_string().starts_with("\"mxc://") {
							debug!("Pushing URL {url} to list of MXCs to delete");
							let final_url = url.to_string().replace('"', "");
							mxc_urls.push(final_url);
						} else {
							info!("Found a URL in the event ID {event_id} but did not start with mxc://, ignoring");
						}
					}

					// 2. attempts to parse the "info" key
					debug!("Attempting to go into \"info\" key for thumbnails");
					if let Some(info_key) = content.get("info") {
						debug!("Event ID has \"info\".");
						let info_obj = info_key.as_object();

						if let Some(info) = info_obj {
							if let Some(thumbnail_url) = info.get("thumbnail_url") {
								debug!("Found a thumbnail_url in info key: {thumbnail_url}");

								if thumbnail_url.to_string().starts_with("\"mxc://") {
									debug!("Pushing thumbnail URL {thumbnail_url} to list of MXCs to delete");
									let final_thumbnail_url = thumbnail_url.to_string().replace('"', "");
									mxc_urls.push(final_thumbnail_url);
								} else {
									info!(
										"Found a thumbnail URL in the event ID {event_id} but did not start with \
										 mxc://, ignoring"
									);
								}
							} else {
								info!("No \"thumbnail_url\" key in \"info\" key, assuming no thumbnails.");
							}
						}
					}

					// 3. attempts to parse the "file" key
					debug!("Attempting to go into \"file\" key");
					if let Some(file_key) = content.get("file") {
						debug!("Event ID has \"file\".");
						let file_obj = file_key.as_object();

						if let Some(file) = file_obj {
							if let Some(url) = file.get("url") {
								debug!("Found url in file key: {url}");

								if url.to_string().starts_with("\"mxc://") {
									debug!("Pushing URL {url} to list of MXCs to delete");
									let final_url = url.to_string().replace('"', "");
									mxc_urls.push(final_url);
								} else {
									info!(
										"Found a URL in the event ID {event_id} but did not start with mxc://, \
										 ignoring"
									);
								}
							} else {
								info!("No \"url\" key in \"file\" key.");
							}
						}
					}
				} else {
					return Ok(RoomMessageEventContent::text_plain(
						"Event ID does not have a \"content\" key or failed parsing the event ID JSON.",
					));
				}
			} else {
				return Ok(RoomMessageEventContent::text_plain(
					"Event ID does not have a \"content\" key, this is not a message or an event type that contains \
					 media.",
				));
			}
		} else {
			return Ok(RoomMessageEventContent::text_plain(
				"Event ID does not exist or is not known to us.",
			));
		}

		if mxc_urls.is_empty() {
			// we shouldn't get here (should have errored earlier) but just in case for
			// whatever reason we do...
			info!("Parsed event ID {event_id} but did not contain any MXC URLs.");
			return Ok(RoomMessageEventContent::text_plain("Parsed event ID but found no MXC URLs."));
		}

		for mxc_url in mxc_urls {
			self.services.media.delete(&mxc_url).await?;
			mxc_deletion_count = mxc_deletion_count.saturating_add(1);
		}

		return Ok(RoomMessageEventContent::text_plain(format!(
			"Deleted {mxc_deletion_count} total MXCs from our database and the filesystem from event ID {event_id}."
		)));
	}

	Ok(RoomMessageEventContent::text_plain(
		"Please specify either an MXC using --mxc or an event ID using --event-id of the message containing an image. \
		 See --help for details.",
	))
}

#[admin_command]
pub(super) async fn delete_list(&self) -> Result<RoomMessageEventContent> {
	if self.body.len() < 2 || !self.body[0].trim().starts_with("```") || self.body.last().unwrap_or(&"").trim() != "```"
	{
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

	let mxc_list = self
		.body
		.to_vec()
		.drain(1..self.body.len().checked_sub(1).unwrap())
		.collect::<Vec<_>>();

	let mut mxc_deletion_count: usize = 0;

	for mxc in mxc_list {
		debug!("Deleting MXC {mxc} in bulk");
		self.services.media.delete(mxc).await?;
		mxc_deletion_count = mxc_deletion_count
			.checked_add(1)
			.expect("mxc_deletion_count should not get this high");
	}

	Ok(RoomMessageEventContent::text_plain(format!(
		"Finished bulk MXC deletion, deleted {mxc_deletion_count} total MXCs from our database and the filesystem.",
	)))
}

#[admin_command]
pub(super) async fn delete_past_remote_media(&self, duration: String, force: bool) -> Result<RoomMessageEventContent> {
	let deleted_count = self
		.services
		.media
		.delete_all_remote_media_at_after_time(duration, force)
		.await?;

	Ok(RoomMessageEventContent::text_plain(format!(
		"Deleted {deleted_count} total files.",
	)))
}
