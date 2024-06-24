mod data;
mod tests;

use std::{collections::HashMap, io::Cursor, num::Saturating as Sat, path::PathBuf, sync::Arc, time::SystemTime};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use conduit::{checked, debug, debug_error, error, utils, Error, Result, Server};
use data::Data;
use image::imageops::FilterType;
use ruma::{OwnedMxcUri, OwnedUserId};
use serde::Serialize;
use tokio::{
	fs,
	io::{AsyncReadExt, AsyncWriteExt, BufReader},
	sync::{Mutex, RwLock},
};

use crate::services;

#[derive(Debug)]
pub struct FileMeta {
	pub content: Option<Vec<u8>>,
	pub content_type: Option<String>,
	pub content_disposition: Option<String>,
}

#[derive(Serialize, Default)]
pub struct UrlPreviewData {
	#[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:title"))]
	pub title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:description"))]
	pub description: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:image"))]
	pub image: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none", rename(serialize = "matrix:image:size"))]
	pub image_size: Option<usize>,
	#[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:image:width"))]
	pub image_width: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none", rename(serialize = "og:image:height"))]
	pub image_height: Option<u32>,
}

pub struct Service {
	server: Arc<Server>,
	pub(crate) db: Data,
	pub url_preview_mutex: RwLock<HashMap<String, Arc<Mutex<()>>>>,
}

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			server: args.server.clone(),
			db: Data::new(args.db),
			url_preview_mutex: RwLock::new(HashMap::new()),
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		self.create_media_dir().await?;

		Ok(())
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Uploads a file.
	pub async fn create(
		&self, sender_user: Option<OwnedUserId>, mxc: &str, content_disposition: Option<&str>,
		content_type: Option<&str>, file: &[u8],
	) -> Result<()> {
		// Width, Height = 0 if it's not a thumbnail
		let key = if let Some(user) = sender_user {
			self.db
				.create_file_metadata(Some(user.as_str()), mxc, 0, 0, content_disposition, content_type)?
		} else {
			self.db
				.create_file_metadata(None, mxc, 0, 0, content_disposition, content_type)?
		};

		//TODO: Dangling metadata in database if creation fails
		let mut f = self.create_media_file(&key).await?;
		f.write_all(file).await?;

		Ok(())
	}

	/// Deletes a file in the database and from the media directory via an MXC
	pub async fn delete(&self, mxc: &str) -> Result<()> {
		if let Ok(keys) = self.db.search_mxc_metadata_prefix(mxc) {
			for key in keys {
				self.remove_media_file(&key).await?;

				debug!("Deleting MXC {mxc} from database");
				self.db.delete_file_mxc(mxc)?;
			}

			Ok(())
		} else {
			error!("Failed to find any media keys for MXC \"{mxc}\" in our database (MXC does not exist)");
			Err(Error::bad_database(
				"Failed to find any media keys for the provided MXC in our database (MXC does not exist)",
			))
		}
	}

	/// Uploads or replaces a file thumbnail.
	#[allow(clippy::too_many_arguments)]
	pub async fn upload_thumbnail(
		&self, sender_user: Option<OwnedUserId>, mxc: &str, content_disposition: Option<&str>,
		content_type: Option<&str>, width: u32, height: u32, file: &[u8],
	) -> Result<()> {
		let key = if let Some(user) = sender_user {
			self.db
				.create_file_metadata(Some(user.as_str()), mxc, width, height, content_disposition, content_type)?
		} else {
			self.db
				.create_file_metadata(None, mxc, width, height, content_disposition, content_type)?
		};

		//TODO: Dangling metadata in database if creation fails
		let mut f = self.create_media_file(&key).await?;
		f.write_all(file).await?;

		Ok(())
	}

	/// Downloads a file.
	pub async fn get(&self, mxc: &str) -> Result<Option<FileMeta>> {
		if let Ok((content_disposition, content_type, key)) = self.db.search_file_metadata(mxc, 0, 0) {
			let mut content = Vec::new();
			let path = self.get_media_file(&key);
			BufReader::new(fs::File::open(path).await?)
				.read_to_end(&mut content)
				.await?;

			Ok(Some(FileMeta {
				content: Some(content),
				content_type,
				content_disposition,
			}))
		} else {
			Ok(None)
		}
	}

	/// Deletes all remote only media files in the given at or after
	/// time/duration. Returns a u32 with the amount of media files deleted.
	pub async fn delete_all_remote_media_at_after_time(&self, time: String, force: bool) -> Result<usize> {
		let all_keys = self.db.get_all_media_keys();

		let user_duration: SystemTime = match cyborgtime::parse_duration(&time) {
			Ok(duration) => {
				debug!("Parsed duration: {:?}", duration);
				debug!("System time now: {:?}", SystemTime::now());
				SystemTime::now().checked_sub(duration).ok_or_else(|| {
					Error::bad_database("Duration specified is not valid against the current system time")
				})?
			},
			Err(e) => {
				error!("Failed to parse user-specified time duration: {}", e);
				return Err(Error::bad_database("Failed to parse user-specified time duration."));
			},
		};

		let mut remote_mxcs: Vec<String> = vec![];

		for key in all_keys {
			debug!("Full MXC key from database: {:?}", key);

			// we need to get the MXC URL from the first part of the key (the first 0xff /
			// 255 push). this is all necessary because of conduit using magic keys for
			// media
			let mut parts = key.split(|&b| b == 0xFF);
			let mxc = parts
				.next()
				.map(|bytes| {
					utils::string_from_bytes(bytes).map_err(|e| {
						error!("Failed to parse MXC unicode bytes from our database: {}", e);
						Error::bad_database("Failed to parse MXC unicode bytes from our database")
					})
				})
				.transpose()?;

			let Some(mxc_s) = mxc else {
				return Err(Error::bad_database(
					"Parsed MXC URL unicode bytes from database but still is None",
				));
			};

			debug!("Parsed MXC key to URL: {}", mxc_s);

			let mxc = OwnedMxcUri::from(mxc_s);
			if mxc.server_name() == Ok(services().globals.server_name()) {
				debug!("Ignoring local media MXC: {}", mxc);
				// ignore our own MXC URLs as this would be local media.
				continue;
			}

			let path = self.get_media_file(&key);
			debug!("MXC path: {path:?}");

			let file_metadata = fs::metadata(path.clone()).await?;
			debug!("File metadata: {file_metadata:?}");

			let file_created_at = match file_metadata.created() {
				Ok(value) => value,
				Err(err) if err.kind() == std::io::ErrorKind::Unsupported => {
					debug!("btime is unsupported, using mtime instead");
					file_metadata.modified()?
				},
				Err(err) => {
					if force {
						error!("Could not delete MXC path {:?}: {:?}. Skipping...", path, err);
						continue;
					}
					return Err(err.into());
				},
			};
			debug!("File created at: {:?}", file_created_at);

			if file_created_at <= user_duration {
				debug!("File is within user duration, pushing to list of file paths and keys to delete.");
				remote_mxcs.push(mxc.to_string());
			}
		}

		debug!(
			"Finished going through all our media in database for eligible keys to delete, checking if these are empty"
		);

		if remote_mxcs.is_empty() {
			return Err(Error::bad_database("Did not found any eligible MXCs to delete."));
		}

		debug!("Deleting media now in the past \"{:?}\".", user_duration);

		let mut deletion_count: usize = 0;

		for mxc in remote_mxcs {
			debug!("Deleting MXC {mxc} from database and filesystem");
			self.delete(&mxc).await?;
			deletion_count = deletion_count.saturating_add(1);
		}

		Ok(deletion_count)
	}

	/// Returns width, height of the thumbnail and whether it should be cropped.
	/// Returns None when the server should send the original file.
	pub fn thumbnail_properties(&self, width: u32, height: u32) -> Option<(u32, u32, bool)> {
		match (width, height) {
			(0..=32, 0..=32) => Some((32, 32, true)),
			(0..=96, 0..=96) => Some((96, 96, true)),
			(0..=320, 0..=240) => Some((320, 240, false)),
			(0..=640, 0..=480) => Some((640, 480, false)),
			(0..=800, 0..=600) => Some((800, 600, false)),
			_ => None,
		}
	}

	/// Downloads a file's thumbnail.
	///
	/// Here's an example on how it works:
	///
	/// - Client requests an image with width=567, height=567
	/// - Server rounds that up to (800, 600), so it doesn't have to save too
	///   many thumbnails
	/// - Server rounds that up again to (958, 600) to fix the aspect ratio
	///   (only for width,height>96)
	/// - Server creates the thumbnail and sends it to the user
	///
	/// For width,height <= 96 the server uses another thumbnailing algorithm
	/// which crops the image afterwards.
	pub async fn get_thumbnail(&self, mxc: &str, width: u32, height: u32) -> Result<Option<FileMeta>> {
		let (width, height, crop) = self
			.thumbnail_properties(width, height)
			.unwrap_or((0, 0, false)); // 0, 0 because that's the original file

		if let Ok((content_disposition, content_type, key)) = self.db.search_file_metadata(mxc, width, height) {
			// Using saved thumbnail
			let mut content = Vec::new();
			let path = self.get_media_file(&key);
			fs::File::open(path)
				.await?
				.read_to_end(&mut content)
				.await?;

			Ok(Some(FileMeta {
				content: Some(content),
				content_type,
				content_disposition,
			}))
		} else if let Ok((content_disposition, content_type, key)) = self.db.search_file_metadata(mxc, 0, 0) {
			// Generate a thumbnail
			let mut content = Vec::new();
			let path = self.get_media_file(&key);
			fs::File::open(path)
				.await?
				.read_to_end(&mut content)
				.await?;

			if let Ok(image) = image::load_from_memory(&content) {
				let original_width = image.width();
				let original_height = image.height();
				if width > original_width || height > original_height {
					return Ok(Some(FileMeta {
						content: Some(content),
						content_type,
						content_disposition,
					}));
				}

				let thumbnail = if crop {
					image.resize_to_fill(width, height, FilterType::CatmullRom)
				} else {
					let (exact_width, exact_height) = {
						let ratio = Sat(original_width) * Sat(height);
						let nratio = Sat(width) * Sat(original_height);

						let use_width = nratio <= ratio;
						let intermediate = if use_width {
							Sat(original_height) * Sat(checked!(width / original_width)?)
						} else {
							Sat(original_width) * Sat(checked!(height / original_height)?)
						};

						if use_width {
							(width, intermediate.0)
						} else {
							(intermediate.0, height)
						}
					};

					image.thumbnail_exact(exact_width, exact_height)
				};

				let mut thumbnail_bytes = Vec::new();
				thumbnail.write_to(&mut Cursor::new(&mut thumbnail_bytes), image::ImageFormat::Png)?;

				// Save thumbnail in database so we don't have to generate it again next time
				let thumbnail_key = self.db.create_file_metadata(
					None,
					mxc,
					width,
					height,
					content_disposition.as_deref(),
					content_type.as_deref(),
				)?;

				let mut f = self.create_media_file(&thumbnail_key).await?;
				f.write_all(&thumbnail_bytes).await?;

				Ok(Some(FileMeta {
					content: Some(thumbnail_bytes),
					content_type,
					content_disposition,
				}))
			} else {
				// Couldn't parse file to generate thumbnail, send original
				Ok(Some(FileMeta {
					content: Some(content),
					content_type,
					content_disposition,
				}))
			}
		} else {
			Ok(None)
		}
	}

	pub async fn get_url_preview(&self, url: &str) -> Option<UrlPreviewData> { self.db.get_url_preview(url) }

	/// TODO: use this?
	#[allow(dead_code)]
	pub async fn remove_url_preview(&self, url: &str) -> Result<()> {
		// TODO: also remove the downloaded image
		self.db.remove_url_preview(url)
	}

	pub async fn set_url_preview(&self, url: &str, data: &UrlPreviewData) -> Result<()> {
		let now = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH)
			.expect("valid system time");
		self.db.set_url_preview(url, data, now)
	}

	pub async fn create_media_dir(&self) -> Result<()> {
		let dir = self.get_media_dir();
		Ok(fs::create_dir_all(dir).await?)
	}

	async fn remove_media_file(&self, key: &[u8]) -> Result<()> {
		let path = self.get_media_file(key);
		let legacy = self.get_media_file_b64(key);
		debug!(?key, ?path, ?legacy, "Removing media file");

		let file_rm = fs::remove_file(&path);
		let legacy_rm = fs::remove_file(&legacy);
		let (file_rm, legacy_rm) = tokio::join!(file_rm, legacy_rm);
		if let Err(e) = legacy_rm {
			if self.server.config.media_compat_file_link {
				debug_error!(?key, ?legacy, "Failed to remove legacy media symlink: {e}");
			}
		}

		Ok(file_rm?)
	}

	async fn create_media_file(&self, key: &[u8]) -> Result<fs::File> {
		let path = self.get_media_file(key);
		debug!(?key, ?path, "Creating media file");

		let file = fs::File::create(&path).await?;
		if self.server.config.media_compat_file_link {
			let legacy = self.get_media_file_b64(key);
			if let Err(e) = fs::symlink(&path, &legacy).await {
				debug_error!(
					key = ?encode_key(key), ?path, ?legacy,
					"Failed to create legacy media symlink: {e}"
				);
			}
		}

		Ok(file)
	}

	#[inline]
	pub fn get_media_file(&self, key: &[u8]) -> PathBuf { self.get_media_file_sha256(key) }

	/// new SHA256 file name media function. requires database migrated. uses
	/// SHA256 hash of the base64 key as the file name
	pub fn get_media_file_sha256(&self, key: &[u8]) -> PathBuf {
		let mut r = self.get_media_dir();
		// Using the hash of the base64 key as the filename
		// This is to prevent the total length of the path from exceeding the maximum
		// length in most filesystems
		let digest = <sha2::Sha256 as sha2::Digest>::digest(key);
		let encoded = encode_key(&digest);
		r.push(encoded);
		r
	}

	/// old base64 file name media function
	/// This is the old version of `get_media_file` that uses the full base64
	/// key as the filename.
	pub fn get_media_file_b64(&self, key: &[u8]) -> PathBuf {
		let mut r = self.get_media_dir();
		let encoded = encode_key(key);
		r.push(encoded);
		r
	}

	pub fn get_media_dir(&self) -> PathBuf {
		let mut r = PathBuf::new();
		r.push(self.server.config.database_path.clone());
		r.push("media");
		r
	}
}

#[inline]
#[must_use]
pub fn encode_key(key: &[u8]) -> String { general_purpose::URL_SAFE_NO_PAD.encode(key) }
