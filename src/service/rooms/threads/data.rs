use std::{mem::size_of, sync::Arc};

use conduit::{
	checked,
	result::LogErr,
	utils,
	utils::{stream::TryIgnore, ReadyExt},
	PduEvent, Result,
};
use database::{Deserialized, Map};
use futures::{Stream, StreamExt};
use ruma::{api::client::threads::get_threads::v1::IncludeThreads, OwnedUserId, RoomId, UserId};

use crate::{rooms, Dep};

pub(super) struct Data {
	threadid_userids: Arc<Map>,
	services: Services,
}

struct Services {
	short: Dep<rooms::short::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			threadid_userids: db["threadid_userids"].clone(),
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
		}
	}

	pub(super) async fn threads_until<'a>(
		&'a self, user_id: &'a UserId, room_id: &'a RoomId, until: u64, _include: &'a IncludeThreads,
	) -> Result<impl Stream<Item = (u64, PduEvent)> + Send + 'a> {
		let prefix = self
			.services
			.short
			.get_shortroomid(room_id)
			.await?
			.to_be_bytes()
			.to_vec();

		let mut current = prefix.clone();
		current.extend_from_slice(&(checked!(until - 1)?).to_be_bytes());

		let stream = self
			.threadid_userids
			.rev_raw_keys_from(&current)
			.ignore_err()
			.ready_take_while(move |key| key.starts_with(&prefix))
			.map(|pduid| (utils::u64_from_u8(&pduid[(size_of::<u64>())..]), pduid))
			.filter_map(move |(count, pduid)| async move {
				let mut pdu = self.services.timeline.get_pdu_from_id(pduid).await.ok()?;

				if pdu.sender != user_id {
					pdu.remove_transaction_id().log_err().ok();
				}

				Some((count, pdu))
			});

		Ok(stream)
	}

	pub(super) fn update_participants(&self, root_id: &[u8], participants: &[OwnedUserId]) -> Result<()> {
		let users = participants
			.iter()
			.map(|user| user.as_bytes())
			.collect::<Vec<_>>()
			.join(&[0xFF][..]);

		self.threadid_userids.insert(root_id, &users);

		Ok(())
	}

	pub(super) async fn get_participants(&self, root_id: &[u8]) -> Result<Vec<OwnedUserId>> {
		self.threadid_userids.qry(root_id).await.deserialized()
	}
}
