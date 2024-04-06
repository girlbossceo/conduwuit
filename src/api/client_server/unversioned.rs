use std::collections::BTreeMap;

use axum::{response::IntoResponse, Json};
use ruma::api::client::{
	discovery::{
		discover_support::{self, Contact},
		get_supported_versions,
	},
	error::ErrorKind,
};

use crate::{services, Error, Result, Ruma};

/// # `GET /_matrix/client/versions`
///
/// Get the versions of the specification and unstable features supported by
/// this server.
///
/// - Versions take the form MAJOR.MINOR.PATCH
/// - Only the latest PATCH release will be reported for each MAJOR.MINOR value
/// - Unstable features are namespaced and may include version information in
///   their name
///
/// Note: Unstable features are used while developing new features. Clients
/// should avoid using unstable features in their stable releases
pub async fn get_supported_versions_route(
	_body: Ruma<get_supported_versions::Request>,
) -> Result<get_supported_versions::Response> {
	let resp = get_supported_versions::Response {
		versions: vec![
			"r0.0.1".to_owned(),
			"r0.1.0".to_owned(),
			"r0.2.0".to_owned(),
			"r0.3.0".to_owned(),
			"r0.4.0".to_owned(),
			"r0.5.0".to_owned(),
			"r0.6.0".to_owned(),
			"r0.6.1".to_owned(),
			"v1.1".to_owned(),
			"v1.2".to_owned(),
			"v1.3".to_owned(),
			"v1.4".to_owned(),
			"v1.5".to_owned(),
		],
		unstable_features: BTreeMap::from_iter([
			("org.matrix.e2e_cross_signing".to_owned(), true),
			("org.matrix.msc2285.stable".to_owned(), true),
			("org.matrix.msc2836".to_owned(), true),
			("org.matrix.msc2946".to_owned(), true),
			("org.matrix.msc3827".to_owned(), true),
			("org.matrix.msc3026.busy_presence".to_owned(), true),
		]),
	};

	Ok(resp)
}

/// # `GET /.well-known/matrix/client`
pub async fn well_known_client_route() -> Result<impl IntoResponse> {
	let client_url = match services().globals.well_known_client() {
		Some(url) => url.clone(),
		None => return Err(Error::BadRequest(ErrorKind::NotFound, "Not found.")),
	};

	Ok(Json(serde_json::json!({
		"m.homeserver": {"base_url": client_url},
		"org.matrix.msc3575.proxy": {"url": client_url}
	})))
}

/// # `GET /.well-known/matrix/support`
///
/// Server support contact and support page of a homeserver's domain.
pub async fn well_known_support(_body: Ruma<discover_support::Request>) -> Result<discover_support::Response> {
	let support_page = services().globals.well_known_support_page().clone();

	let role = services().globals.well_known_support_role().clone();

	// support page or role must be either defined for this to be valid
	if support_page.is_none() && role.is_none() {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Not found."));
	}

	let email_address = services().globals.well_known_support_email().clone();
	let matrix_id = services().globals.well_known_support_mxid().clone();

	// if a role is specified, an email address or matrix id is required
	if role.is_some() && (email_address.is_none() && matrix_id.is_none()) {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Not found."));
	}

	// TOOD: support defining multiple contacts in the config
	let mut contacts: Vec<Contact> = vec![];

	if let Some(role) = role {
		let contact = Contact {
			role,
			email_address,
			matrix_id,
		};

		contacts.push(contact);
	}

	// support page or role+contacts must be either defined for this to be valid
	if contacts.is_empty() && support_page.is_none() {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Not found."));
	}

	Ok(discover_support::Response {
		contacts,
		support_page,
	})
}

/// # `GET /client/server.json`
///
/// Endpoint provided by sliding sync proxy used by some clients such as Element
/// Web as a non-standard health check.
pub async fn syncv3_client_server_json() -> Result<impl IntoResponse> {
	let server_url = match services().globals.well_known_client() {
		Some(url) => url.clone(),
		None => match services().globals.well_known_server() {
			Some(url) => url.clone(),
			None => return Err(Error::BadRequest(ErrorKind::NotFound, "Not found.")),
		},
	};

	let version = match option_env!("CONDUIT_VERSION_EXTRA") {
		Some(extra) => format!("{} ({})", env!("CARGO_PKG_VERSION"), extra),
		None => env!("CARGO_PKG_VERSION").to_owned(),
	};

	Ok(Json(serde_json::json!({
		"server": server_url,
		"version": version,
	})))
}

/// # `GET /_conduwuit/server_version`
///
/// Conduwuit-specific API to get the server version, results akin to
/// `/_matrix/federation/v1/version`
pub async fn conduwuit_server_version() -> Result<impl IntoResponse> {
	let version = match option_env!("CONDUIT_VERSION_EXTRA") {
		Some(extra) => format!("{} ({})", env!("CARGO_PKG_VERSION"), extra),
		None => env!("CARGO_PKG_VERSION").to_owned(),
	};

	Ok(Json(serde_json::json!({
		"name": "Conduwuit",
		"version": version,
	})))
}
