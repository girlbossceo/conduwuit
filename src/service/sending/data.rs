use ruma::ServerName;

use super::{Destination, SendingEvent};
use crate::Result;

type OutgoingSendingIter<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, Destination, SendingEvent)>> + 'a>;
type SendingEventIter<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, SendingEvent)>> + 'a>;

pub(crate) trait Data: Send + Sync {
	fn active_requests(&self) -> OutgoingSendingIter<'_>;
	fn active_requests_for(&self, destination: &Destination) -> SendingEventIter<'_>;
	fn delete_active_request(&self, key: Vec<u8>) -> Result<()>;
	fn delete_all_active_requests_for(&self, destination: &Destination) -> Result<()>;

	/// TODO: use this?
	#[allow(dead_code)]
	fn delete_all_requests_for(&self, destination: &Destination) -> Result<()>;
	fn queue_requests(&self, requests: &[(&Destination, SendingEvent)]) -> Result<Vec<Vec<u8>>>;
	fn queued_requests<'a>(
		&'a self, destination: &Destination,
	) -> Box<dyn Iterator<Item = Result<(SendingEvent, Vec<u8>)>> + 'a>;
	fn mark_as_active(&self, events: &[(SendingEvent, Vec<u8>)]) -> Result<()>;
	fn set_latest_educount(&self, server_name: &ServerName, educount: u64) -> Result<()>;
	fn get_latest_educount(&self, server_name: &ServerName) -> Result<u64>;
}
