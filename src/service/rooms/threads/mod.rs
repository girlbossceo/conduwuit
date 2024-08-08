mod data;

use std::{collections::BTreeMap, sync::Arc};

use conduit::{err, PduEvent, Result};
use data::Data;
use futures::Stream;
use ruma::{
	api::client::threads::get_threads::v1::IncludeThreads, events::relation::BundledThread, uint, CanonicalJsonValue,
	EventId, RoomId, UserId,
};
use serde_json::json;

use crate::{rooms, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	timeline: Dep<rooms::timeline::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub async fn threads_until<'a>(
		&'a self, user_id: &'a UserId, room_id: &'a RoomId, until: u64, include: &'a IncludeThreads,
	) -> Result<impl Stream<Item = (u64, PduEvent)> + Send + 'a> {
		self.db
			.threads_until(user_id, room_id, until, include)
			.await
	}

	pub async fn add_to_thread(&self, root_event_id: &EventId, pdu: &PduEvent) -> Result<()> {
		let root_id = self
			.services
			.timeline
			.get_pdu_id(root_event_id)
			.await
			.map_err(|e| err!(Request(InvalidParam("Invalid event_id in thread message: {e:?}"))))?;

		let root_pdu = self
			.services
			.timeline
			.get_pdu_from_id(&root_id)
			.await
			.map_err(|e| err!(Request(InvalidParam("Thread root not found: {e:?}"))))?;

		let mut root_pdu_json = self
			.services
			.timeline
			.get_pdu_json_from_id(&root_id)
			.await
			.map_err(|e| err!(Request(InvalidParam("Thread root pdu not found: {e:?}"))))?;

		if let CanonicalJsonValue::Object(unsigned) = root_pdu_json
			.entry("unsigned".to_owned())
			.or_insert_with(|| CanonicalJsonValue::Object(BTreeMap::default()))
		{
			if let Some(mut relations) = unsigned
				.get("m.relations")
				.and_then(|r| r.as_object())
				.and_then(|r| r.get("m.thread"))
				.and_then(|relations| serde_json::from_value::<BundledThread>(relations.clone().into()).ok())
			{
				// Thread already existed
				relations.count = relations.count.saturating_add(uint!(1));
				relations.latest_event = pdu.to_message_like_event();

				let content = serde_json::to_value(relations).expect("to_value always works");

				unsigned.insert(
					"m.relations".to_owned(),
					json!({ "m.thread": content })
						.try_into()
						.expect("thread is valid json"),
				);
			} else {
				// New thread
				let relations = BundledThread {
					latest_event: pdu.to_message_like_event(),
					count: uint!(1),
					current_user_participated: true,
				};

				let content = serde_json::to_value(relations).expect("to_value always works");

				unsigned.insert(
					"m.relations".to_owned(),
					json!({ "m.thread": content })
						.try_into()
						.expect("thread is valid json"),
				);
			}

			self.services
				.timeline
				.replace_pdu(&root_id, &root_pdu_json, &root_pdu)
				.await?;
		}

		let mut users = Vec::new();
		if let Ok(userids) = self.db.get_participants(&root_id).await {
			users.extend_from_slice(&userids);
		} else {
			users.push(root_pdu.sender);
		}
		users.push(pdu.sender.clone());

		self.db.update_participants(&root_id, &users)
	}
}
