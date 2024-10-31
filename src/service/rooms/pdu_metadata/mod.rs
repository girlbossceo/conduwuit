mod data;
use std::sync::Arc;

use conduit::{
	at,
	utils::{result::FlatOk, stream::ReadyExt, IterStream},
	PduCount, Result,
};
use futures::{FutureExt, StreamExt};
use ruma::{
	api::{client::relations::get_relating_events, Direction},
	events::{relation::RelationType, TimelineEventType},
	EventId, RoomId, UInt, UserId,
};
use serde::Deserialize;

use self::data::{Data, PdusIterItem};
use crate::{rooms, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	short: Dep<rooms::short::Service>,
	state_accessor: Dep<rooms::state_accessor::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

#[derive(Clone, Debug, Deserialize)]
struct ExtractRelType {
	rel_type: RelationType,
}
#[derive(Clone, Debug, Deserialize)]
struct ExtractRelatesToEventId {
	#[serde(rename = "m.relates_to")]
	relates_to: ExtractRelType,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state_accessor: args.depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	#[tracing::instrument(skip(self, from, to), level = "debug")]
	pub fn add_relation(&self, from: PduCount, to: PduCount) {
		match (from, to) {
			(PduCount::Normal(f), PduCount::Normal(t)) => self.db.add_relation(f, t),
			_ => {
				// TODO: Relations with backfilled pdus
			},
		}
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn paginate_relations_with_filter(
		&self, sender_user: &UserId, room_id: &RoomId, target: &EventId, filter_event_type: Option<TimelineEventType>,
		filter_rel_type: Option<RelationType>, from: Option<&str>, to: Option<&str>, limit: Option<UInt>,
		recurse: bool, dir: Direction,
	) -> Result<get_relating_events::v1::Response> {
		let from = from
			.map(PduCount::try_from_string)
			.transpose()?
			.unwrap_or_else(|| match dir {
				Direction::Forward => PduCount::min(),
				Direction::Backward => PduCount::max(),
			});

		let to = to.map(PduCount::try_from_string).flat_ok();

		// Use limit or else 30, with maximum 100
		let limit: usize = limit
			.map(TryInto::try_into)
			.flat_ok()
			.unwrap_or(30)
			.min(100);

		// Spec (v1.10) recommends depth of at least 3
		let depth: u8 = if recurse {
			3
		} else {
			1
		};

		let events: Vec<PdusIterItem> = self
			.get_relations(sender_user, room_id, target, from, limit, depth, dir)
			.await
			.into_iter()
			.filter(|(_, pdu)| {
				filter_event_type
					.as_ref()
					.is_none_or(|kind| *kind == pdu.kind)
			})
			.filter(|(_, pdu)| {
				filter_rel_type.as_ref().is_none_or(|rel_type| {
					pdu.get_content()
						.map(|c: ExtractRelatesToEventId| c.relates_to.rel_type)
						.is_ok_and(|r| r == *rel_type)
				})
			})
			.stream()
			.filter_map(|item| self.visibility_filter(sender_user, item))
			.ready_take_while(|(count, _)| Some(*count) != to)
			.take(limit)
			.collect()
			.boxed()
			.await;

		let next_batch = match dir {
			Direction::Backward => events.first(),
			Direction::Forward => events.last(),
		}
		.map(at!(0))
		.map(|t| t.stringify());

		Ok(get_relating_events::v1::Response {
			next_batch,
			prev_batch: Some(from.stringify()),
			recursion_depth: recurse.then_some(depth.into()),
			chunk: events
				.into_iter()
				.map(at!(1))
				.map(|pdu| pdu.to_message_like_event())
				.collect(),
		})
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn get_relations(
		&self, user_id: &UserId, room_id: &RoomId, target: &EventId, until: PduCount, limit: usize, max_depth: u8,
		dir: Direction,
	) -> Vec<PdusIterItem> {
		let room_id = self.services.short.get_or_create_shortroomid(room_id).await;

		let target = match self.services.timeline.get_pdu_count(target).await {
			Ok(PduCount::Normal(c)) => c,
			// TODO: Support backfilled relations
			_ => 0, // This will result in an empty iterator
		};

		let mut pdus: Vec<_> = self
			.db
			.get_relations(user_id, room_id, target, until, dir)
			.collect()
			.await;

		let mut stack: Vec<_> = pdus.iter().map(|pdu| (pdu.clone(), 1)).collect();

		'limit: while let Some(stack_pdu) = stack.pop() {
			let target = match stack_pdu.0 .0 {
				PduCount::Normal(c) => c,
				// TODO: Support backfilled relations
				PduCount::Backfilled(_) => 0, // This will result in an empty iterator
			};

			let relations: Vec<_> = self
				.db
				.get_relations(user_id, room_id, target, until, dir)
				.collect()
				.await;

			for relation in relations {
				if stack_pdu.1 < max_depth {
					stack.push((relation.clone(), stack_pdu.1.saturating_add(1)));
				}

				pdus.push(relation);
				if pdus.len() >= limit {
					break 'limit;
				}
			}
		}

		pdus
	}

	async fn visibility_filter(&self, sender_user: &UserId, item: PdusIterItem) -> Option<PdusIterItem> {
		let (_, pdu) = &item;

		self.services
			.state_accessor
			.user_can_see_event(sender_user, &pdu.room_id, &pdu.event_id)
			.await
			.then_some(item)
	}

	#[inline]
	#[tracing::instrument(skip_all, level = "debug")]
	pub fn mark_as_referenced(&self, room_id: &RoomId, event_ids: &[Arc<EventId>]) {
		self.db.mark_as_referenced(room_id, event_ids);
	}

	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> bool {
		self.db.is_event_referenced(room_id, event_id).await
	}

	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn mark_event_soft_failed(&self, event_id: &EventId) { self.db.mark_event_soft_failed(event_id) }

	#[inline]
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn is_event_soft_failed(&self, event_id: &EventId) -> bool {
		self.db.is_event_soft_failed(event_id).await
	}
}
