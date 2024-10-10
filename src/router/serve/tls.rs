use std::{net::SocketAddr, sync::Arc};

use axum::Router;
use axum_server::Handle as ServerHandle;
use axum_server_dual_protocol::{
	axum_server::{bind_rustls, tls_rustls::RustlsConfig},
	ServerExt,
};
use conduit::{Result, Server};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

pub(super) async fn serve(
	server: &Arc<Server>, app: Router, handle: ServerHandle, addrs: Vec<SocketAddr>,
) -> Result<()> {
	let config = &server.config;
	let tls = config.tls.as_ref().expect("TLS configuration");
	let certs = &tls.certs;
	let key = &tls.key;

	// we use ring for ruma and hashing state, but aws-lc-rs is the new default.
	// without this, TLS mode will panic.
	rustls::crypto::aws_lc_rs::default_provider()
		.install_default()
		.expect("failed to initialise aws-lc-rs rustls crypto provider");

	debug!("Using direct TLS. Certificate path {certs} and certificate private key path {key}",);
	info!(
		"Note: It is strongly recommended that you use a reverse proxy instead of running conduwuit directly with TLS."
	);
	let conf = RustlsConfig::from_pem_file(certs, key).await?;

	let mut join_set = JoinSet::new();
	let app = app.into_make_service_with_connect_info::<SocketAddr>();
	if tls.dual_protocol {
		for addr in &addrs {
			join_set.spawn_on(
				axum_server_dual_protocol::bind_dual_protocol(*addr, conf.clone())
					.set_upgrade(false)
					.handle(handle.clone())
					.serve(app.clone()),
				server.runtime(),
			);
		}
	} else {
		for addr in &addrs {
			join_set.spawn_on(
				bind_rustls(*addr, conf.clone())
					.handle(handle.clone())
					.serve(app.clone()),
				server.runtime(),
			);
		}
	}

	if tls.dual_protocol {
		warn!(
			"Listening on {addrs:?} with TLS certificate {certs} and supporting plain text (HTTP) connections too \
			 (insecure!)",
		);
	} else {
		info!("Listening on {addrs:?} with TLS certificate {certs}");
	}

	while join_set.join_next().await.is_some() {}

	Ok(())
}
