use std::sync::Arc;

use tracing_subscriber::{reload, EnvFilter};

/// We need to store a reload::Handle value, but can't name it's type explicitly
/// because the S type parameter depends on the subscriber's previous layers. In
/// our case, this includes unnameable 'impl Trait' types.
///
/// This is fixed[1] in the unreleased tracing-subscriber from the master
/// branch, which removes the S parameter. Unfortunately can't use it without
/// pulling in a version of tracing that's incompatible with the rest of our
/// deps.
///
/// To work around this, we define an trait without the S paramter that forwards
/// to the reload::Handle::reload method, and then store the handle as a trait
/// object.
///
/// [1]: <https://github.com/tokio-rs/tracing/pull/1035/commits/8a87ea52425098d3ef8f56d92358c2f6c144a28f>
pub trait ReloadHandle<L> {
	fn reload(&self, new_value: L) -> Result<(), reload::Error>;
}

impl<L, S> ReloadHandle<L> for reload::Handle<L, S> {
	fn reload(&self, new_value: L) -> Result<(), reload::Error> { reload::Handle::reload(self, new_value) }
}

struct LogLevelReloadHandlesInner {
	handles: Vec<Box<dyn ReloadHandle<EnvFilter> + Send + Sync>>,
}

/// Wrapper to allow reloading the filter on several several
/// [`tracing_subscriber::reload::Handle`]s at once, with the same value.
#[derive(Clone)]
pub struct LogLevelReloadHandles {
	inner: Arc<LogLevelReloadHandlesInner>,
}

impl LogLevelReloadHandles {
	#[must_use]
	pub fn new(handles: Vec<Box<dyn ReloadHandle<EnvFilter> + Send + Sync>>) -> LogLevelReloadHandles {
		LogLevelReloadHandles {
			inner: Arc::new(LogLevelReloadHandlesInner {
				handles,
			}),
		}
	}

	pub fn reload(&self, new_value: &EnvFilter) -> Result<(), reload::Error> {
		for handle in &self.inner.handles {
			handle.reload(new_value.clone())?;
		}
		Ok(())
	}
}

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
