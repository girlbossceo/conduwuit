mod debug_inspect;
mod log_debug_err;
mod log_err;
mod map_expect;
mod not_found;
mod unwrap_infallible;

pub use self::{
	debug_inspect::DebugInspect, log_debug_err::LogDebugErr, log_err::LogErr, map_expect::MapExpect,
	not_found::NotFound, unwrap_infallible::UnwrapInfallible,
};

pub type Result<T = (), E = crate::Error> = std::result::Result<T, E>;
