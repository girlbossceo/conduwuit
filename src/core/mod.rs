pub mod alloc;
pub mod config;
pub mod debug;
pub mod error;
pub mod info;
pub mod log;
pub mod metrics;
pub mod mods;
pub mod pdu;
pub mod result;
pub mod server;
pub mod utils;

pub use ::http;
pub use ::ruma;
pub use ::toml;
pub use ::tracing;
pub use config::Config;
pub use error::Error;
pub use info::{rustc_flags_capture, version, version::version};
pub use pdu::{PduBuilder, PduCount, PduEvent};
pub use result::Result;
pub use server::Server;
pub use utils::{ctor, dtor, implement};

pub use crate as conduit_core;

rustc_flags_capture! {}

#[cfg(not(conduit_mods))]
pub mod mods {
	#[macro_export]
	macro_rules! mod_ctor {
		() => {};
	}
	#[macro_export]
	macro_rules! mod_dtor {
		() => {};
	}
}

/// Functor for falsy
#[macro_export]
macro_rules! is_false {
	() => {
		|x| !x
	};
}

/// Functor for truthy
#[macro_export]
macro_rules! is_true {
	() => {
		|x| !!x
	};
}

/// Functor for equality to zero
#[macro_export]
macro_rules! is_zero {
	() => {
		$crate::is_matching!(0)
	};
}

/// Functor for equality i.e. .is_some_and(is_equal!(2))
#[macro_export]
macro_rules! is_equal_to {
	($val:expr) => {
		|x| x == $val
	};
}

/// Functor for less i.e. .is_some_and(is_less_than!(2))
#[macro_export]
macro_rules! is_less_than {
	($val:expr) => {
		|x| x < $val
	};
}

/// Functor for matches! i.e. .is_some_and(is_matching!('A'..='Z'))
#[macro_export]
macro_rules! is_matching {
	($val:expr) => {
		|x| matches!(x, $val)
	};
}

/// Functor for !is_empty()
#[macro_export]
macro_rules! is_not_empty {
	() => {
		|x| !x.is_empty()
	};
}
