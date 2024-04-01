pub mod presence;
pub mod typing;

pub trait Data: presence::Data + 'static {}

pub struct Service {
	pub presence: presence::Service,
	pub typing: typing::Service,
}
