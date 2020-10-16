use crate::{utils, Error, Result};
use std::mem;

pub struct FileMeta {
    pub filename: Option<String>,
    pub content_type: String,
    pub file: Vec<u8>,
}

pub struct Media {
    pub(super) mediaid_file: sled::Tree, // MediaId = MXC + WidthHeight + Filename + ContentType
}

impl Media {
    /// Uploads or replaces a file.
    pub fn create(
        &self,
        mxc: String,
        filename: &Option<&str>,
        content_type: &str,
        file: &[u8],
    ) -> Result<()> {
        let mut key = mxc.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(&0_u32.to_be_bytes()); // Width = 0 if it's not a thumbnail
        key.extend_from_slice(&0_u32.to_be_bytes()); // Height = 0 if it's not a thumbnail
        key.push(0xff);
        key.extend_from_slice(filename.as_ref().map(|f| f.as_bytes()).unwrap_or_default());
        key.push(0xff);
        key.extend_from_slice(content_type.as_bytes());

        self.mediaid_file.insert(key, file)?;

        Ok(())
    }

    /// Uploads or replaces a file thumbnail.
    pub fn upload_thumbnail(
        &self,
        mxc: String,
        filename: &Option<String>,
        content_type: &str,
        width: u32,
        height: u32,
        file: &[u8],
    ) -> Result<()> {
        let mut key = mxc.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(&width.to_be_bytes());
        key.extend_from_slice(&height.to_be_bytes());
        key.push(0xff);
        key.extend_from_slice(filename.as_ref().map(|f| f.as_bytes()).unwrap_or_default());
        key.push(0xff);
        key.extend_from_slice(content_type.as_bytes());

        self.mediaid_file.insert(key, file)?;

        Ok(())
    }

    /// Downloads a file.
    pub fn get(&self, mxc: &str) -> Result<Option<FileMeta>> {
        let mut prefix = mxc.as_bytes().to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(&0_u32.to_be_bytes()); // Width = 0 if it's not a thumbnail
        prefix.extend_from_slice(&0_u32.to_be_bytes()); // Height = 0 if it's not a thumbnail
        prefix.push(0xff);

        if let Some(r) = self.mediaid_file.scan_prefix(&prefix).next() {
            let (key, file) = r?;
            let mut parts = key.rsplit(|&b| b == 0xff);

            let content_type = utils::string_from_bytes(
                parts
                    .next()
                    .ok_or_else(|| Error::bad_database("Media ID in db is invalid."))?,
            )
            .map_err(|_| Error::bad_database("Content type in mediaid_file is invalid unicode."))?;

            let filename_bytes = parts
                .next()
                .ok_or_else(|| Error::bad_database("Media ID in db is invalid."))?;

            let filename = if filename_bytes.is_empty() {
                None
            } else {
                Some(utils::string_from_bytes(filename_bytes).map_err(|_| {
                    Error::bad_database("Filename in mediaid_file is invalid unicode.")
                })?)
            };

            Ok(Some(FileMeta {
                filename,
                content_type,
                file: file.to_vec(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Downloads a file's thumbnail.
    pub fn get_thumbnail(&self, mxc: String, width: u32, height: u32) -> Result<Option<FileMeta>> {
        let mut main_prefix = mxc.as_bytes().to_vec();
        main_prefix.push(0xff);

        let mut thumbnail_prefix = main_prefix.clone();
        thumbnail_prefix.extend_from_slice(&width.to_be_bytes());
        thumbnail_prefix.extend_from_slice(&height.to_be_bytes());
        thumbnail_prefix.push(0xff);

        let mut original_prefix = main_prefix;
        original_prefix.extend_from_slice(&0_u32.to_be_bytes()); // Width = 0 if it's not a thumbnail
        original_prefix.extend_from_slice(&0_u32.to_be_bytes()); // Height = 0 if it's not a thumbnail
        original_prefix.push(0xff);

        if let Some(r) = self.mediaid_file.scan_prefix(&thumbnail_prefix).next() {
            // Using saved thumbnail
            let (key, file) = r?;
            let mut parts = key.rsplit(|&b| b == 0xff);

            let content_type = utils::string_from_bytes(
                parts
                    .next()
                    .ok_or_else(|| Error::bad_database("Invalid Media ID in db"))?,
            )
            .map_err(|_| Error::bad_database("Content type in mediaid_file is invalid unicode."))?;

            let filename_bytes = parts
                .next()
                .ok_or_else(|| Error::bad_database("Media ID in db is invalid."))?;

            let filename = if filename_bytes.is_empty() {
                None
            } else {
                Some(
                    utils::string_from_bytes(filename_bytes)
                        .map_err(|_| Error::bad_database("Filename in db is invalid."))?,
                )
            };

            Ok(Some(FileMeta {
                filename,
                content_type,
                file: file.to_vec(),
            }))
        } else if let Some(r) = self.mediaid_file.scan_prefix(&original_prefix).next() {
            // Generate a thumbnail
            let (key, file) = r?;
            let mut parts = key.rsplit(|&b| b == 0xff);

            let content_type = utils::string_from_bytes(
                parts
                    .next()
                    .ok_or_else(|| Error::bad_database("Media ID in db is invalid."))?,
            )
            .map_err(|_| Error::bad_database("Content type in mediaid_file is invalid unicode."))?;

            let filename_bytes = parts
                .next()
                .ok_or_else(|| Error::bad_database("Media ID in db is invalid."))?;

            let filename = if filename_bytes.is_empty() {
                None
            } else {
                Some(utils::string_from_bytes(filename_bytes).map_err(|_| {
                    Error::bad_database("Filename in mediaid_file is invalid unicode.")
                })?)
            };

            if let Ok(image) = image::load_from_memory(&file) {
                let thumbnail = image.thumbnail(width, height);
                let mut thumbnail_bytes = Vec::new();
                thumbnail.write_to(&mut thumbnail_bytes, image::ImageOutputFormat::Png)?;

                // Save thumbnail in database so we don't have to generate it again next time
                let mut thumbnail_key = key.to_vec();
                let width_index = thumbnail_key
                    .iter()
                    .position(|&b| b == 0xff)
                    .ok_or_else(|| Error::bad_database("Media in db is invalid."))?
                    + 1;
                let mut widthheight = width.to_be_bytes().to_vec();
                widthheight.extend_from_slice(&height.to_be_bytes());

                thumbnail_key.splice(
                    width_index..width_index + 2 * mem::size_of::<u32>(),
                    widthheight,
                );

                self.mediaid_file.insert(thumbnail_key, &*thumbnail_bytes)?;

                Ok(Some(FileMeta {
                    filename,
                    content_type,
                    file: thumbnail_bytes.to_vec(),
                }))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}
