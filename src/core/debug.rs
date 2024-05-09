#![allow(dead_code)] // this is a developer's toolbox

use std::{panic, panic::PanicInfo};

/// Log event at given level in debug-mode (when debug-assertions are enabled).
/// In release-mode it becomes DEBUG level, and possibly subject to elision.
///
/// Release-mode can be simulated in debug-mode builds by enabling the feature
/// 'dev_release_log_level'.
#[macro_export]
macro_rules! debug_event {
	( $level:expr, $($x:tt)+ ) => {
		if cfg!(debug_assertions) && cfg!(not(feature = "dev_release_log_level")) {
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

pub fn set_panic_trap() {
	let next = panic::take_hook();
	panic::set_hook(Box::new(move |info| {
		panic_handler(info, &next);
	}));
}

#[inline(always)]
fn panic_handler(info: &PanicInfo<'_>, next: &dyn Fn(&PanicInfo<'_>)) {
	trap();
	next(info);
}

#[inline(always)]
#[allow(unexpected_cfgs)]
pub fn trap() {
	#[cfg(core_intrinsics)]
	//SAFETY: embeds llvm intrinsic for hardware breakpoint
	unsafe {
		std::intrinsics::breakpoint();
	}

	#[cfg(all(not(core_intrinsics), target_arch = "x86_64"))]
	//SAFETY: embeds instruction for hardware breakpoint
	unsafe {
		std::arch::asm!("int3");
	}
}
