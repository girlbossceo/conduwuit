use std::sync::Arc;

use conduit::{debug, debug_info, utils::string_from_bytes, Error, Result};
use database::{Database, Map};
use ruma::api::client::error::ErrorKind;

use super::preview::UrlPreviewData;

pub(crate) struct Data {
	mediaid_file: Arc<Map>,
	mediaid_user: Arc<Map>,
	url_previews: Arc<Map>,
}

#[derive(Debug)]
pub(super) struct Metadata {
	pub(super) content_disposition: Option<String>,
	pub(super) content_type: Option<String>,
	pub(super) key: Vec<u8>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			mediaid_file: db["mediaid_file"].clone(),
			mediaid_user: db["mediaid_user"].clone(),
			url_previews: db["url_previews"].clone(),
		}
	}

	pub(super) fn create_file_metadata(
		&self, sender_user: Option<&str>, mxc: &str, width: u32, height: u32, content_disposition: Option<&str>,
		content_type: Option<&str>,
	) -> Result<Vec<u8>> {
		let mut key = mxc.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(&width.to_be_bytes());
		key.extend_from_slice(&height.to_be_bytes());
		key.push(0xFF);
		key.extend_from_slice(
			content_disposition
				.as_ref()
				.map(|f| f.as_bytes())
				.unwrap_or_default(),
		);
		key.push(0xFF);
		key.extend_from_slice(
			content_type
				.as_ref()
				.map(|c| c.as_bytes())
				.unwrap_or_default(),
		);

		self.mediaid_file.insert(&key, &[])?;

		if let Some(user) = sender_user {
			let key = mxc.as_bytes().to_vec();
			let user = user.as_bytes().to_vec();
			self.mediaid_user.insert(&key, &user)?;
		}

		Ok(key)
	}

	pub(super) fn delete_file_mxc(&self, mxc: &str) -> Result<()> {
		debug!("MXC URI: {:?}", mxc);

		let mut prefix = mxc.as_bytes().to_vec();
		prefix.push(0xFF);

		debug!("MXC db prefix: {prefix:?}");

		for (key, _) in self.mediaid_file.scan_prefix(prefix) {
			debug!("Deleting key: {:?}", key);
			self.mediaid_file.remove(&key)?;
		}

		for (key, value) in self.mediaid_user.scan_prefix(mxc.as_bytes().to_vec()) {
			if key == mxc.as_bytes().to_vec() {
				let user = string_from_bytes(&value).unwrap_or_default();

				debug_info!("Deleting key \"{key:?}\" which was uploaded by user {user}");
				self.mediaid_user.remove(&key)?;
			}
		}

		Ok(())
	}

	/// Searches for all files with the given MXC
	pub(super) fn search_mxc_metadata_prefix(&self, mxc: &str) -> Result<Vec<Vec<u8>>> {
		debug!("MXC URI: {:?}", mxc);

		let mut prefix = mxc.as_bytes().to_vec();
		prefix.push(0xFF);

		let keys: Vec<Vec<u8>> = self
			.mediaid_file
			.scan_prefix(prefix)
			.map(|(key, _)| key)
			.collect();

		if keys.is_empty() {
			return Err(Error::bad_database(
				"Failed to find any keys in database with the provided MXC.",
			));
		}

		debug!("Got the following keys: {:?}", keys);

		Ok(keys)
	}

	pub(super) fn search_file_metadata(&self, mxc: &str, width: u32, height: u32) -> Result<Metadata> {
		let mut prefix = mxc.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(&width.to_be_bytes());
		prefix.extend_from_slice(&height.to_be_bytes());
		prefix.push(0xFF);

		let (key, _) = self
			.mediaid_file
			.scan_prefix(prefix)
			.next()
			.ok_or_else(|| Error::BadRequest(ErrorKind::NotFound, "Media not found"))?;

		let mut parts = key.rsplit(|&b| b == 0xFF);

		let content_type = parts
			.next()
			.map(|bytes| {
				string_from_bytes(bytes)
					.map_err(|_| Error::bad_database("Content type in mediaid_file is invalid unicode."))
			})
			.transpose()?;

		let content_disposition_bytes = parts
			.next()
			.ok_or_else(|| Error::bad_database("Media ID in db is invalid."))?;

		let content_disposition = if content_disposition_bytes.is_empty() {
			None
		} else {
			Some(
				string_from_bytes(content_disposition_bytes)
					.map_err(|_| Error::bad_database("Content Disposition in mediaid_file is invalid unicode."))?,
			)
		};

		Ok(Metadata {
			content_disposition,
			content_type,
			key,
		})
	}

	/// Gets all the media keys in our database (this includes all the metadata
	/// associated with it such as width, height, content-type, etc)
	pub(crate) fn get_all_media_keys(&self) -> Vec<Vec<u8>> { self.mediaid_file.iter().map(|(key, _)| key).collect() }

	#[inline]
	pub(super) fn remove_url_preview(&self, url: &str) -> Result<()> { self.url_previews.remove(url.as_bytes()) }

	pub(super) fn set_url_preview(
		&self, url: &str, data: &UrlPreviewData, timestamp: std::time::Duration,
	) -> Result<()> {
		let mut value = Vec::<u8>::new();
		value.extend_from_slice(&timestamp.as_secs().to_be_bytes());
		value.push(0xFF);
		value.extend_from_slice(
			data.title
				.as_ref()
				.map(String::as_bytes)
				.unwrap_or_default(),
		);
		value.push(0xFF);
		value.extend_from_slice(
			data.description
				.as_ref()
				.map(String::as_bytes)
				.unwrap_or_default(),
		);
		value.push(0xFF);
		value.extend_from_slice(
			data.image
				.as_ref()
				.map(String::as_bytes)
				.unwrap_or_default(),
		);
		value.push(0xFF);
		value.extend_from_slice(&data.image_size.unwrap_or(0).to_be_bytes());
		value.push(0xFF);
		value.extend_from_slice(&data.image_width.unwrap_or(0).to_be_bytes());
		value.push(0xFF);
		value.extend_from_slice(&data.image_height.unwrap_or(0).to_be_bytes());

		self.url_previews.insert(url.as_bytes(), &value)
	}

	pub(super) fn get_url_preview(&self, url: &str) -> Option<UrlPreviewData> {
		let values = self.url_previews.get(url.as_bytes()).ok()??;

		let mut values = values.split(|&b| b == 0xFF);

		let _ts = values.next();
		/* if we ever decide to use timestamp, this is here.
		match values.next().map(|b| u64::from_be_bytes(b.try_into().expect("valid BE array"))) {
			Some(0) => None,
			x => x,
		};*/

		let title = match values
			.next()
			.and_then(|b| String::from_utf8(b.to_vec()).ok())
		{
			Some(s) if s.is_empty() => None,
			x => x,
		};
		let description = match values
			.next()
			.and_then(|b| String::from_utf8(b.to_vec()).ok())
		{
			Some(s) if s.is_empty() => None,
			x => x,
		};
		let image = match values
			.next()
			.and_then(|b| String::from_utf8(b.to_vec()).ok())
		{
			Some(s) if s.is_empty() => None,
			x => x,
		};
		let image_size = match values
			.next()
			.map(|b| usize::from_be_bytes(b.try_into().unwrap_or_default()))
		{
			Some(0) => None,
			x => x,
		};
		let image_width = match values
			.next()
			.map(|b| u32::from_be_bytes(b.try_into().unwrap_or_default()))
		{
			Some(0) => None,
			x => x,
		};
		let image_height = match values
			.next()
			.map(|b| u32::from_be_bytes(b.try_into().unwrap_or_default()))
		{
			Some(0) => None,
			x => x,
		};

		Some(UrlPreviewData {
			title,
			description,
			image,
			image_size,
			image_width,
			image_height,
		})
	}
}
