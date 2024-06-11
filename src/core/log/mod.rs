pub mod color;
mod reload;

pub use reload::ReloadHandle;
pub use reload::LogLevelReloadHandles;

// Wraps for logging macros. Use these macros rather than extern tracing:: or log:: crates in
// project code. ::log and ::tracing can still be used if necessary but discouraged. Remember
// debug_ log macros are also exported to the crate namespace like these.

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
