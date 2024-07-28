use std::{ops::Deref, sync::Arc};

use conduit_service::Services;

#[derive(Clone)]
pub struct State {
	services: Arc<Services>,
}

impl State {
	pub fn new(services: Arc<Services>) -> Self {
		Self {
			services,
		}
	}
}

impl Deref for State {
	type Target = Arc<Services>;

	fn deref(&self) -> &Self::Target { &self.services }
}
