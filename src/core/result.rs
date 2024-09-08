mod debug_inspect;
mod log_debug_err;
mod log_err;
mod map_expect;

pub use self::{debug_inspect::DebugInspect, log_debug_err::LogDebugErr, log_err::LogErr, map_expect::MapExpect};

pub type Result<T = (), E = crate::Error> = std::result::Result<T, E>;
