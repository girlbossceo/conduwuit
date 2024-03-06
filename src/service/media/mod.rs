mod data;
use std::{collections::HashMap, io::Cursor, sync::Arc, time::SystemTime};

pub(crate) use data::Data;
use image::imageops::FilterType;
use ruma::OwnedMxcUri;
use serde::Serialize;
use tokio::{
	fs::{self, File},
	io::{AsyncReadExt, AsyncWriteExt, BufReader},
	sync::{Mutex, RwLock},
};
use tracing::{debug, error};

use crate::{services, utils, Error, Result};

#[derive(Debug)]
pub struct FileMeta {
	pub content_disposition: Option<String>,
	pub content_type: Option<String>,
	pub file: Vec<u8>,
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
	pub db: &'static dyn Data,
	pub url_preview_mutex: RwLock<HashMap<String, Arc<Mutex<()>>>>,
}

impl Service {
	/// Uploads a file.
	pub async fn create(
		&self, mxc: String, content_disposition: Option<&str>, content_type: Option<&str>, file: &[u8],
	) -> Result<()> {
		// Width, Height = 0 if it's not a thumbnail
		let key = self.db.create_file_metadata(mxc, 0, 0, content_disposition, content_type)?;

		let path = if cfg!(feature = "sha256_media") {
			services().globals.get_media_file_new(&key)
		} else {
			#[allow(deprecated)]
			services().globals.get_media_file(&key)
		};

		let mut f = File::create(path).await?;
		f.write_all(file).await?;
		Ok(())
	}

	/// Deletes a file in the database and from the media directory via an MXC
	pub async fn delete(&self, mxc: String) -> Result<()> {
		if let Ok(keys) = self.db.search_mxc_metadata_prefix(mxc.clone()) {
			for key in keys {
				let file_path = if cfg!(feature = "sha256_media") {
					services().globals.get_media_file_new(&key)
				} else {
					#[allow(deprecated)]
					services().globals.get_media_file(&key)
				};
				debug!("Got local file path: {:?}", file_path);

				debug!("Deleting local file {:?} from filesystem, original MXC: {}", file_path, mxc);
				tokio::fs::remove_file(file_path).await?;

				debug!("Deleting MXC {mxc} from database");
				self.db.delete_file_mxc(mxc.clone())?;
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
	pub async fn upload_thumbnail(
		&self, mxc: String, content_disposition: Option<&str>, content_type: Option<&str>, width: u32, height: u32,
		file: &[u8],
	) -> Result<()> {
		let key = self.db.create_file_metadata(mxc, width, height, content_disposition, content_type)?;

		let path = if cfg!(feature = "sha256_media") {
			services().globals.get_media_file_new(&key)
		} else {
			#[allow(deprecated)]
			services().globals.get_media_file(&key)
		};

		let mut f = File::create(path).await?;
		f.write_all(file).await?;

		Ok(())
	}

	/// Downloads a file.
	pub async fn get(&self, mxc: String) -> Result<Option<FileMeta>> {
		if let Ok((content_disposition, content_type, key)) = self.db.search_file_metadata(mxc, 0, 0) {
			let path = if cfg!(feature = "sha256_media") {
				services().globals.get_media_file_new(&key)
			} else {
				#[allow(deprecated)]
				services().globals.get_media_file(&key)
			};

			let mut file = Vec::new();
			BufReader::new(File::open(path).await?).read_to_end(&mut file).await?;

			Ok(Some(FileMeta {
				content_disposition,
				content_type,
				file,
			}))
		} else {
			Ok(None)
		}
	}

	/// Deletes all remote only media files in the given at or after
	/// time/duration. Returns a u32 with the amount of media files deleted.
	pub async fn delete_all_remote_media_at_after_time(&self, time: String) -> Result<u32> {
		if let Ok(all_keys) = self.db.get_all_media_keys() {
			let user_duration: SystemTime = match cyborgtime::parse_duration(&time) {
				Ok(duration) => {
					debug!("Parsed duration: {:?}", duration);
					debug!("System time now: {:?}", SystemTime::now());
					SystemTime::now() - duration
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
				// 255 push) this code does look kinda crazy but blame conduit for using magic
				// keys
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

				let mxc_s = match mxc {
					Some(mxc) => mxc,
					None => {
						return Err(Error::bad_database(
							"Parsed MXC URL unicode bytes from database but still is None",
						));
					},
				};

				debug!("Parsed MXC key to URL: {}", mxc_s);

				let mxc = OwnedMxcUri::from(mxc_s);
				if mxc.server_name() == Ok(services().globals.server_name()) {
					debug!("Ignoring local media MXC: {}", mxc);
					// ignore our own MXC URLs as this would be local media.
					continue;
				}

				let path = if cfg!(feature = "sha256_media") {
					services().globals.get_media_file_new(&key)
				} else {
					#[allow(deprecated)]
					services().globals.get_media_file(&key)
				};

				debug!("MXC path: {:?}", path);

				let file_metadata = fs::metadata(path.clone()).await?;
				debug!("File metadata: {:?}", file_metadata);

				let file_created_at = file_metadata.created()?;
				debug!("File created at: {:?}", file_created_at);

				if file_created_at >= user_duration {
					debug!("File is within user duration, pushing to list of file paths and keys to delete.");
					remote_mxcs.push(mxc.to_string());
				}
			}

			debug!(
				"Finished going through all our media in database for eligible keys to delete, checking if these are \
				 empty"
			);

			if remote_mxcs.is_empty() {
				return Err(Error::bad_database("Did not found any eligible MXCs to delete."));
			}

			debug!("Deleting media now in the past \"{:?}\".", user_duration);

			let mut deletion_count = 0;

			for mxc in remote_mxcs {
				debug!("Deleting MXC {mxc} from database and filesystem");
				self.delete(mxc).await?;
				deletion_count += 1;
			}

			Ok(deletion_count)
		} else {
			Err(Error::bad_database(
				"Failed to get all our media keys (filesystem or database issue?).",
			))
		}
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
	pub async fn get_thumbnail(&self, mxc: String, width: u32, height: u32) -> Result<Option<FileMeta>> {
		let (width, height, crop) = self.thumbnail_properties(width, height).unwrap_or((0, 0, false)); // 0, 0 because that's the original file

		if let Ok((content_disposition, content_type, key)) = self.db.search_file_metadata(mxc.clone(), width, height) {
			// Using saved thumbnail
			let path = if cfg!(feature = "sha256_media") {
				services().globals.get_media_file_new(&key)
			} else {
				#[allow(deprecated)]
				services().globals.get_media_file(&key)
			};

			let mut file = Vec::new();
			File::open(path).await?.read_to_end(&mut file).await?;

			Ok(Some(FileMeta {
				content_disposition,
				content_type,
				file: file.clone(),
			}))
		} else if let Ok((content_disposition, content_type, key)) = self.db.search_file_metadata(mxc.clone(), 0, 0) {
			// Generate a thumbnail
			let path = if cfg!(feature = "sha256_media") {
				services().globals.get_media_file_new(&key)
			} else {
				#[allow(deprecated)]
				services().globals.get_media_file(&key)
			};

			let mut file = Vec::new();
			File::open(path).await?.read_to_end(&mut file).await?;

			if let Ok(image) = image::load_from_memory(&file) {
				let original_width = image.width();
				let original_height = image.height();
				if width > original_width || height > original_height {
					return Ok(Some(FileMeta {
						content_disposition,
						content_type,
						file: file.clone(),
					}));
				}

				let thumbnail = if crop {
					image.resize_to_fill(width, height, FilterType::CatmullRom)
				} else {
					let (exact_width, exact_height) = {
						// Copied from image::dynimage::resize_dimensions
						let ratio = u64::from(original_width) * u64::from(height);
						let nratio = u64::from(width) * u64::from(original_height);

						let use_width = nratio <= ratio;
						let intermediate = if use_width {
							u64::from(original_height) * u64::from(width) / u64::from(original_width)
						} else {
							u64::from(original_width) * u64::from(height) / u64::from(original_height)
						};
						if use_width {
							if intermediate <= u64::from(::std::u32::MAX) {
								(width, intermediate as u32)
							} else {
								(
									(u64::from(width) * u64::from(::std::u32::MAX) / intermediate) as u32,
									::std::u32::MAX,
								)
							}
						} else if intermediate <= u64::from(::std::u32::MAX) {
							(intermediate as u32, height)
						} else {
							(
								::std::u32::MAX,
								(u64::from(height) * u64::from(::std::u32::MAX) / intermediate) as u32,
							)
						}
					};

					image.thumbnail_exact(exact_width, exact_height)
				};

				let mut thumbnail_bytes = Vec::new();
				thumbnail.write_to(&mut Cursor::new(&mut thumbnail_bytes), image::ImageOutputFormat::Png)?;

				// Save thumbnail in database so we don't have to generate it again next time
				let thumbnail_key = self.db.create_file_metadata(
					mxc,
					width,
					height,
					content_disposition.as_deref(),
					content_type.as_deref(),
				)?;

				let path = if cfg!(feature = "sha256_media") {
					services().globals.get_media_file_new(&thumbnail_key)
				} else {
					#[allow(deprecated)]
					services().globals.get_media_file(&thumbnail_key)
				};

				let mut f = File::create(path).await?;
				f.write_all(&thumbnail_bytes).await?;

				Ok(Some(FileMeta {
					content_disposition,
					content_type,
					file: thumbnail_bytes.clone(),
				}))
			} else {
				// Couldn't parse file to generate thumbnail, send original
				Ok(Some(FileMeta {
					content_disposition,
					content_type,
					file: file.clone(),
				}))
			}
		} else {
			Ok(None)
		}
	}

	pub async fn get_url_preview(&self, url: &str) -> Option<UrlPreviewData> { self.db.get_url_preview(url) }

	pub async fn remove_url_preview(&self, url: &str) -> Result<()> {
		// TODO: also remove the downloaded image
		self.db.remove_url_preview(url)
	}

	pub async fn set_url_preview(&self, url: &str, data: &UrlPreviewData) -> Result<()> {
		let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("valid system time");
		self.db.set_url_preview(url, data, now)
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use base64::{engine::general_purpose, Engine as _};
	use sha2::Digest;

	use super::*;

	struct MockedKVDatabase;

	impl Data for MockedKVDatabase {
		fn create_file_metadata(
			&self, mxc: String, width: u32, height: u32, content_disposition: Option<&str>, content_type: Option<&str>,
		) -> Result<Vec<u8>> {
			// copied from src/database/key_value/media.rs
			let mut key = mxc.as_bytes().to_vec();
			key.push(0xFF);
			key.extend_from_slice(&width.to_be_bytes());
			key.extend_from_slice(&height.to_be_bytes());
			key.push(0xFF);
			key.extend_from_slice(content_disposition.as_ref().map(|f| f.as_bytes()).unwrap_or_default());
			key.push(0xFF);
			key.extend_from_slice(content_type.as_ref().map(|c| c.as_bytes()).unwrap_or_default());

			Ok(key)
		}

		fn delete_file_mxc(&self, _mxc: String) -> Result<()> { todo!() }

		fn search_mxc_metadata_prefix(&self, _mxc: String) -> Result<Vec<Vec<u8>>> { todo!() }

		fn get_all_media_keys(&self) -> Result<Vec<Vec<u8>>> { todo!() }

		fn search_file_metadata(
			&self, _mxc: String, _width: u32, _height: u32,
		) -> Result<(Option<String>, Option<String>, Vec<u8>)> {
			todo!()
		}

		fn remove_url_preview(&self, _url: &str) -> Result<()> { todo!() }

		fn set_url_preview(&self, _url: &str, _data: &UrlPreviewData, _timestamp: std::time::Duration) -> Result<()> {
			todo!()
		}

		fn get_url_preview(&self, _url: &str) -> Option<UrlPreviewData> { todo!() }
	}

	#[tokio::test]
	async fn long_file_names_works() {
		static DB: MockedKVDatabase = MockedKVDatabase;
		let media = Service {
			db: &DB,
			url_preview_mutex: RwLock::new(HashMap::new()),
		};

		let mxc = "mxc://example.com/ascERGshawAWawugaAcauga".to_owned();
		let width = 100;
		let height = 100;
		let content_disposition = "attachment; filename=\"this is a very long file name with spaces and special \
		                           characters like äöüß and even emoji like 🦀.png\"";
		let content_type = "image/png";
		let key =
			media.db.create_file_metadata(mxc, width, height, Some(content_disposition), Some(content_type)).unwrap();
		let mut r = PathBuf::new();
		r.push("/tmp");
		r.push("media");
		// r.push(base64::encode_config(key, base64::URL_SAFE_NO_PAD));
		// use the sha256 hash of the key as the file name instead of the key itself
		// this is because the base64 encoded key can be longer than 255 characters.
		r.push(general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(key)));
		// Check that the file path is not longer than 255 characters
		// (255 is the maximum length of a file path on most file systems)
		assert!(
			r.to_str().unwrap().len() <= 255,
			"File path is too long: {}",
			r.to_str().unwrap().len()
		);
	}
}
