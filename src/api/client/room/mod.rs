mod aliases;
mod create;
mod event;
mod initial_sync;
mod upgrade;

pub(crate) use self::{
	aliases::get_room_aliases_route, create::create_room_route, event::get_room_event_route,
	initial_sync::room_initial_sync_route, upgrade::upgrade_room_route,
};
