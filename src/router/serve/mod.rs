mod plain;
mod tls;
mod unix;

use std::sync::Arc;

use axum_server::Handle as ServerHandle;
use conduit::{Result, Server};
use tokio::sync::broadcast;

use crate::layers;

/// Serve clients
pub(super) async fn serve(server: Arc<Server>, handle: ServerHandle, shutdown: broadcast::Receiver<()>) -> Result<()> {
	let config = &server.config;
	let addrs = config.get_bind_addrs();
	let app = layers::build(&server)?;

	if cfg!(unix) && config.unix_socket_path.is_some() {
		unix::serve(&server, app, shutdown).await
	} else if config.tls.is_some() {
		tls::serve(&server, app, handle, addrs).await
	} else {
		plain::serve(&server, app, handle, addrs).await
	}
}
