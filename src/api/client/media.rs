use axum::extract::State;
use axum_client_ip::InsecureClientIp;
use conduit::{
	utils::{self, content_disposition::make_content_disposition},
	Result,
};
use conduit_service::media::MXC_LENGTH;
use ruma::{api::client::media::create_content, Mxc};

use crate::Ruma;

/// # `POST /_matrix/media/v3/upload`
///
/// Permanently save media in the server.
///
/// - Some metadata will be saved in the database
/// - Media will be saved in the media/ directory
#[tracing::instrument(skip_all, fields(%client), name = "media_upload")]
pub(crate) async fn create_content_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<create_content::v3::Request>,
) -> Result<create_content::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let content_disposition = make_content_disposition(None, body.content_type.as_deref(), body.filename.as_deref());
	let mxc = Mxc {
		server_name: services.globals.server_name(),
		media_id: &utils::random_string(MXC_LENGTH),
	};

	services
		.media
		.create(
			&mxc,
			Some(sender_user),
			Some(&content_disposition),
			body.content_type.as_deref(),
			&body.file,
		)
		.await?;

	Ok(create_content::v3::Response {
		content_uri: mxc.to_string().into(),
		blurhash: None,
	})
}
