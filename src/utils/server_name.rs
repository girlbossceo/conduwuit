//! utilities for doing/checking things with ServerName's/server_name's

use ruma::ServerName;

use crate::services;

/// checks if `server_name` is ours
pub(crate) fn server_is_ours(server_name: &ServerName) -> bool { server_name == services().globals.config.server_name }
