use std::time::SystemTime;

use conduit_service::Services;

pub(crate) struct Command<'a> {
	pub(crate) services: &'a Services,
	pub(crate) body: &'a [&'a str],
	pub(crate) timer: SystemTime,
}
