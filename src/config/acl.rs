use serde::Deserialize;
use std::collections::HashSet;
use url::Host;
#[derive(Deserialize, Debug, Default, Clone)]
pub struct AccessControlListConfig {
    #[serde(default = "default_as_false")]
    pub allow_only_federation_from_allow_list: bool,
    #[serde(default)]
    pub(crate) allow_list: HashSet<Host<String>>,

    #[serde(default)]
    pub(crate) block_list: HashSet<Host<String>>,
}

fn default_as_false() -> bool {
    false
}
