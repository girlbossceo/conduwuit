use conduit::{debug, debug_info, error, implement, info};
use ruma::events::room::message::RoomMessageEventContent;
use tokio::time::{sleep, Duration};

use super::console;

/// Possibly spawn the terminal console at startup if configured.
#[implement(super::Service)]
pub(super) async fn console_auto_start(&self) {
	#[cfg(feature = "console")]
	if self.services.server.config.admin_console_automatic {
		// Allow more of the startup sequence to execute before spawning
		tokio::task::yield_now().await;
		self.console.start().await;
	}
}

/// Shutdown the console when the admin worker terminates.
#[implement(super::Service)]
pub(super) async fn console_auto_stop(&self) {
	#[cfg(feature = "console")]
	self.console.close().await;
}

/// Execute admin commands after startup
#[implement(super::Service)]
pub(super) async fn startup_execute(&self) {
	sleep(Duration::from_millis(500)).await; //TODO: remove this after run-states are broadcast
	for (i, command) in self.services.server.config.admin_execute.iter().enumerate() {
		self.startup_execute_command(i, command.clone()).await;
		tokio::task::yield_now().await;
	}

	// The smoketest functionality is placed here for now and simply initiates
	// shutdown after all commands have executed.
	if self.services.server.config.test.contains("smoke") {
		debug_info!("Smoketest mode. All commands complete. Shutting down now...");
		self.services
			.server
			.shutdown()
			.unwrap_or_else(error::default_log);
	}
}

/// Execute one admin command after startup
#[implement(super::Service)]
async fn startup_execute_command(&self, i: usize, command: String) {
	debug!("Startup command #{i}: executing {command:?}");

	match self.command_in_place(command, None).await {
		Err(e) => error!("Startup command #{i} failed: {e:?}"),
		Ok(None) => info!("Startup command #{i} completed (no output)."),
		Ok(Some(output)) => Self::startup_command_output(i, &output),
	}
}

#[cfg(feature = "console")]
#[implement(super::Service)]
fn startup_command_output(i: usize, content: &RoomMessageEventContent) {
	info!("Startup command #{i} completed:");
	console::print(content.body());
}

#[cfg(not(feature = "console"))]
#[implement(super::Service)]
fn startup_command_output(i: usize, content: &RoomMessageEventContent) {
	info!("Startup command #{i} completed:\n{:#?}", content.body());
}
