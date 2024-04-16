use ruma::{events::presence::PresenceEvent, presence::PresenceState, OwnedUserId, UInt, UserId};

use crate::Result;

pub trait Data: Send + Sync {
	/// Returns the latest presence event for the given user.
	fn get_presence(&self, user_id: &UserId) -> Result<Option<(u64, PresenceEvent)>>;

	/// Adds a presence event which will be saved until a new event replaces it.
	fn set_presence(
		&self, user_id: &UserId, presence_state: &PresenceState, currently_active: Option<bool>,
		last_active_ago: Option<UInt>, status_msg: Option<String>,
	) -> Result<()>;

	/// Removes the presence record for the given user from the database.
	fn remove_presence(&self, user_id: &UserId) -> Result<()>;

	/// Returns the most recent presence updates that happened after the event
	/// with id `since`.
	fn presence_since<'a>(&'a self, since: u64) -> Box<dyn Iterator<Item = (OwnedUserId, u64, Vec<u8>)> + 'a>;
}
