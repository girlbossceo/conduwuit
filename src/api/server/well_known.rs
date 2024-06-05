use ruma::api::{client::error::ErrorKind, federation::discovery::discover_homeserver};

use crate::{services, Error, Result, Ruma};

/// # `GET /.well-known/matrix/server`
///
/// Returns the .well-known URL if it is configured, otherwise returns 404.
pub(crate) async fn well_known_server(
	_body: Ruma<discover_homeserver::Request>,
) -> Result<discover_homeserver::Response> {
	Ok(discover_homeserver::Response {
		server: match services().globals.well_known_server() {
			Some(server_name) => server_name.to_owned(),
			None => return Err(Error::BadRequest(ErrorKind::NotFound, "Not found.")),
		},
	})
}
