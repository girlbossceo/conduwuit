use std::sync::Arc;

use conduit::{implement, utils::stream::TryIgnore, Result};
use database::{Ignore, Map};
use futures::{Stream, StreamExt};
use ruma::RoomId;

pub struct Service {
	db: Data,
}

struct Data {
	publicroomids: Arc<Map>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data {
				publicroomids: args.db["publicroomids"].clone(),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
pub fn set_public(&self, room_id: &RoomId) { self.db.publicroomids.insert(room_id.as_bytes(), &[]); }

#[implement(Service)]
pub fn set_not_public(&self, room_id: &RoomId) { self.db.publicroomids.remove(room_id.as_bytes()); }

#[implement(Service)]
pub async fn is_public_room(&self, room_id: &RoomId) -> bool { self.db.publicroomids.qry(room_id).await.is_ok() }

#[implement(Service)]
pub fn public_rooms(&self) -> impl Stream<Item = &RoomId> + Send {
	self.db
		.publicroomids
		.keys()
		.ignore_err()
		.map(|(room_id, _): (&RoomId, Ignore)| room_id)
}
