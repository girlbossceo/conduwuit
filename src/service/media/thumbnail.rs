use std::{cmp, io::Cursor, num::Saturating as Sat};

use conduit::{checked, Result};
use image::{imageops::FilterType, DynamicImage};
use ruma::{http_headers::ContentDisposition, OwnedUserId};
use tokio::{
	fs,
	io::{AsyncReadExt, AsyncWriteExt},
};

use super::{data::Metadata, FileMeta};

impl super::Service {
	/// Uploads or replaces a file thumbnail.
	#[allow(clippy::too_many_arguments)]
	pub async fn upload_thumbnail(
		&self, sender_user: Option<OwnedUserId>, mxc: &str, content_disposition: Option<&ContentDisposition>,
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
	#[tracing::instrument(skip(self), name = "thumbnail", level = "debug")]
	pub async fn get_thumbnail(&self, mxc: &str, width: u32, height: u32) -> Result<Option<FileMeta>> {
		// 0, 0 because that's the original file
		let (width, height, crop) = thumbnail_properties(width, height).unwrap_or((0, 0, false));

		if let Ok(metadata) = self.db.search_file_metadata(mxc, width, height) {
			self.get_thumbnail_saved(metadata).await
		} else if let Ok(metadata) = self.db.search_file_metadata(mxc, 0, 0) {
			self.get_thumbnail_generate(mxc, width, height, crop, metadata)
				.await
		} else {
			Ok(None)
		}
	}

	/// Using saved thumbnail
	#[tracing::instrument(skip(self), name = "saved", level = "debug")]
	async fn get_thumbnail_saved(&self, data: Metadata) -> Result<Option<FileMeta>> {
		let mut content = Vec::new();
		let path = self.get_media_file(&data.key);
		fs::File::open(path)
			.await?
			.read_to_end(&mut content)
			.await?;

		Ok(Some(into_filemeta(data, content)))
	}

	/// Generate a thumbnail
	#[tracing::instrument(skip(self), name = "generate", level = "debug")]
	async fn get_thumbnail_generate(
		&self, mxc: &str, width: u32, height: u32, crop: bool, data: Metadata,
	) -> Result<Option<FileMeta>> {
		let mut content = Vec::new();
		let path = self.get_media_file(&data.key);
		fs::File::open(path)
			.await?
			.read_to_end(&mut content)
			.await?;

		let Ok(image) = image::load_from_memory(&content) else {
			// Couldn't parse file to generate thumbnail, send original
			return Ok(Some(into_filemeta(data, content)));
		};

		if width > image.width() || height > image.height() {
			return Ok(Some(into_filemeta(data, content)));
		}

		let mut thumbnail_bytes = Vec::new();
		let thumbnail = thumbnail_generate(&image, width, height, crop)?;
		thumbnail.write_to(&mut Cursor::new(&mut thumbnail_bytes), image::ImageFormat::Png)?;

		// Save thumbnail in database so we don't have to generate it again next time
		let thumbnail_key = self.db.create_file_metadata(
			None,
			mxc,
			width,
			height,
			data.content_disposition.as_ref(),
			data.content_type.as_deref(),
		)?;

		let mut f = self.create_media_file(&thumbnail_key).await?;
		f.write_all(&thumbnail_bytes).await?;

		Ok(Some(into_filemeta(data, thumbnail_bytes)))
	}
}

fn thumbnail_generate(image: &DynamicImage, width: u32, height: u32, crop: bool) -> Result<DynamicImage> {
	let thumbnail = if crop {
		image.resize_to_fill(width, height, FilterType::CatmullRom)
	} else {
		let (exact_width, exact_height) = thumbnail_dimension(image, width, height)?;
		image.thumbnail_exact(exact_width, exact_height)
	};

	Ok(thumbnail)
}

fn thumbnail_dimension(image: &DynamicImage, width: u32, height: u32) -> Result<(u32, u32)> {
	let image_width = image.width();
	let image_height = image.height();

	let width = cmp::min(width, image_width);
	let height = cmp::min(height, image_height);

	let use_width = Sat(width) * Sat(image_height) < Sat(height) * Sat(image_width);

	let x = if use_width {
		let dividend = (Sat(height) * Sat(image_width)).0;
		checked!(dividend / image_height)?
	} else {
		width
	};

	let y = if !use_width {
		let dividend = (Sat(width) * Sat(image_height)).0;
		checked!(dividend / image_width)?
	} else {
		height
	};

	Ok((x, y))
}

/// Returns width, height of the thumbnail and whether it should be cropped.
/// Returns None when the server should send the original file.
fn thumbnail_properties(width: u32, height: u32) -> Option<(u32, u32, bool)> {
	match (width, height) {
		(0..=32, 0..=32) => Some((32, 32, true)),
		(0..=96, 0..=96) => Some((96, 96, true)),
		(0..=320, 0..=240) => Some((320, 240, false)),
		(0..=640, 0..=480) => Some((640, 480, false)),
		(0..=800, 0..=600) => Some((800, 600, false)),
		_ => None,
	}
}

fn into_filemeta(data: Metadata, content: Vec<u8>) -> FileMeta {
	FileMeta {
		content: Some(content),
		content_type: data.content_type,
		content_disposition: data.content_disposition,
	}
}
