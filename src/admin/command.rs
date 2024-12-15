use std::time::SystemTime;

use conduwuit_service::Services;
use ruma::EventId;

pub(crate) struct Command<'a> {
	pub(crate) services: &'a Services,
	pub(crate) body: &'a [&'a str],
	pub(crate) timer: SystemTime,
	pub(crate) reply_id: Option<&'a EventId>,
}
