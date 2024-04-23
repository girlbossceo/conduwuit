use crate::Result;

pub(crate) trait Data: Send + Sync {
	fn create_file_metadata(
		&self, sender_user: Option<&str>, mxc: String, width: u32, height: u32, content_disposition: Option<&str>,
		content_type: Option<&str>,
	) -> Result<Vec<u8>>;

	fn delete_file_mxc(&self, mxc: String) -> Result<()>;

	/// Returns content_disposition, content_type and the metadata key.
	fn search_file_metadata(
		&self, mxc: String, width: u32, height: u32,
	) -> Result<(Option<String>, Option<String>, Vec<u8>)>;

	fn search_mxc_metadata_prefix(&self, mxc: String) -> Result<Vec<Vec<u8>>>;

	fn get_all_media_keys(&self) -> Result<Vec<Vec<u8>>>;

	// TODO: use this
	#[allow(dead_code)]
	fn remove_url_preview(&self, url: &str) -> Result<()>;

	fn set_url_preview(&self, url: &str, data: &super::UrlPreviewData, timestamp: std::time::Duration) -> Result<()>;

	fn get_url_preview(&self, url: &str) -> Option<super::UrlPreviewData>;
}
