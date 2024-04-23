/// Log event at given level in debug-mode (when debug-assertions are enabled).
/// In release-mode it becomes DEBUG level, and possibly subject to elision.
///
/// Release-mode can be simulated in debug-mode builds by enabling the feature
/// 'release_log_level'.
#[macro_export]
macro_rules! debug_event {
	( $level:expr, $($x:tt)+ ) => {
		if cfg!(debug_assertions) && cfg!(not(feature = "release_log_level")) {
			tracing::event!( $level, $($x)+ );
		} else {
			tracing::debug!( $($x)+ );
		}
	}
}

/// Log message at the ERROR level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_error {
	( $($x:tt)+ ) => {
		$crate::debug_event!(tracing::Level::ERROR, $($x)+ );
	}
}

/// Log message at the WARN level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_warn {
	( $($x:tt)+ ) => {
		$crate::debug_event!(tracing::Level::WARN, $($x)+ );
	}
}

/// Log message at the INFO level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_info {
	( $($x:tt)+ ) => {
		$crate::debug_event!(tracing::Level::INFO, $($x)+ );
	}
}
