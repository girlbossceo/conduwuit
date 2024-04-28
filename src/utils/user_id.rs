//! utilities for doing things with UserId's / usernames

use ruma::UserId;

use crate::services;

/// checks if `user_id` is local to us via server_name comparison
pub(crate) fn user_is_local(user_id: &UserId) -> bool { user_id.server_name() == services().globals.config.server_name }
