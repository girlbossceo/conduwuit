mod debug_inspect;
mod flat_ok;
mod into_is_ok;
mod log_debug_err;
mod log_err;
mod map_expect;
mod not_found;
mod unwrap_infallible;

pub use self::{
	debug_inspect::DebugInspect, flat_ok::FlatOk, into_is_ok::IntoIsOk, log_debug_err::LogDebugErr, log_err::LogErr,
	map_expect::MapExpect, not_found::NotFound, unwrap_infallible::UnwrapInfallible,
};

pub type Result<T = (), E = crate::Error> = std::result::Result<T, E>;
