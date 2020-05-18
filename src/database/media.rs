use crate::{utils, Error, Result};

pub struct Media {
    pub(super) mediaid_file: sled::Tree, // MediaId = MXC + Filename + ContentType
}

impl Media {
    /// Uploads or replaces a file.
    pub fn create(
        &self,
        mxc: String,
        filename: Option<&String>,
        content_type: &str,
        file: &[u8],
    ) -> Result<()> {
        let mut key = mxc.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(filename.map(|f| f.as_bytes()).unwrap_or_default());
        key.push(0xff);
        key.extend_from_slice(content_type.as_bytes());

        self.mediaid_file.insert(key, file)?;

        Ok(())
    }

    /// Downloads a file.
    pub fn get(&self, mxc: String) -> Result<Option<(Option<String>, String, Vec<u8>)>> {
        let mut prefix = mxc.as_bytes().to_vec();
        prefix.push(0xff);

        if let Some(r) = self.mediaid_file.scan_prefix(&prefix).next() {
            let (key, file) = r?;
            let mut parts = key.split(|&b| b == 0xff).skip(1);

            let filename_bytes = parts
                .next()
                .ok_or(Error::BadDatabase("mediaid is invalid"))?;
            let filename = if filename_bytes.is_empty() {
                None
            } else {
                Some(utils::string_from_bytes(filename_bytes)?)
            };

            let content_type = utils::string_from_bytes(
                parts
                    .next()
                    .ok_or(Error::BadDatabase("mediaid is invalid"))?,
            )?;

            Ok(Some((filename, content_type, file.to_vec())))
        } else {
            Ok(None)
        }
    }
}
