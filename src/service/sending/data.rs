use ruma::ServerName;

use super::{OutgoingDestination, SendingEventType};
use crate::Result;

type OutgoingSendingIter<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, OutgoingDestination, SendingEventType)>> + 'a>;
type SendingEventTypeIter<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, SendingEventType)>> + 'a>;

pub trait Data: Send + Sync {
	fn active_requests(&self) -> OutgoingSendingIter<'_>;
	fn active_requests_for(&self, outgoing_kind: &OutgoingDestination) -> SendingEventTypeIter<'_>;
	fn delete_active_request(&self, key: Vec<u8>) -> Result<()>;
	fn delete_all_active_requests_for(&self, outgoing_kind: &OutgoingDestination) -> Result<()>;
	fn delete_all_requests_for(&self, outgoing_kind: &OutgoingDestination) -> Result<()>;
	fn queue_requests(&self, requests: &[(&OutgoingDestination, SendingEventType)]) -> Result<Vec<Vec<u8>>>;
	fn queued_requests<'a>(
		&'a self, outgoing_kind: &OutgoingDestination,
	) -> Box<dyn Iterator<Item = Result<(SendingEventType, Vec<u8>)>> + 'a>;
	fn mark_as_active(&self, events: &[(SendingEventType, Vec<u8>)]) -> Result<()>;
	fn set_latest_educount(&self, server_name: &ServerName, educount: u64) -> Result<()>;
	fn get_latest_educount(&self, server_name: &ServerName) -> Result<u64>;
}
