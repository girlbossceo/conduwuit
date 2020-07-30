use crate::ConduitResult;
use ruma::api::client::unversioned::get_supported_versions;
use std::collections::BTreeMap;

#[cfg(feature = "conduit_bin")]
use rocket::get;

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/versions"))]
pub fn get_supported_versions_route() -> ConduitResult<get_supported_versions::Response> {
    let mut unstable_features = BTreeMap::new();

    unstable_features.insert("org.matrix.e2e_cross_signing".to_owned(), true);

    Ok(get_supported_versions::Response {
        versions: vec!["r0.5.0".to_owned(), "r0.6.0".to_owned()],
        unstable_features,
    }
    .into())
}
