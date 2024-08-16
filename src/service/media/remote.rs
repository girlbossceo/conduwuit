use std::time::Duration;

use conduit::{debug_warn, err, implement, utils::content_disposition::make_content_disposition, Err, Error, Result};
use ruma::{api::client::media, Mxc};

#[implement(super::Service)]
#[allow(deprecated)]
pub async fn fetch_remote_thumbnail_legacy(
	&self, body: &media::get_content_thumbnail::v3::Request,
) -> Result<media::get_content_thumbnail::v3::Response> {
	let mxc = Mxc {
		server_name: &body.server_name,
		media_id: &body.media_id,
	};

	self.check_fetch_authorized(&mxc)?;
	let reponse = self
		.services
		.sending
		.send_federation_request(
			mxc.server_name,
			media::get_content_thumbnail::v3::Request {
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
		&mxc,
		None,
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
pub async fn fetch_remote_content_legacy(
	&self, mxc: &Mxc<'_>, allow_redirect: bool, timeout_ms: Duration,
) -> Result<media::get_content::v3::Response, Error> {
	self.check_fetch_authorized(mxc)?;
	let response = self
		.services
		.sending
		.send_federation_request(
			mxc.server_name,
			media::get_content::v3::Request {
				allow_remote: true,
				server_name: mxc.server_name.into(),
				media_id: mxc.media_id.into(),
				timeout_ms,
				allow_redirect,
			},
		)
		.await?;

	let content_disposition =
		make_content_disposition(response.content_disposition.as_ref(), response.content_type.as_deref(), None);

	self.create(
		mxc,
		None,
		Some(&content_disposition),
		response.content_type.as_deref(),
		&response.file,
	)
	.await?;

	Ok(response)
}

#[implement(super::Service)]
fn check_fetch_authorized(&self, mxc: &Mxc<'_>) -> Result<()> {
	if self
		.services
		.server
		.config
		.prevent_media_downloads_from
		.iter()
		.any(|entry| entry == mxc.server_name)
	{
		// we'll lie to the client and say the blocked server's media was not found and
		// log. the client has no way of telling anyways so this is a security bonus.
		debug_warn!(%mxc, "Received request for media on blocklisted server");
		return Err!(Request(NotFound("Media not found.")));
	}

	Ok(())
}
