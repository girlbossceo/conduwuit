pub mod capture;
pub mod color;
pub mod fmt;
mod reload;
mod server;

pub use capture::Capture;
pub use reload::{LogLevelReloadHandles, ReloadHandle};
pub use server::Server;
pub use tracing::Level;
pub use tracing_core::{Event, Metadata};

// Wraps for logging macros. Use these macros rather than extern tracing:: or
// log:: crates in project code. ::log and ::tracing can still be used if
// necessary but discouraged. Remember debug_ log macros are also exported to
// the crate namespace like these.

#[macro_export]
macro_rules! error {
    ( $($x:tt)+ ) => { tracing::error!( $($x)+ ); }
}

#[macro_export]
macro_rules! warn {
    ( $($x:tt)+ ) => { tracing::warn!( $($x)+ ); }
}

#[macro_export]
macro_rules! info {
    ( $($x:tt)+ ) => { tracing::info!( $($x)+ ); }
}

#[macro_export]
macro_rules! debug {
    ( $($x:tt)+ ) => { tracing::debug!( $($x)+ ); }
}

#[macro_export]
macro_rules! trace {
    ( $($x:tt)+ ) => { tracing::trace!( $($x)+ ); }
}
