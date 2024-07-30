mod plain;
mod tls;
mod unix;

use std::sync::Arc;

use axum_server::Handle as ServerHandle;
use conduit::Result;
use conduit_service::Services;
use tokio::sync::broadcast;

use super::layers;

/// Serve clients
pub(super) async fn serve(
	services: Arc<Services>, handle: ServerHandle, shutdown: broadcast::Receiver<()>,
) -> Result<()> {
	let server = &services.server;
	let config = &server.config;
	let addrs = config.get_bind_addrs();
	let (app, _guard) = layers::build(&services)?;

	if cfg!(unix) && config.unix_socket_path.is_some() {
		unix::serve(server, app, shutdown).await
	} else if config.tls.is_some() {
		tls::serve(server, app, handle, addrs).await
	} else {
		plain::serve(server, app, handle, addrs).await
	}
}
