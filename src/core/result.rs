mod debug_inspect;
mod map_expect;

pub use self::{debug_inspect::DebugInspect, map_expect::MapExpect};

pub type Result<T = (), E = crate::Error> = std::result::Result<T, E>;
