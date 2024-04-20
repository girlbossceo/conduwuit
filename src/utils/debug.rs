/// Log message at the ERROR level in debug-mode (when debug-assertions are
/// enabled). In release mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_error {
	( $($x:tt)+ ) => {
		if cfg!(debug_assertions) {
			error!( $($x)+ );
		} else {
			debug!( $($x)+ );
		}
	}
}

/// Log message at the WARN level in debug-mode (when debug-assertions are
/// enabled). In release mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_warn {
	( $($x:tt)+ ) => {
		if cfg!(debug_assertions) {
			warn!( $($x)+ );
		} else {
			debug!( $($x)+ );
		}
	}
}

/// Log message at the INFO level in debug-mode (when debug-assertions are
/// enabled). In release mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_info {
	( $($x:tt)+ ) => {
		if cfg!(debug_assertions) {
			info!( $($x)+ );
		} else {
			debug!( $($x)+ );
		}
	}
}
