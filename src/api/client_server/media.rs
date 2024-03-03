use std::{io::Cursor, net::IpAddr, sync::Arc, time::Duration};

use crate::{
    service::media::{FileMeta, UrlPreviewData},
    services, utils, Error, Result, Ruma,
};
use image::io::Reader as ImgReader;

use reqwest::Url;
use ruma::api::client::{
    error::ErrorKind,
    media::{
        create_content, get_content, get_content_as_filename, get_content_thumbnail,
        get_media_config, get_media_preview,
    },
};
use tracing::{debug, error, info, warn};
use webpage::HTML;

/// generated MXC ID (`media-id`) length
const MXC_LENGTH: usize = 32;

/// # `GET /_matrix/media/v3/config`
///
/// Returns max upload size.
pub async fn get_media_config_route(
    _body: Ruma<get_media_config::v3::Request>,
) -> Result<get_media_config::v3::Response> {
    Ok(get_media_config::v3::Response {
        upload_size: services().globals.max_request_size().into(),
    })
}

/// # `GET /_matrix/media/v3/preview_url`
///
/// Returns URL preview.
pub async fn get_media_preview_route(
    body: Ruma<get_media_preview::v3::Request>,
) -> Result<get_media_preview::v3::Response> {
    let url = &body.url;
    if !url_preview_allowed(url) {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "URL is not allowed to be previewed",
        ));
    }

    if let Ok(preview) = get_url_preview(url).await {
        let res = serde_json::value::to_raw_value(&preview).map_err(|e| {
            error!(
                "Failed to convert UrlPreviewData into a serde json value: {}",
                e
            );
            Error::BadRequest(
                ErrorKind::Unknown,
                "Unknown error occurred parsing URL preview",
            )
        })?;

        return Ok(get_media_preview::v3::Response::from_raw_value(res));
    }

    Err(Error::BadRequest(
        ErrorKind::LimitExceeded {
            retry_after_ms: Some(Duration::from_secs(5)),
        },
        "Retry later",
    ))
}

/// # `POST /_matrix/media/v3/upload`
///
/// Permanently save media in the server.
///
/// - Some metadata will be saved in the database
/// - Media will be saved in the media/ directory
pub async fn create_content_route(
    body: Ruma<create_content::v3::Request>,
) -> Result<create_content::v3::Response> {
    let mxc = format!(
        "mxc://{}/{}",
        services().globals.server_name(),
        utils::random_string(MXC_LENGTH)
    );

    services()
        .media
        .create(
            mxc.clone(),
            body.filename
                .as_ref()
                .map(|filename| "inline; filename=".to_owned() + filename)
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

/// helper method to fetch remote media from other servers over federation
pub async fn get_remote_content(
    mxc: &str,
    server_name: &ruma::ServerName,
    media_id: String,
    allow_redirect: bool,
    timeout_ms: Duration,
) -> Result<get_content::v3::Response, Error> {
    // we'll lie to the client and say the blocked server's media was not found and log.
    // the client has no way of telling anyways so this is a security bonus.
    if services()
        .globals
        .prevent_media_downloads_from()
        .contains(&server_name.to_owned())
    {
        info!("Received request for remote media `{}` but server is in our media server blocklist. Returning 404.", mxc);
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
            mxc.to_owned(),
            content_response.content_disposition.as_deref(),
            content_response.content_type.as_deref(),
            &content_response.file,
        )
        .await?;

    Ok(content_response)
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}`
///
/// Load media from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20 seconds
pub async fn get_content_route(
    body: Ruma<get_content::v3::Request>,
) -> Result<get_content::v3::Response> {
    let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

    if let Some(FileMeta {
        content_disposition,
        content_type,
        file,
    }) = services().media.get(mxc.clone()).await?
    {
        Ok(get_content::v3::Response {
            file,
            content_type,
            content_disposition,
            cross_origin_resource_policy: Some("cross-origin".to_owned()),
        })
    } else if &*body.server_name != services().globals.server_name() && body.allow_remote {
        let remote_content_response = get_remote_content(
            &mxc,
            &body.server_name,
            body.media_id.clone(),
            body.allow_redirect,
            body.timeout_ms,
        )
        .await?;
        Ok(remote_content_response)
    } else {
        Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
    }
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}/{fileName}`
///
/// Load media from our server or over federation, permitting desired filename.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20 seconds
pub async fn get_content_as_filename_route(
    body: Ruma<get_content_as_filename::v3::Request>,
) -> Result<get_content_as_filename::v3::Response> {
    let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

    if let Some(FileMeta {
        content_type, file, ..
    }) = services().media.get(mxc.clone()).await?
    {
        Ok(get_content_as_filename::v3::Response {
            file,
            content_type,
            content_disposition: Some(format!("inline; filename={}", body.filename)),
            cross_origin_resource_policy: Some("cross-origin".to_owned()),
        })
    } else if &*body.server_name != services().globals.server_name() && body.allow_remote {
        let remote_content_response = get_remote_content(
            &mxc,
            &body.server_name,
            body.media_id.clone(),
            body.allow_redirect,
            body.timeout_ms,
        )
        .await?;

        Ok(get_content_as_filename::v3::Response {
            content_disposition: Some(format!("inline: filename={}", body.filename)),
            content_type: remote_content_response.content_type,
            file: remote_content_response.file,
            cross_origin_resource_policy: Some("cross-origin".to_owned()),
        })
    } else {
        Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
    }
}

/// # `GET /_matrix/media/v3/thumbnail/{serverName}/{mediaId}`
///
/// Load media thumbnail from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Only redirects if `allow_redirect` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20 seconds
pub async fn get_content_thumbnail_route(
    body: Ruma<get_content_thumbnail::v3::Request>,
) -> Result<get_content_thumbnail::v3::Response> {
    let mxc = format!("mxc://{}/{}", body.server_name, body.media_id);

    if let Some(FileMeta {
        content_type, file, ..
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
            cross_origin_resource_policy: Some("cross-origin".to_owned()),
        })
    } else if &*body.server_name != services().globals.server_name() && body.allow_remote {
        // we'll lie to the client and say the blocked server's media was not found and log.
        // the client has no way of telling anyways so this is a security bonus.
        if services()
            .globals
            .prevent_media_downloads_from()
            .contains(&body.server_name.clone())
        {
            info!("Received request for remote media `{}` but server is in our media server blocklist. Returning 404.", mxc);
            return Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."));
        }

        let get_thumbnail_response = services()
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
            .await?;

        services()
            .media
            .upload_thumbnail(
                mxc,
                None,
                get_thumbnail_response.content_type.as_deref(),
                body.width.try_into().expect("all UInts are valid u32s"),
                body.height.try_into().expect("all UInts are valid u32s"),
                &get_thumbnail_response.file,
            )
            .await?;

        Ok(get_thumbnail_response)
    } else {
        Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
    }
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
        .create(mxc.clone(), None, None, &image)
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
            debug!("Response body from URL {} exceeds url_preview_max_spider_size ({}), not processing the rest of the response body and assuming our necessary data is in this range.", url, services().globals.url_preview_max_spider_size());
            break;
        }
    }
    let body = String::from_utf8_lossy(&bytes);
    let html = match HTML::from_string(body.to_string(), Some(url.to_owned())) {
        Ok(html) => html,
        Err(_) => {
            return Err(Error::BadRequest(
                ErrorKind::Unknown,
                "Failed to parse HTML",
            ))
        }
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

fn url_request_allowed(addr: &IpAddr) -> bool {
    // TODO: make this check ip_range_denylist

    // could be implemented with reqwest when it supports IP filtering:
    // https://github.com/seanmonstar/reqwest/issues/1515

    // These checks have been taken from the Rust core/net/ipaddr.rs crate,
    // IpAddr::V4.is_global() and IpAddr::V6.is_global(), as .is_global is not
    // yet stabilized. TODO: Once this is stable, this match can be simplified.
    match addr {
        IpAddr::V4(ip4) => {
            !(ip4.octets()[0] == 0 // "This network"
                || ip4.is_private()
                || (ip4.octets()[0] == 100 && (ip4.octets()[1] & 0b1100_0000 == 0b0100_0000)) // is_shared()
                || ip4.is_loopback()
                || ip4.is_link_local()
                // addresses reserved for future protocols (`192.0.0.0/24`)
                || (ip4.octets()[0] == 192 && ip4.octets()[1] == 0 && ip4.octets()[2] == 0)
                || ip4.is_documentation()
                || (ip4.octets()[0] == 198 && (ip4.octets()[1] & 0xfe) == 18) // is_benchmarking()
                || (ip4.octets()[0] & 240 == 240 && !ip4.is_broadcast()) // is_reserved()
                || ip4.is_broadcast())
        }
        IpAddr::V6(ip6) => {
            !(ip6.is_unspecified()
                || ip6.is_loopback()
                // IPv4-mapped Address (`::ffff:0:0/96`)
                || matches!(ip6.segments(), [0, 0, 0, 0, 0, 0xffff, _, _])
                // IPv4-IPv6 Translat. (`64:ff9b:1::/48`)
                || matches!(ip6.segments(), [0x64, 0xff9b, 1, _, _, _, _, _])
                // Discard-Only Address Block (`100::/64`)
                || matches!(ip6.segments(), [0x100, 0, 0, 0, _, _, _, _])
                // IETF Protocol Assignments (`2001::/23`)
                || (matches!(ip6.segments(), [0x2001, b, _, _, _, _, _, _] if b < 0x200)
                    && !(
                        // Port Control Protocol Anycast (`2001:1::1`)
                        u128::from_be_bytes(ip6.octets()) == 0x2001_0001_0000_0000_0000_0000_0000_0001
                        // Traversal Using Relays around NAT Anycast (`2001:1::2`)
                        || u128::from_be_bytes(ip6.octets()) == 0x2001_0001_0000_0000_0000_0000_0000_0002
                        // AMT (`2001:3::/32`)
                        || matches!(ip6.segments(), [0x2001, 3, _, _, _, _, _, _])
                        // AS112-v6 (`2001:4:112::/48`)
                        || matches!(ip6.segments(), [0x2001, 4, 0x112, _, _, _, _, _])
                        // ORCHIDv2 (`2001:20::/28`)
                        || matches!(ip6.segments(), [0x2001, b, _, _, _, _, _, _] if (0x20..=0x2F).contains(&b))
                    ))
                || ((ip6.segments()[0] == 0x2001) && (ip6.segments()[1] == 0xdb8)) // is_documentation()
                || ((ip6.segments()[0] & 0xfe00) == 0xfc00) // is_unique_local()
                || ((ip6.segments()[0] & 0xffc0) == 0xfe80)) // is_unicast_link_local
        }
    }
}

async fn request_url_preview(url: &str) -> Result<UrlPreviewData> {
    let client = services().globals.url_preview_client();
    let response = client.head(url).send().await?;

    if !response
        .remote_addr()
        .map_or(false, |a| url_request_allowed(&a.ip()))
    {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "Requesting from this address is forbidden",
        ));
    }

    let content_type = match response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|x| x.to_str().ok())
    {
        Some(ct) => ct,
        None => {
            return Err(Error::BadRequest(
                ErrorKind::Unknown,
                "Unknown Content-Type",
            ))
        }
    };
    let data = match content_type {
        html if html.starts_with("text/html") => download_html(&client, url).await?,
        img if img.starts_with("image/") => download_image(&client, url).await?,
        _ => {
            return Err(Error::BadRequest(
                ErrorKind::Unknown,
                "Unsupported Content-Type",
            ))
        }
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
            .unwrap()
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
        }
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
            debug!(
                "Ignoring URL preview for a URL that does not have a host (?): {}",
                url
            );
            return false;
        }
        Some(h) => h.to_owned(),
    };

    let allowlist_domain_contains = services().globals.url_preview_domain_contains_allowlist();
    let allowlist_domain_explicit = services().globals.url_preview_domain_explicit_allowlist();
    let allowlist_url_contains = services().globals.url_preview_url_contains_allowlist();

    if allowlist_domain_contains.contains(&"*".to_owned())
        || allowlist_domain_explicit.contains(&"*".to_owned())
        || allowlist_url_contains.contains(&"*".to_owned())
    {
        debug!(
            "Config key contains * which is allowing all URL previews. Allowing URL {}",
            url
        );
        return true;
    }

    if !host.is_empty() {
        if allowlist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is allowed by url_preview_domain_explicit_allowlist (check 1/3)",
                &host
            );
            return true;
        }

        if allowlist_domain_contains
            .iter()
            .any(|domain_s| domain_s.contains(&host.clone()))
        {
            debug!(
                "Host {} is allowed by url_preview_domain_contains_allowlist (check 2/3)",
                &host
            );
            return true;
        }

        if allowlist_url_contains
            .iter()
            .any(|url_s| url.to_string().contains(&url_s.to_string()))
        {
            debug!(
                "URL {} is allowed by url_preview_url_contains_allowlist (check 3/3)",
                &host
            );
            return true;
        }

        // check root domain if available and if user has root domain checks
        if services().globals.url_preview_check_root_domain() {
            debug!("Checking root domain");
            match host.split_once('.') {
                None => return false,
                Some((_, root_domain)) => {
                    if allowlist_domain_explicit.contains(&root_domain.to_owned()) {
                        debug!(
                        "Root domain {} is allowed by url_preview_domain_explicit_allowlist (check 1/3)",
                        &root_domain
                    );
                        return true;
                    }

                    if allowlist_domain_contains
                        .iter()
                        .any(|domain_s| domain_s.contains(&root_domain.to_owned()))
                    {
                        debug!(
                    "Root domain {} is allowed by url_preview_domain_contains_allowlist (check 2/3)",
                    &root_domain
                );
                        return true;
                    }
                }
            }
        }
    }

    false
}
