use std::{any::Any, panic};

// Export debug proc_macros
pub use conduit_macros::recursion_depth;

// Export all of the ancillary tools from here as well.
pub use crate::{result::DebugInspect, utils::debug::*};

/// Log event at given level in debug-mode (when debug-assertions are enabled).
/// In release-mode it becomes DEBUG level, and possibly subject to elision.
///
/// Release-mode can be simulated in debug-mode builds by enabling the feature
/// 'dev_release_log_level'.
#[macro_export]
macro_rules! debug_event {
	( $level:expr, $($x:tt)+ ) => {
		if $crate::debug::logging() {
			::tracing::event!( $level, $($x)+ )
		} else {
			::tracing::debug!( $($x)+ )
		}
	}
}

/// Log message at the ERROR level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_error {
	( $($x:tt)+ ) => {
		$crate::debug_event!(::tracing::Level::ERROR, $($x)+ )
	}
}

/// Log message at the WARN level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_warn {
	( $($x:tt)+ ) => {
		$crate::debug_event!(::tracing::Level::WARN, $($x)+ )
	}
}

/// Log message at the INFO level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_info {
	( $($x:tt)+ ) => {
		$crate::debug_event!(::tracing::Level::INFO, $($x)+ )
	}
}

pub fn set_panic_trap() {
	let next = panic::take_hook();
	panic::set_hook(Box::new(move |info| {
		panic_handler(info, &next);
	}));
}

#[inline(always)]
#[allow(deprecated_in_future)]
fn panic_handler(info: &panic::PanicInfo<'_>, next: &dyn Fn(&panic::PanicInfo<'_>)) {
	trap();
	next(info);
}

#[inline(always)]
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

#[must_use]
pub fn panic_str(p: &Box<dyn Any + Send>) -> &'static str { p.downcast_ref::<&str>().copied().unwrap_or_default() }

#[inline(always)]
#[must_use]
pub fn rttype_name<T: ?Sized>(_: &T) -> &'static str { type_name::<T>() }

#[inline(always)]
#[must_use]
pub fn type_name<T: ?Sized>() -> &'static str { std::any::type_name::<T>() }

#[must_use]
#[inline]
pub const fn logging() -> bool { cfg!(debug_assertions) && cfg!(not(feature = "dev_release_log_level")) }
