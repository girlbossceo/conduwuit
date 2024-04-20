use ruma::{events::room::message::RoomMessageEventContent, EventId};
use tracing::{debug, info};

use crate::{service::admin::MxcUri, services, Result};

pub(super) async fn delete(
	_body: Vec<&str>, mxc: Option<Box<MxcUri>>, event_id: Option<Box<EventId>>,
) -> Result<RoomMessageEventContent> {
	if event_id.is_some() && mxc.is_some() {
		return Ok(RoomMessageEventContent::text_plain(
			"Please specify either an MXC or an event ID, not both.",
		));
	}

	if let Some(mxc) = mxc {
		debug!("Got MXC URL: {mxc}");
		services().media.delete(mxc.to_string()).await?;

		return Ok(RoomMessageEventContent::text_plain(
			"Deleted the MXC from our database and on our filesystem.",
		));
	} else if let Some(event_id) = event_id {
		debug!("Got event ID to delete media from: {event_id}");

		let mut mxc_urls = vec![];
		let mut mxc_deletion_count = 0;

		// parsing the PDU for any MXC URLs begins here
		if let Some(event_json) = services().rooms.timeline.get_pdu_json(&event_id)? {
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
			services().media.delete(mxc_url).await?;
			mxc_deletion_count += 1;
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

pub(super) async fn delete_list(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let mxc_list = body.clone().drain(1..body.len() - 1).collect::<Vec<_>>();

		let mut mxc_deletion_count = 0;

		for mxc in mxc_list {
			debug!("Deleting MXC {mxc} in bulk");
			services().media.delete(mxc.to_owned()).await?;
			mxc_deletion_count += 1;
		}

		return Ok(RoomMessageEventContent::text_plain(format!(
			"Finished bulk MXC deletion, deleted {mxc_deletion_count} total MXCs from our database and the filesystem.",
		)));
	}

	Ok(RoomMessageEventContent::text_plain(
		"Expected code block in command body. Add --help for details.",
	))
}

pub(super) async fn delete_past_remote_media(_body: Vec<&str>, duration: String) -> Result<RoomMessageEventContent> {
	let deleted_count = services()
		.media
		.delete_all_remote_media_at_after_time(duration)
		.await?;

	Ok(RoomMessageEventContent::text_plain(format!(
		"Deleted {deleted_count} total files.",
	)))
}
