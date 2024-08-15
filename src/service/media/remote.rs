use std::time::Duration;

use conduit::{debug_warn, err, implement, utils::content_disposition::make_content_disposition, Err, Error, Result};
use ruma::{
	api::client::media::{get_content, get_content_thumbnail},
	ServerName,
};

#[implement(super::Service)]
#[allow(deprecated)]
pub async fn fetch_remote_thumbnail(
	&self, mxc: &str, body: &get_content_thumbnail::v3::Request,
) -> Result<get_content_thumbnail::v3::Response> {
	let server_name = &body.server_name;
	self.check_fetch_authorized(mxc, server_name)?;

	let reponse = self
		.services
		.sending
		.send_federation_request(
			server_name,
			get_content_thumbnail::v3::Request {
				allow_remote: body.allow_remote,
				height: body.height,
				width: body.width,
				method: body.method.clone(),
				server_name: body.server_name.clone(),
				media_id: body.media_id.clone(),
				timeout_ms: body.timeout_ms,
				allow_redirect: body.allow_redirect,
				animated: body.animated,
			},
		)
		.await?;

	self.upload_thumbnail(
		None,
		mxc,
		None,
		reponse.content_type.as_deref(),
		body.width
			.try_into()
			.map_err(|e| err!(Request(InvalidParam("Width is invalid: {e:?}"))))?,
		body.height
			.try_into()
			.map_err(|e| err!(Request(InvalidParam("Height is invalid: {e:?}"))))?,
		&reponse.file,
	)
	.await?;

	Ok(reponse)
}

#[implement(super::Service)]
#[allow(deprecated)]
pub async fn fetch_remote_content(
	&self, mxc: &str, server_name: &ServerName, media_id: String, allow_redirect: bool, timeout_ms: Duration,
) -> Result<get_content::v3::Response, Error> {
	self.check_fetch_authorized(mxc, server_name)?;

	let response = self
		.services
		.sending
		.send_federation_request(
			server_name,
			get_content::v3::Request {
				allow_remote: true,
				server_name: server_name.to_owned(),
				media_id,
				timeout_ms,
				allow_redirect,
			},
		)
		.await?;

	let content_disposition =
		make_content_disposition(response.content_disposition.as_ref(), response.content_type.as_deref(), None);

	self.create(
		None,
		mxc,
		Some(&content_disposition),
		response.content_type.as_deref(),
		&response.file,
	)
	.await?;

	Ok(response)
}

#[implement(super::Service)]
fn check_fetch_authorized(&self, mxc: &str, server_name: &ServerName) -> Result<()> {
	if self
		.services
		.server
		.config
		.prevent_media_downloads_from
		.iter()
		.any(|entry| entry == server_name)
	{
		// we'll lie to the client and say the blocked server's media was not found and
		// log. the client has no way of telling anyways so this is a security bonus.
		debug_warn!(%mxc, "Received request for media on blocklisted server");
		return Err!(Request(NotFound("Media not found.")));
	}

	Ok(())
}
