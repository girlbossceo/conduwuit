use crate::{Result, Ruma};
use ruma::api::client::r0::thirdparty::get_protocols;

use std::collections::BTreeMap;

/// # `GET /_matrix/client/r0/thirdparty/protocols`
///
/// TODO: Fetches all metadata about protocols supported by the homeserver.
#[tracing::instrument(skip(_body))]
pub async fn get_protocols_route(
    _body: Ruma<get_protocols::Request>,
) -> Result<get_protocols::Response> {
    // TODO
    Ok(get_protocols::Response {
        protocols: BTreeMap::new(),
    })
}
