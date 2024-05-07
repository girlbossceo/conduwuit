use std::{io::Cursor, sync::Arc, time::Duration};

use image::io::Reader as ImgReader;
use ipaddress::IPAddress;
use reqwest::Url;
use ruma::api::client::{
	error::{ErrorKind, RetryAfter},
	media::{
		create_content, get_content, get_content_as_filename, get_content_thumbnail, get_media_config,
		get_media_preview,
	},
};
use tracing::{debug, error, warn};
use webpage::HTML;

use crate::{
	debug_warn,
	service::media::{FileMeta, UrlPreviewData},
	services,
	utils::{self, server_name::server_is_ours},
	Error, Result, Ruma, RumaResponse,
};

/// generated MXC ID (`media-id`) length
const MXC_LENGTH: usize = 32;

/// Cache control for immutable objects
const CACHE_CONTROL_IMMUTABLE: &str = "public,max-age=31536000,immutable";

const CORP_CROSS_ORIGIN: &str = "cross-origin";

/// # `GET /_matrix/media/v3/config`
///
/// Returns max upload size.
pub(crate) async fn get_media_config_route(
	_body: Ruma<get_media_config::v3::Request>,
) -> Result<get_media_config::v3::Response> {
	Ok(get_media_config::v3::Response {
		upload_size: services().globals.max_request_size().into(),
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
	body: Ruma<get_media_config::v3::Request>,
) -> Result<RumaResponse<get_media_config::v3::Response>> {
	get_media_config_route(body).await.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/preview_url`
///
/// Returns URL preview.
pub(crate) async fn get_media_preview_route(
	body: Ruma<get_media_preview::v3::Request>,
) -> Result<get_media_preview::v3::Response> {
	let url = &body.url;
	if !url_preview_allowed(url) {
		return Err(Error::BadRequest(ErrorKind::forbidden(), "URL is not allowed to be previewed"));
	}

	match get_url_preview(url).await {
		Ok(preview) => {
			let res = serde_json::value::to_raw_value(&preview).map_err(|e| {
				error!("Failed to convert UrlPreviewData into a serde json value: {}", e);
				Error::BadRequest(
					ErrorKind::LimitExceeded {
						retry_after: Some(RetryAfter::Delay(Duration::from_secs(5))),
					},
					"Failed to generate a URL preview, try again later.",
				)
			})?;

			Ok(get_media_preview::v3::Response::from_raw_value(res))
		},
		Err(e) => {
			warn!("Failed to generate a URL preview: {e}");

			// there doesn't seem to be an agreed-upon error code in the spec.
			// the only response codes in the preview_url spec page are 200 and 429.
			Err(Error::BadRequest(
				ErrorKind::LimitExceeded {
					retry_after: Some(RetryAfter::Delay(Duration::from_secs(5))),
				},
				"Failed to generate a URL preview, try again later.",
			))
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
pub(crate) async fn get_media_preview_v1_route(
	body: Ruma<get_media_preview::v3::Request>,
) -> Result<RumaResponse<get_media_preview::v3::Response>> {
	get_media_preview_route(body).await.map(RumaResponse)
}

/// # `POST /_matrix/media/v3/upload`
///
/// Permanently save media in the server.
///
/// - Some metadata will be saved in the database
/// - Media will be saved in the media/ directory
pub(crate) async fn create_content_route(
	body: Ruma<create_content::v3::Request>,
) -> Result<create_content::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let mxc = format!(
		"mxc://{}/{}",
		services().globals.server_name(),
		utils::random_string(MXC_LENGTH)
	);

	services()
		.media
		.create(
			Some(sender_user.clone()),
			mxc.clone(),
			body.filename
				.as_ref()
				.map(|filename| format!("attachment; filename={filename}"))
				.as_deref(),
			body.content_type.as_deref(),
			&body.file,
		)
		.await?;

	let content_uri = mxc.into();

	Ok(create_content::v3::Response {
		content_uri,
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
pub(crate) async fn create_content_v1_route(
	body: Ruma<create_content::v3::Request>,
) -> Result<RumaResponse<create_content::v3::Response>> {
	create_content_route(body).await.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}`
///
/// Load media from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
pub(crate) async fn get_content_route(body: Ruma<get_content::v3::Request>) -> Result<get_content::v3::Response> {
	let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

	if let Some(FileMeta {
		content_type,
		file,
		..
	}) = services().media.get(mxc.clone()).await?
	{
		// TODO: safely sanitise filename to be included in the content-disposition
		Ok(get_content::v3::Response {
			file,
			content_type,
			content_disposition: Some("attachment".to_owned()),
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
		})
	} else if !server_is_ours(&body.server_name) && body.allow_remote {
		get_remote_content(
			&mxc,
			&body.server_name,
			body.media_id.clone(),
			body.allow_redirect,
			body.timeout_ms,
		)
		.await
		.map_err(|e| {
			debug_warn!("Fetching media `{}` failed: {:?}", mxc, e);
			Error::BadRequest(ErrorKind::NotFound, "Remote media error.")
		})
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
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
pub(crate) async fn get_content_v1_route(
	body: Ruma<get_content::v3::Request>,
) -> Result<RumaResponse<get_content::v3::Response>> {
	get_content_route(body).await.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}/{fileName}`
///
/// Load media from our server or over federation, permitting desired filename.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
pub(crate) async fn get_content_as_filename_route(
	body: Ruma<get_content_as_filename::v3::Request>,
) -> Result<get_content_as_filename::v3::Response> {
	let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

	if let Some(FileMeta {
		content_type,
		file,
		..
	}) = services().media.get(mxc.clone()).await?
	{
		Ok(get_content_as_filename::v3::Response {
			file,
			content_type,
			content_disposition: Some("attachment".to_owned()),
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
		})
	} else if !server_is_ours(&body.server_name) && body.allow_remote {
		match get_remote_content(
			&mxc,
			&body.server_name,
			body.media_id.clone(),
			body.allow_redirect,
			body.timeout_ms,
		)
		.await
		{
			Ok(remote_content_response) => Ok(get_content_as_filename::v3::Response {
				content_disposition: Some("attachment".to_owned()),
				content_type: remote_content_response.content_type,
				file: remote_content_response.file,
				cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
				cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
			}),
			Err(e) => {
				debug_warn!("Fetching media `{}` failed: {:?}", mxc, e);
				Err(Error::BadRequest(ErrorKind::NotFound, "Remote media error."))
			},
		}
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
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
pub(crate) async fn get_content_as_filename_v1_route(
	body: Ruma<get_content_as_filename::v3::Request>,
) -> Result<RumaResponse<get_content_as_filename::v3::Response>> {
	get_content_as_filename_route(body).await.map(RumaResponse)
}

/// # `GET /_matrix/media/v3/thumbnail/{serverName}/{mediaId}`
///
/// Load media thumbnail from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
pub(crate) async fn get_content_thumbnail_route(
	body: Ruma<get_content_thumbnail::v3::Request>,
) -> Result<get_content_thumbnail::v3::Response> {
	let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

	if let Some(FileMeta {
		content_type,
		file,
		..
	}) = services()
		.media
		.get_thumbnail(
			mxc.clone(),
			body.width
				.try_into()
				.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Width is invalid."))?,
			body.height
				.try_into()
				.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Height is invalid."))?,
		)
		.await?
	{
		Ok(get_content_thumbnail::v3::Response {
			file,
			content_type,
			cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
			cache_control: Some(CACHE_CONTROL_IMMUTABLE.into()),
			content_disposition: Some("attachment".to_owned()),
		})
	} else if !server_is_ours(&body.server_name) && body.allow_remote {
		if services()
			.globals
			.prevent_media_downloads_from()
			.contains(&body.server_name.clone())
		{
			// we'll lie to the client and say the blocked server's media was not found and
			// log. the client has no way of telling anyways so this is a security bonus.
			debug_warn!("Received request for media `{}` on blocklisted server", mxc);
			return Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."));
		}

		match services()
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
				},
			)
			.await
		{
			Ok(get_thumbnail_response) => {
				services()
					.media
					.upload_thumbnail(
						None,
						mxc,
						None,
						get_thumbnail_response.content_type.as_deref(),
						body.width.try_into().expect("all UInts are valid u32s"),
						body.height.try_into().expect("all UInts are valid u32s"),
						&get_thumbnail_response.file,
					)
					.await?;

				Ok(get_content_thumbnail::v3::Response {
					file: get_thumbnail_response.file,
					content_type: get_thumbnail_response.content_type,
					cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
					cache_control: Some(CACHE_CONTROL_IMMUTABLE.to_owned()),
					content_disposition: Some("attachment".to_owned()),
				})
			},
			Err(e) => {
				debug_warn!("Fetching media `{}` failed: {:?}", mxc, e);
				Err(Error::BadRequest(ErrorKind::NotFound, "Remote media error."))
			},
		}
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
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
pub(crate) async fn get_content_thumbnail_v1_route(
	body: Ruma<get_content_thumbnail::v3::Request>,
) -> Result<RumaResponse<get_content_thumbnail::v3::Response>> {
	get_content_thumbnail_route(body).await.map(RumaResponse)
}

async fn get_remote_content(
	mxc: &str, server_name: &ruma::ServerName, media_id: String, allow_redirect: bool, timeout_ms: Duration,
) -> Result<get_content::v3::Response, Error> {
	if services()
		.globals
		.prevent_media_downloads_from()
		.contains(&server_name.to_owned())
	{
		// we'll lie to the client and say the blocked server's media was not found and
		// log. the client has no way of telling anyways so this is a security bonus.
		debug_warn!("Received request for media `{mxc}` on blocklisted server");
		return Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."));
	}

	let content_response = services()
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

	services()
		.media
		.create(
			None,
			mxc.to_owned(),
			Some("attachment"),
			content_response.content_type.as_deref(),
			&content_response.file,
		)
		.await?;

	Ok(get_content::v3::Response {
		file: content_response.file,
		content_type: content_response.content_type,
		content_disposition: Some("attachment".to_owned()),
		cross_origin_resource_policy: Some(CORP_CROSS_ORIGIN.to_owned()),
		cache_control: Some(CACHE_CONTROL_IMMUTABLE.to_owned()),
	})
}

async fn download_image(client: &reqwest::Client, url: &str) -> Result<UrlPreviewData> {
	let image = client.get(url).send().await?.bytes().await?;
	let mxc = format!(
		"mxc://{}/{}",
		services().globals.server_name(),
		utils::random_string(MXC_LENGTH)
	);

	services()
		.media
		.create(None, mxc.clone(), None, None, &image)
		.await?;

	let (width, height) = match ImgReader::new(Cursor::new(&image)).with_guessed_format() {
		Err(_) => (None, None),
		Ok(reader) => match reader.into_dimensions() {
			Err(_) => (None, None),
			Ok((width, height)) => (Some(width), Some(height)),
		},
	};

	Ok(UrlPreviewData {
		image: Some(mxc),
		image_size: Some(image.len()),
		image_width: width,
		image_height: height,
		..Default::default()
	})
}

async fn download_html(client: &reqwest::Client, url: &str) -> Result<UrlPreviewData> {
	let mut response = client.get(url).send().await?;

	let mut bytes: Vec<u8> = Vec::new();
	while let Some(chunk) = response.chunk().await? {
		bytes.extend_from_slice(&chunk);
		if bytes.len() > services().globals.url_preview_max_spider_size() {
			debug!(
				"Response body from URL {} exceeds url_preview_max_spider_size ({}), not processing the rest of the \
				 response body and assuming our necessary data is in this range.",
				url,
				services().globals.url_preview_max_spider_size()
			);
			break;
		}
	}
	let body = String::from_utf8_lossy(&bytes);
	let Ok(html) = HTML::from_string(body.to_string(), Some(url.to_owned())) else {
		return Err(Error::BadRequest(ErrorKind::Unknown, "Failed to parse HTML"));
	};

	let mut data = match html.opengraph.images.first() {
		None => UrlPreviewData::default(),
		Some(obj) => download_image(client, &obj.url).await?,
	};

	let props = html.opengraph.properties;

	/* use OpenGraph title/description, but fall back to HTML if not available */
	data.title = props.get("title").cloned().or(html.title);
	data.description = props.get("description").cloned().or(html.description);

	Ok(data)
}

async fn request_url_preview(url: &str) -> Result<UrlPreviewData> {
	if let Ok(ip) = IPAddress::parse(url) {
		if !services().globals.valid_cidr_range(&ip) {
			return Err(Error::BadServerResponse("Requesting from this address is forbidden"));
		}
	}

	let client = &services().globals.client.url_preview;
	let response = client.head(url).send().await?;

	if let Some(remote_addr) = response.remote_addr() {
		if let Ok(ip) = IPAddress::parse(remote_addr.ip().to_string()) {
			if !services().globals.valid_cidr_range(&ip) {
				return Err(Error::BadServerResponse("Requesting from this address is forbidden"));
			}
		}
	}

	let Some(content_type) = response
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.and_then(|x| x.to_str().ok())
	else {
		return Err(Error::BadRequest(ErrorKind::Unknown, "Unknown Content-Type"));
	};
	let data = match content_type {
		html if html.starts_with("text/html") => download_html(client, url).await?,
		img if img.starts_with("image/") => download_image(client, url).await?,
		_ => return Err(Error::BadRequest(ErrorKind::Unknown, "Unsupported Content-Type")),
	};

	services().media.set_url_preview(url, &data).await?;

	Ok(data)
}

async fn get_url_preview(url: &str) -> Result<UrlPreviewData> {
	if let Some(preview) = services().media.get_url_preview(url).await {
		return Ok(preview);
	}

	// ensure that only one request is made per URL
	let mutex_request = Arc::clone(
		services()
			.media
			.url_preview_mutex
			.write()
			.await
			.entry(url.to_owned())
			.or_default(),
	);
	let _request_lock = mutex_request.lock().await;

	match services().media.get_url_preview(url).await {
		Some(preview) => Ok(preview),
		None => request_url_preview(url).await,
	}
}

fn url_preview_allowed(url_str: &str) -> bool {
	let url: Url = match Url::parse(url_str) {
		Ok(u) => u,
		Err(e) => {
			warn!("Failed to parse URL from a str: {}", e);
			return false;
		},
	};

	if ["http", "https"]
		.iter()
		.all(|&scheme| scheme != url.scheme().to_lowercase())
	{
		debug!("Ignoring non-HTTP/HTTPS URL to preview: {}", url);
		return false;
	}

	let host = match url.host_str() {
		None => {
			debug!("Ignoring URL preview for a URL that does not have a host (?): {}", url);
			return false;
		},
		Some(h) => h.to_owned(),
	};

	let allowlist_domain_contains = services().globals.url_preview_domain_contains_allowlist();
	let allowlist_domain_explicit = services().globals.url_preview_domain_explicit_allowlist();
	let denylist_domain_explicit = services().globals.url_preview_domain_explicit_denylist();
	let allowlist_url_contains = services().globals.url_preview_url_contains_allowlist();

	if allowlist_domain_contains.contains(&"*".to_owned())
		|| allowlist_domain_explicit.contains(&"*".to_owned())
		|| allowlist_url_contains.contains(&"*".to_owned())
	{
		debug!("Config key contains * which is allowing all URL previews. Allowing URL {}", url);
		return true;
	}

	if !host.is_empty() {
		if denylist_domain_explicit.contains(&host) {
			debug!(
				"Host {} is not allowed by url_preview_domain_explicit_denylist (check 1/4)",
				&host
			);
			return false;
		}

		if allowlist_domain_explicit.contains(&host) {
			debug!("Host {} is allowed by url_preview_domain_explicit_allowlist (check 2/4)", &host);
			return true;
		}

		if allowlist_domain_contains
			.iter()
			.any(|domain_s| domain_s.contains(&host.clone()))
		{
			debug!("Host {} is allowed by url_preview_domain_contains_allowlist (check 3/4)", &host);
			return true;
		}

		if allowlist_url_contains
			.iter()
			.any(|url_s| url.to_string().contains(&url_s.to_string()))
		{
			debug!("URL {} is allowed by url_preview_url_contains_allowlist (check 4/4)", &host);
			return true;
		}

		// check root domain if available and if user has root domain checks
		if services().globals.url_preview_check_root_domain() {
			debug!("Checking root domain");
			match host.split_once('.') {
				None => return false,
				Some((_, root_domain)) => {
					if denylist_domain_explicit.contains(&root_domain.to_owned()) {
						debug!(
							"Root domain {} is not allowed by url_preview_domain_explicit_denylist (check 1/3)",
							&root_domain
						);
						return true;
					}

					if allowlist_domain_explicit.contains(&root_domain.to_owned()) {
						debug!(
							"Root domain {} is allowed by url_preview_domain_explicit_allowlist (check 2/3)",
							&root_domain
						);
						return true;
					}

					if allowlist_domain_contains
						.iter()
						.any(|domain_s| domain_s.contains(&root_domain.to_owned()))
					{
						debug!(
							"Root domain {} is allowed by url_preview_domain_contains_allowlist (check 3/3)",
							&root_domain
						);
						return true;
					}
				},
			}
		}
	}

	false
}
