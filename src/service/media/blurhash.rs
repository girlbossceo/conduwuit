use std::{error::Error, ffi::OsStr, fmt::Display, io::Cursor, path::Path};

use conduwuit::{config::BlurhashConfig as CoreBlurhashConfig, err, implement, Result};
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader};

use super::Service;
#[implement(Service)]
pub fn create_blurhash(
	&self,
	file: &[u8],
	content_type: Option<&str>,
	file_name: Option<&str>,
) -> Result<Option<String>> {
	if !cfg!(feature = "blurhashing") {
		return Ok(None);
	}

	let config = BlurhashConfig::from(self.services.server.config.blurhashing);

	// since 0 means disabled blurhashing, skipped blurhashing
	if config.size_limit == 0 {
		return Ok(None);
	}

	get_blurhash_from_request(file, content_type, file_name, config)
		.map_err(|e| err!(debug_error!("blurhashing error: {e}")))
		.map(Some)
}

/// Returns the blurhash or a blurhash error which implements Display.
#[tracing::instrument(
	name = "blurhash",
	level = "debug",
	skip(data),
	fields(
		bytes = data.len(),
	),
)]
fn get_blurhash_from_request(
	data: &[u8],
	mime: Option<&str>,
	filename: Option<&str>,
	config: BlurhashConfig,
) -> Result<String, BlurhashingError> {
	// Get format image is supposed to be in
	let format = get_format_from_data_mime_and_filename(data, mime, filename)?;

	// Get the image reader for said image format
	let decoder = get_image_decoder_with_format_and_data(format, data)?;

	// Check image size makes sense before unpacking whole image
	if is_image_above_size_limit(&decoder, config) {
		return Err(BlurhashingError::ImageTooLarge);
	}

	// decode the image finally
	let image = DynamicImage::from_decoder(decoder)?;

	blurhash_an_image(&image, config)
}

/// Gets the Image Format value from the data,mime, and filename
/// It first checks if the mime is a valid image format
/// Then it checks if the filename has a format, otherwise just guess based on
/// the binary data Assumes that mime and filename extension won't be for a
/// different file format than file.
fn get_format_from_data_mime_and_filename(
	data: &[u8],
	mime: Option<&str>,
	filename: Option<&str>,
) -> Result<ImageFormat, BlurhashingError> {
	let extension = filename
		.map(Path::new)
		.and_then(Path::extension)
		.map(OsStr::to_string_lossy);

	mime.or(extension.as_deref())
		.and_then(ImageFormat::from_mime_type)
		.map_or_else(|| image::guess_format(data).map_err(Into::into), Ok)
}

fn get_image_decoder_with_format_and_data(
	image_format: ImageFormat,
	data: &[u8],
) -> Result<Box<dyn ImageDecoder + '_>, BlurhashingError> {
	let mut image_reader = ImageReader::new(Cursor::new(data));
	image_reader.set_format(image_format);
	Ok(Box::new(image_reader.into_decoder()?))
}

fn is_image_above_size_limit<T: ImageDecoder>(
	decoder: &T,
	blurhash_config: BlurhashConfig,
) -> bool {
	decoder.total_bytes() >= blurhash_config.size_limit
}

#[cfg(feature = "blurhashing")]
#[tracing::instrument(name = "encode", level = "debug", skip_all)]
#[inline]
fn blurhash_an_image(
	image: &DynamicImage,
	blurhash_config: BlurhashConfig,
) -> Result<String, BlurhashingError> {
	Ok(blurhash::encode_image(
		blurhash_config.components_x,
		blurhash_config.components_y,
		&image.to_rgba8(),
	)?)
}

#[cfg(not(feature = "blurhashing"))]
#[inline]
fn blurhash_an_image(
	_image: &DynamicImage,
	_blurhash_config: BlurhashConfig,
) -> Result<String, BlurhashingError> {
	Err(BlurhashingError::Unavailable)
}

#[derive(Clone, Copy, Debug)]
pub struct BlurhashConfig {
	pub components_x: u32,
	pub components_y: u32,

	/// size limit in bytes
	pub size_limit: u64,
}

impl From<CoreBlurhashConfig> for BlurhashConfig {
	fn from(value: CoreBlurhashConfig) -> Self {
		Self {
			components_x: value.components_x,
			components_y: value.components_y,
			size_limit: value.blurhash_max_raw_size,
		}
	}
}

#[derive(Debug)]
pub enum BlurhashingError {
	HashingLibError(Box<dyn Error + Send>),
	ImageError(Box<ImageError>),
	ImageTooLarge,

	#[cfg(not(feature = "blurhashing"))]
	Unavailable,
}

impl From<ImageError> for BlurhashingError {
	fn from(value: ImageError) -> Self { Self::ImageError(Box::new(value)) }
}

#[cfg(feature = "blurhashing")]
impl From<blurhash::Error> for BlurhashingError {
	fn from(value: blurhash::Error) -> Self { Self::HashingLibError(Box::new(value)) }
}

impl Display for BlurhashingError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Blurhash Error:")?;
		match &self {
			| Self::ImageTooLarge => write!(f, "Image was too large to blurhash")?,
			| Self::HashingLibError(e) =>
				write!(f, "There was an error with the blurhashing library => {e}")?,

			| Self::ImageError(e) =>
				write!(f, "There was an error with the image loading library => {e}")?,

			#[cfg(not(feature = "blurhashing"))]
			| Self::Unavailable => write!(f, "Blurhashing is not supported")?,
		};

		Ok(())
	}
}
