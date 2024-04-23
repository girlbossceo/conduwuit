use std::sync::Arc;

use crate::Result;

pub(crate) trait Data: Send + Sync {
	fn get_cached_eventid_authchain(&self, shorteventid: &[u64]) -> Result<Option<Arc<[u64]>>>;
	fn cache_auth_chain(&self, shorteventid: Vec<u64>, auth_chain: Arc<[u64]>) -> Result<()>;
}
