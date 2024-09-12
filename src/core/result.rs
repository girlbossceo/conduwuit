mod debug_inspect;
mod log_debug_err;
mod log_err;
mod map_expect;
mod not_found;

pub use self::{
	debug_inspect::DebugInspect, log_debug_err::LogDebugErr, log_err::LogErr, map_expect::MapExpect,
	not_found::NotFound,
};

pub type Result<T = (), E = crate::Error> = std::result::Result<T, E>;
