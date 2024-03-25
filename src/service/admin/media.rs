use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, EventId};
use tracing::{debug, info};

use crate::{service::admin::MxcUri, services, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum MediaCommand {
	/// - Deletes a single media file from our database and on the filesystem
	///   via a single MXC URL
	Delete {
		/// The MXC URL to delete
		#[arg(long)]
		mxc: Option<Box<MxcUri>>,

		/// - The message event ID which contains the media and thumbnail MXC
		///   URLs
		#[arg(long)]
		event_id: Option<Box<EventId>>,
	},

	/// - Deletes a codeblock list of MXC URLs from our database and on the
	///   filesystem
	DeleteList,

	/// - Deletes all remote media in the last X amount of time using filesystem
	///   metadata first created at date.
	DeletePastRemoteMedia {
		/// - The duration (at or after), e.g. "5m" to delete all media in the
		///   past 5 minutes
		duration: String,
	},
}

pub(crate) async fn process(command: MediaCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		MediaCommand::Delete {
			mxc,
			event_id,
		} => {
			if event_id.is_some() && mxc.is_some() {
				return Ok(RoomMessageEventContent::text_plain(
					"Please specify either an MXC or an event ID, not both.",
				));
			}

			if let Some(mxc) = mxc {
				if !mxc.to_string().starts_with("mxc://") {
					return Ok(RoomMessageEventContent::text_plain("MXC provided is not valid."));
				}

				debug!("Got MXC URL: {}", mxc);
				services().media.delete(mxc.to_string()).await?;

				return Ok(RoomMessageEventContent::text_plain(
					"Deleted the MXC from our database and on our filesystem.",
				));
			} else if let Some(event_id) = event_id {
				debug!("Got event ID to delete media from: {}", event_id);

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
									debug!("Pushing URL {} to list of MXCs to delete", url);
									let final_url = url.to_string().replace('"', "");
									mxc_urls.push(final_url);
								} else {
									info!(
										"Found a URL in the event ID {event_id} but did not start with mxc://, \
										 ignoring"
									);
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
											debug!("Pushing thumbnail URL {} to list of MXCs to delete", thumbnail_url);
											let final_thumbnail_url = thumbnail_url.to_string().replace('"', "");
											mxc_urls.push(final_thumbnail_url);
										} else {
											info!(
												"Found a thumbnail URL in the event ID {event_id} but did not start \
												 with mxc://, ignoring"
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
											debug!("Pushing URL {} to list of MXCs to delete", url);
											let final_url = url.to_string().replace('"', "");
											mxc_urls.push(final_url);
										} else {
											info!(
												"Found a URL in the event ID {event_id} but did not start with \
												 mxc://, ignoring"
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
							"Event ID does not have a \"content\" key, this is not a message or an event type that \
							 contains media.",
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
					"Deleted {mxc_deletion_count} total MXCs from our database and the filesystem from event ID \
					 {event_id}."
				)));
			}

			Ok(RoomMessageEventContent::text_plain(
				"Please specify either an MXC using --mxc or an event ID using --event-id of the message containing \
				 an image. See --help for details.",
			))
		},
		MediaCommand::DeleteList => {
			if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
				let mxc_list = body.clone().drain(1..body.len() - 1).collect::<Vec<_>>();

				let mut mxc_deletion_count = 0;

				for mxc in mxc_list {
					debug!("Deleting MXC {} in bulk", mxc);
					services().media.delete(mxc.to_owned()).await?;
					mxc_deletion_count += 1;
				}

				return Ok(RoomMessageEventContent::text_plain(format!(
					"Finished bulk MXC deletion, deleted {} total MXCs from our database and the filesystem.",
					mxc_deletion_count
				)));
			}

			Ok(RoomMessageEventContent::text_plain(
				"Expected code block in command body. Add --help for details.",
			))
		},
		MediaCommand::DeletePastRemoteMedia {
			duration,
		} => {
			let deleted_count = services()
				.media
				.delete_all_remote_media_at_after_time(duration)
				.await?;

			Ok(RoomMessageEventContent::text_plain(format!(
				"Deleted {} total files.",
				deleted_count
			)))
		},
	}
}
