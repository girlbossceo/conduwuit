use std::sync::Arc;

use super::{capture, reload::LogLevelReloadHandles};

/// Logging subsystem. This is a singleton member of super::Server which holds
/// all logging and tracing related state rather than shoving it all in
/// super::Server directly.
pub struct Server {
	/// General log level reload handles.
	pub reload: LogLevelReloadHandles,

	/// Tracing capture state for ephemeral/oneshot uses.
	pub capture: Arc<capture::State>,
}
