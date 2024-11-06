mod aliases;
mod create;
mod event;
mod upgrade;

pub(crate) use self::{
	aliases::get_room_aliases_route, create::create_room_route, event::get_room_event_route,
	upgrade::upgrade_room_route,
};
