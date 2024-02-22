use crate::Result;

pub trait Data: Send + Sync {
    fn create_file_metadata(
        &self,
        mxc: String,
        width: u32,
        height: u32,
        content_disposition: Option<&str>,
        content_type: Option<&str>,
    ) -> Result<Vec<u8>>;

    fn delete_file_mxc(&self, mxc: String) -> Result<()>;

    /// Returns content_disposition, content_type and the metadata key.
    fn search_file_metadata(
        &self,
        mxc: String,
        width: u32,
        height: u32,
    ) -> Result<(Option<String>, Option<String>, Vec<u8>)>;

    fn remove_url_preview(&self, url: &str) -> Result<()>;

    fn set_url_preview(
        &self,
        url: &str,
        data: &super::UrlPreviewData,
        timestamp: std::time::Duration,
    ) -> Result<()>;

    fn get_url_preview(&self, url: &str) -> Option<super::UrlPreviewData>;
}
