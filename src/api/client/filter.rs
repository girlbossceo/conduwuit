use axum::extract::State;
use conduit::err;
use ruma::api::client::filter::{create_filter, get_filter};

use crate::{Result, Ruma};

/// # `GET /_matrix/client/r0/user/{userId}/filter/{filterId}`
///
/// Loads a filter that was previously created.
///
/// - A user can only access their own filters
pub(crate) async fn get_filter_route(
	State(services): State<crate::State>, body: Ruma<get_filter::v3::Request>,
) -> Result<get_filter::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	services
		.users
		.get_filter(sender_user, &body.filter_id)
		.await
		.map(get_filter::v3::Response::new)
		.map_err(|_| err!(Request(NotFound("Filter not found."))))
}

/// # `PUT /_matrix/client/r0/user/{userId}/filter`
///
/// Creates a new filter to be used by other endpoints.
pub(crate) async fn create_filter_route(
	State(services): State<crate::State>, body: Ruma<create_filter::v3::Request>,
) -> Result<create_filter::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let filter_id = services.users.create_filter(sender_user, &body.filter);

	Ok(create_filter::v3::Response::new(filter_id))
}
