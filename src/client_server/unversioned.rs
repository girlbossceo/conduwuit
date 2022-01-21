use std::{collections::BTreeMap, iter::FromIterator};

use crate::ConduitResult;
use ruma::api::client::unversioned::get_supported_versions;

#[cfg(feature = "conduit_bin")]
use rocket::get;

/// # `GET /_matrix/client/versions`
///
/// Get the versions of the specification and unstable features supported by this server.
///
/// - Versions take the form MAJOR.MINOR.PATCH
/// - Only the latest PATCH release will be reported for each MAJOR.MINOR value
/// - Unstable features are namespaced and may include version information in their name
///
/// Note: Unstable features are used while developing new features. Clients should avoid using
/// unstable features in their stable releases
#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/versions"))]
#[tracing::instrument]
pub async fn get_supported_versions_route() -> ConduitResult<get_supported_versions::Response> {
    let resp = get_supported_versions::Response {
        versions: vec!["r0.5.0".to_owned(), "r0.6.0".to_owned()],
        unstable_features: BTreeMap::from_iter([("org.matrix.e2e_cross_signing".to_owned(), true)]),
    };

    Ok(resp.into())
}
