#![allow(deprecated)]

use std::time::Duration;

use axum::extract::State;
use axum_client_ip::InsecureClientIp;
use conduit::{
	debug_info, debug_warn, err, info,
	utils::{
		self,
		content_disposition::{content_disposition_type, make_content_disposition, sanitise_filename},
		math::ruma_from_usize,
	},
	warn, Err, Error, Result,
};
use ruma::api::client::media::{
	create_content, get_content, get_content_as_filename, get_content_thumbnail, get_media_config, get_media_preview,
};
use service::{
	media::{FileMeta, MXC_LENGTH},
	Services,
};

use crate::{Ruma, RumaResponse};

/// Cache control for immutable objects
const CACHE_CONTROL_IMMUTABLE: &str = "public,max-age=31536000,immutable";

const CORP_CROSS_ORIGIN: &str = "cross-origin";

/// # `GET /_matrix/media/v3/config`
///
/// Returns max upload size.
pub(crate) async fn get_media_config_route(
	State(services): State<crate::State>, _body: Ruma<get_media_config::v3::Request>,
) -> Result<get_media_config::v3::Response> {
	Ok(get_media_config::v3::Response {
		upload_size: ruma_from_usize(services.globals.config.max_request_size),
	})
}

/// # `GET /_matrix/media/v1/config`
///
/// This is a legacy endpoint ("/v1/") that some very old homeservers and/or
/// clients may call. conduwuit adds these for compatibility purposes.
/// See <https://spec.matrix.org/legacy/legacy/#id27>
///
/// Returns max upload size.
pub(crate) async fn get_media_config_v1_route(
	State(services): State<crate::State>, body: Ruma<get_media_config::v3::Request>,
) -> Result<RumaResponse<get_media_config::v3::Response>> {
	get_media_config_route(State(services), body)
		.await
		.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/preview_url`
///
/// Returns URL preview.
#[tracing::instrument(skip_all, fields(%client), name = "url_preview")]
pub(crate) async fn get_media_preview_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_media_preview::v3::Request>,
) -> Result<get_media_preview::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let url = &body.url;
	if !services.media.url_preview_allowed(url) {
		debug_info!(%sender_user, %url, "URL is not allowed to be previewed");
		return Err!(Request(Forbidden("URL is not allowed to be previewed")));
	}

	match services.media.get_url_preview(url).await {
		Ok(preview) => {
			let res = serde_json::value::to_raw_value(&preview).map_err(|e| {
				warn!(%sender_user, "Failed to convert UrlPreviewData into a serde json value: {e}");
				err!(Request(Unknown("Failed to generate a URL preview")))
			})?;

			Ok(get_media_preview::v3::Response::from_raw_value(res))
		},
		Err(e) => {
			info!(%sender_user, "Failed to generate a URL preview: {e}");
			Err!(Request(Unknown("Failed to generate a URL preview")))
		},
	}
}

/// # `GET /_matrix/media/v1/preview_url`
///
/// This is a legacy endpoint ("/v1/") that some very old homeservers and/or
/// clients may call. conduwuit adds these for compatibility purposes.
/// See <https://spec.matrix.org/legacy/legacy/#id27>
///
/// Returns URL preview.
#[tracing::instrument(skip_all, fields(%client), name = "url_preview")]
pub(crate) async fn get_media_preview_v1_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_media_preview::v3::Request>,
) -> Result<RumaResponse<get_media_preview::v3::Response>> {
	get_media_preview_route(State(services), InsecureClientIp(client), body)
		.await
		.map(RumaResponse)
}

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

	let mxc = format!("mxc://{}/{}", services.globals.server_name(), utils::random_string(MXC_LENGTH));

	services
		.media
		.create(
			Some(sender_user.clone()),
			&mxc,
			body.filename
				.as_ref()
				.map(|filename| {
					format!(
						"{}; filename={}",
						content_disposition_type(&body.content_type),
						sanitise_filename(filename.to_owned())
					)
				})
				.as_deref(),
			body.content_type.as_deref(),
			&body.file,
		)
		.await?;

	Ok(create_content::v3::Response {
		content_uri: mxc.into(),
		blurhash: None,
	})
}

/// # `POST /_matrix/media/v1/upload`
///
/// Permanently save media in the server.
///
/// This is a legacy endpoint ("/v1/") that some very old homeservers and/or
/// clients may call. conduwuit adds these for compatibility purposes.
/// See <https://spec.matrix.org/legacy/legacy/#id27>
///
/// - Some metadata will be saved in the database
/// - Media will be saved in the media/ directory
#[tracing::instrument(skip_all, fields(%client), name = "media_upload")]
pub(crate) async fn create_content_v1_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<create_content::v3::Request>,
) -> Result<RumaResponse<create_content::v3::Response>> {
	create_content_route(State(services), InsecureClientIp(client), body)
		.await
		.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}`
///
/// Load media from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_get")]
pub(crate) async fn get_content_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_content::v3::Request>,
) -> Result<get_content::v3::Response> {
	let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

	if let Some(FileMeta {
		content,
		content_type,
		content_disposition,
	}) = services.media.get(&mxc).await?
	{
		let content_disposition = Some(make_content_disposition(&content_type, content_disposition, None));
		let file = content.expect("content");

		Ok(get_content::v3::Response {
			file,
			content_type,
			content_disposition,
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
		})
	} else if !services.globals.server_is_ours(&body.server_name) && body.allow_remote {
		let response = get_remote_content(
			&services,
			&mxc,
			&body.server_name,
			body.media_id.clone(),
			body.allow_redirect,
			body.timeout_ms,
		)
		.await
		.map_err(|e| err!(Request(NotFound(debug_warn!("Fetching media `{mxc}` failed: {e:?}")))))?;

		let content_disposition = Some(make_content_disposition(
			&response.content_type,
			response.content_disposition,
			None,
		));

		Ok(get_content::v3::Response {
			file: response.file,
			content_type: response.content_type,
			content_disposition,
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.to_owned()),
		})
	} else {
		Err!(Request(NotFound("Media not found.")))
	}
}

/// # `GET /_matrix/media/v1/download/{serverName}/{mediaId}`
///
/// Load media from our server or over federation.
///
/// This is a legacy endpoint ("/v1/") that some very old homeservers and/or
/// clients may call. conduwuit adds these for compatibility purposes.
/// See <https://spec.matrix.org/legacy/legacy/#id27>
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_get")]
pub(crate) async fn get_content_v1_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_content::v3::Request>,
) -> Result<RumaResponse<get_content::v3::Response>> {
	get_content_route(State(services), InsecureClientIp(client), body)
		.await
		.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}/{fileName}`
///
/// Load media from our server or over federation, permitting desired filename.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_get")]
pub(crate) async fn get_content_as_filename_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_content_as_filename::v3::Request>,
) -> Result<get_content_as_filename::v3::Response> {
	let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

	if let Some(FileMeta {
		content,
		content_type,
		content_disposition,
	}) = services.media.get(&mxc).await?
	{
		let content_disposition = Some(make_content_disposition(
			&content_type,
			content_disposition,
			Some(body.filename.clone()),
		));

		let file = content.expect("content");
		Ok(get_content_as_filename::v3::Response {
			file,
			content_type,
			content_disposition,
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
		})
	} else if !services.globals.server_is_ours(&body.server_name) && body.allow_remote {
		match get_remote_content(
			&services,
			&mxc,
			&body.server_name,
			body.media_id.clone(),
			body.allow_redirect,
			body.timeout_ms,
		)
		.await
		{
			Ok(remote_content_response) => {
				let content_disposition = Some(make_content_disposition(
					&remote_content_response.content_type,
					remote_content_response.content_disposition,
					None,
				));

				Ok(get_content_as_filename::v3::Response {
					content_disposition,
					content_type: remote_content_response.content_type,
					file: remote_content_response.file,
					cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
					cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
				})
			},
			Err(e) => Err!(Request(NotFound(debug_warn!("Fetching media `{mxc}` failed: {e:?}")))),
		}
	} else {
		Err!(Request(NotFound("Media not found.")))
	}
}

/// # `GET /_matrix/media/v1/download/{serverName}/{mediaId}/{fileName}`
///
/// Load media from our server or over federation, permitting desired filename.
///
/// This is a legacy endpoint ("/v1/") that some very old homeservers and/or
/// clients may call. conduwuit adds these for compatibility purposes.
/// See <https://spec.matrix.org/legacy/legacy/#id27>
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_get")]
pub(crate) async fn get_content_as_filename_v1_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_content_as_filename::v3::Request>,
) -> Result<RumaResponse<get_content_as_filename::v3::Response>> {
	get_content_as_filename_route(State(services), InsecureClientIp(client), body)
		.await
		.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/thumbnail/{serverName}/{mediaId}`
///
/// Load media thumbnail from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_thumbnail_get")]
pub(crate) async fn get_content_thumbnail_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_content_thumbnail::v3::Request>,
) -> Result<get_content_thumbnail::v3::Response> {
	let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

	if let Some(FileMeta {
		content,
		content_type,
		content_disposition,
	}) = services
		.media
		.get_thumbnail(
			&mxc,
			body.width
				.try_into()
				.map_err(|e| err!(Request(InvalidParam("Width is invalid: {e:?}"))))?,
			body.height
				.try_into()
				.map_err(|e| err!(Request(InvalidParam("Height is invalid: {e:?}"))))?,
		)
		.await?
	{
		let content_disposition = Some(make_content_disposition(&content_type, content_disposition, None));
		let file = content.expect("content");

		Ok(get_content_thumbnail::v3::Response {
			file,
			content_type,
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
			content_disposition,
		})
	} else if !services.globals.server_is_ours(&body.server_name) && body.allow_remote {
		if services
			.globals
			.prevent_media_downloads_from()
			.contains(&body.server_name)
		{
			// we'll lie to the client and say the blocked server's media was not found and
			// log. the client has no way of telling anyways so this is a security bonus.
			debug_warn!("Received request for media `{}` on blocklisted server", mxc);
			return Err!(Request(NotFound("Media not found.")));
		}

		match services
			.sending
			.send_federation_request(
				&body.server_name,
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
			.await
		{
			Ok(get_thumbnail_response) => {
				services
					.media
					.upload_thumbnail(
						None,
						&mxc,
						None,
						get_thumbnail_response.content_type.as_deref(),
						body.width.try_into().expect("all UInts are valid u32s"),
						body.height.try_into().expect("all UInts are valid u32s"),
						&get_thumbnail_response.file,
					)
					.await?;

				let content_disposition = Some(make_content_disposition(
					&get_thumbnail_response.content_type,
					get_thumbnail_response.content_disposition,
					None,
				));

				Ok(get_content_thumbnail::v3::Response {
					file: get_thumbnail_response.file,
					content_type: get_thumbnail_response.content_type,
					cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
					cache_control: Some(CACHE_CONTROL_IMMUTABLE.to_owned()),
					content_disposition,
				})
			},
			Err(e) => Err!(Request(NotFound(debug_warn!("Fetching media `{mxc}` failed: {e:?}")))),
		}
	} else {
		Err!(Request(NotFound("Media not found.")))
	}
}

/// # `GET /_matrix/media/v1/thumbnail/{serverName}/{mediaId}`
///
/// Load media thumbnail from our server or over federation.
///
/// This is a legacy endpoint ("/v1/") that some very old homeservers and/or
/// clients may call. conduwuit adds these for compatibility purposes.
/// See <https://spec.matrix.org/legacy/legacy/#id27>
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_thumbnail_get")]
pub(crate) async fn get_content_thumbnail_v1_route(
	State(services): State<crate::State>, InsecureClientIp(client): InsecureClientIp,
	body: Ruma<get_content_thumbnail::v3::Request>,
) -> Result<RumaResponse<get_content_thumbnail::v3::Response>> {
	get_content_thumbnail_route(State(services), InsecureClientIp(client), body)
		.await
		.map(RumaResponse)
}

async fn get_remote_content(
	services: &Services, mxc: &str, server_name: &ruma::ServerName, media_id: String, allow_redirect: bool,
	timeout_ms: Duration,
) -> Result<get_content::v3::Response, Error> {
	if services
		.globals
		.prevent_media_downloads_from()
		.contains(&server_name.to_owned())
	{
		// we'll lie to the client and say the blocked server's media was not found and
		// log. the client has no way of telling anyways so this is a security bonus.
		debug_warn!("Received request for media `{mxc}` on blocklisted server");
		return Err!(Request(NotFound("Media not found.")));
	}

	let content_response = services
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

	let content_disposition = Some(make_content_disposition(
		&content_response.content_type,
		content_response.content_disposition,
		None,
	));

	services
		.media
		.create(
			None,
			mxc,
			content_disposition.as_deref(),
			content_response.content_type.as_deref(),
			&content_response.file,
		)
		.await?;

	Ok(get_content::v3::Response {
		file: content_response.file,
		content_type: content_response.content_type,
		content_disposition,
		cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
		cache_control: Some(CACHE_CONTROL_IMMUTABLE.to_owned()),
	})
}
