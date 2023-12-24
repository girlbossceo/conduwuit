use serde::Deserialize;
use std::collections::HashSet;
use url::Host;
#[derive(Deserialize, Debug, Default, Clone)]
pub struct AccessControlListConfig {
    /// setting this explicitly enables allowlists
    pub(crate) allow_list: Option<HashSet<Host<String>>>,

    #[serde(default)]
    pub(crate) block_list: HashSet<Host<String>>,
}
