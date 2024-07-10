#![cfg(feature = "sentry_telemetry")]

use std::{str::FromStr, sync::Arc};

use conduit::{config::Config, trace};
use sentry::{
	types::{protocol::v7::Event, Dsn},
	Breadcrumb, ClientOptions,
};

pub(crate) fn init(config: &Config) -> Option<sentry::ClientInitGuard> {
	config.sentry.then(|| sentry::init(options(config)))
}

fn options(config: &Config) -> ClientOptions {
	let dsn = config
		.sentry_endpoint
		.as_ref()
		.expect("init_sentry should only be called if sentry is enabled and this is not None")
		.as_str();

	ClientOptions {
		dsn: Some(Dsn::from_str(dsn).expect("sentry_endpoint must be a valid URL")),
		server_name: config
			.sentry_send_server_name
			.then(|| config.server_name.to_string().into()),
		traces_sample_rate: config.sentry_traces_sample_rate,
		debug: cfg!(debug_assertions),
		release: sentry::release_name!(),
		user_agent: conduit::version::user_agent().into(),
		attach_stacktrace: config.sentry_attach_stacktrace,
		before_send: Some(Arc::new(before_send)),
		before_breadcrumb: Some(Arc::new(before_breadcrumb)),
		..Default::default()
	}
}

fn before_send(event: Event<'static>) -> Option<Event<'static>> {
	trace!("Sending sentry event: {event:?}");
	Some(event)
}

fn before_breadcrumb(crumb: Breadcrumb) -> Option<Breadcrumb> {
	trace!("Adding sentry breadcrumb: {crumb:?}");
	Some(crumb)
}
