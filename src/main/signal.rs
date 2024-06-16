use std::sync::Arc;

use conduit::{debug_error, trace, warn};
use tokio::signal;

use super::server::Server;

#[cfg(unix)]
#[tracing::instrument(skip_all)]
pub(super) async fn signal(server: Arc<Server>) {
	use signal::unix;
	use unix::SignalKind;

	const CONSOLE: bool = cfg!(feature = "console");
	const RELOADING: bool = cfg!(all(conduit_mods, not(CONSOLE)));

	let mut quit = unix::signal(SignalKind::quit()).expect("SIGQUIT handler");
	let mut term = unix::signal(SignalKind::terminate()).expect("SIGTERM handler");
	loop {
		trace!("Installed signal handlers");
		let sig: &'static str;
		tokio::select! {
			_ = signal::ctrl_c() => { sig = "SIGINT"; },
			_ = quit.recv() => { sig = "SIGQUIT"; },
			_ = term.recv() => { sig = "SIGTERM"; },
		}

		warn!("Received {sig}");
		let result = if RELOADING && sig == "SIGINT" {
			server.server.reload()
		} else if matches!(sig, "SIGQUIT" | "SIGTERM") || (!CONSOLE && sig == "SIGINT") {
			server.server.shutdown()
		} else {
			server.server.signal(sig)
		};

		if let Err(e) = result {
			debug_error!(?sig, "signal: {e}");
		}
	}
}

#[cfg(not(unix))]
#[tracing::instrument(skip_all)]
pub(super) async fn signal(server: Arc<Server>) {
	loop {
		tokio::select! {
			_ = signal::ctrl_c() => {
				warn!("Received Ctrl+C");
				if let Err(e) = server.server.signal.send("SIGINT") {
					debug_error!("signal channel: {e}");
				}
			},
		}
	}
}
