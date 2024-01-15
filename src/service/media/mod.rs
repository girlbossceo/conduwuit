mod data;
use std::io::Cursor;

pub(crate) use data::Data;

use crate::{services, Result};
use image::imageops::FilterType;

use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
};

pub struct FileMeta {
    pub content_disposition: Option<String>,
    pub content_type: Option<String>,
    pub file: Vec<u8>,
}

pub struct Service {
    pub db: &'static dyn Data,
}

impl Service {
    /// Uploads a file.
    pub async fn create(
        &self,
        mxc: String,
        content_disposition: Option<&str>,
        content_type: Option<&str>,
        file: &[u8],
    ) -> Result<()> {
        // Width, Height = 0 if it's not a thumbnail
        let key = self
            .db
            .create_file_metadata(mxc, 0, 0, content_disposition, content_type)?;

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

    /// Uploads or replaces a file thumbnail.
    pub async fn upload_thumbnail(
        &self,
        mxc: String,
        content_disposition: Option<&str>,
        content_type: Option<&str>,
        width: u32,
        height: u32,
        file: &[u8],
    ) -> Result<()> {
        let key =
            self.db
                .create_file_metadata(mxc, width, height, content_disposition, content_type)?;

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
        if let Ok((content_disposition, content_type, key)) =
            self.db.search_file_metadata(mxc, 0, 0)
        {
            let path = if cfg!(feature = "sha256_media") {
                services().globals.get_media_file_new(&key)
            } else {
                #[allow(deprecated)]
                services().globals.get_media_file(&key)
            };

            let mut file = Vec::new();
            BufReader::new(File::open(path).await?)
                .read_to_end(&mut file)
                .await?;

            Ok(Some(FileMeta {
                content_disposition,
                content_type,
                file,
            }))
        } else {
            Ok(None)
        }
    }

    /// Returns width, height of the thumbnail and whether it should be cropped. Returns None when
    /// the server should send the original file.
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
    /// - Server rounds that up to (800, 600), so it doesn't have to save too many thumbnails
    /// - Server rounds that up again to (958, 600) to fix the aspect ratio (only for width,height>96)
    /// - Server creates the thumbnail and sends it to the user
    ///
    /// For width,height <= 96 the server uses another thumbnailing algorithm which crops the image afterwards.
    pub async fn get_thumbnail(
        &self,
        mxc: String,
        width: u32,
        height: u32,
    ) -> Result<Option<FileMeta>> {
        let (width, height, crop) = self
            .thumbnail_properties(width, height)
            .unwrap_or((0, 0, false)); // 0, 0 because that's the original file

        if let Ok((content_disposition, content_type, key)) =
            self.db.search_file_metadata(mxc.clone(), width, height)
        {
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
                file: file.to_vec(),
            }))
        } else if let Ok((content_disposition, content_type, key)) =
            self.db.search_file_metadata(mxc.clone(), 0, 0)
        {
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
                        file: file.to_vec(),
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
                            u64::from(original_height) * u64::from(width)
                                / u64::from(original_width)
                        } else {
                            u64::from(original_width) * u64::from(height)
                                / u64::from(original_height)
                        };
                        if use_width {
                            if intermediate <= u64::from(::std::u32::MAX) {
                                (width, intermediate as u32)
                            } else {
                                (
                                    (u64::from(width) * u64::from(::std::u32::MAX) / intermediate)
                                        as u32,
                                    ::std::u32::MAX,
                                )
                            }
                        } else if intermediate <= u64::from(::std::u32::MAX) {
                            (intermediate as u32, height)
                        } else {
                            (
                                ::std::u32::MAX,
                                (u64::from(height) * u64::from(::std::u32::MAX) / intermediate)
                                    as u32,
                            )
                        }
                    };

                    image.thumbnail_exact(exact_width, exact_height)
                };

                let mut thumbnail_bytes = Vec::new();
                thumbnail.write_to(
                    &mut Cursor::new(&mut thumbnail_bytes),
                    image::ImageOutputFormat::Png,
                )?;

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
                    file: thumbnail_bytes.to_vec(),
                }))
            } else {
                // Couldn't parse file to generate thumbnail, send original
                Ok(Some(FileMeta {
                    content_disposition,
                    content_type,
                    file: file.to_vec(),
                }))
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sha2::Digest;

    use base64::{engine::general_purpose, Engine as _};

    use super::*;

    struct MockedKVDatabase;

    impl Data for MockedKVDatabase {
        fn create_file_metadata(
            &self,
            mxc: String,
            width: u32,
            height: u32,
            content_disposition: Option<&str>,
            content_type: Option<&str>,
        ) -> Result<Vec<u8>> {
            // copied from src/database/key_value/media.rs
            let mut key = mxc.as_bytes().to_vec();
            key.push(0xff);
            key.extend_from_slice(&width.to_be_bytes());
            key.extend_from_slice(&height.to_be_bytes());
            key.push(0xff);
            key.extend_from_slice(
                content_disposition
                    .as_ref()
                    .map(|f| f.as_bytes())
                    .unwrap_or_default(),
            );
            key.push(0xff);
            key.extend_from_slice(
                content_type
                    .as_ref()
                    .map(|c| c.as_bytes())
                    .unwrap_or_default(),
            );

            Ok(key)
        }

        fn search_file_metadata(
            &self,
            _mxc: String,
            _width: u32,
            _height: u32,
        ) -> Result<(Option<String>, Option<String>, Vec<u8>)> {
            todo!()
        }
    }

    #[tokio::test]
    async fn long_file_names_works() {
        static DB: MockedKVDatabase = MockedKVDatabase;
        let media = Service { db: &DB };

        let mxc = "mxc://example.com/ascERGshawAWawugaAcauga".to_owned();
        let width = 100;
        let height = 100;
        let content_disposition = "attachment; filename=\"this is a very long file name with spaces and special characters like äöüß and even emoji like 🦀.png\"";
        let content_type = "image/png";
        let key = media
            .db
            .create_file_metadata(
                mxc,
                width,
                height,
                Some(content_disposition),
                Some(content_type),
            )
            .unwrap();
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
