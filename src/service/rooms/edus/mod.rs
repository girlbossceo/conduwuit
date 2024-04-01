pub mod presence;

pub trait Data: presence::Data + 'static {}

pub struct Service {
	pub presence: presence::Service,
}
