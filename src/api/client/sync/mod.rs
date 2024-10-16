mod v3;
mod v4;

use conduit::{
	utils::{math::usize_from_u64_truncated, ReadyExt},
	PduCount,
};
use futures::StreamExt;
use ruma::{RoomId, UserId};

pub(crate) use self::{v3::sync_events_route, v4::sync_events_v4_route};
use crate::{service::Services, Error, PduEvent, Result};

async fn load_timeline(
	services: &Services, sender_user: &UserId, room_id: &RoomId, roomsincecount: PduCount, limit: u64,
) -> Result<(Vec<(PduCount, PduEvent)>, bool), Error> {
	let timeline_pdus;
	let limited = if services
		.rooms
		.timeline
		.last_timeline_count(sender_user, room_id)
		.await?
		> roomsincecount
	{
		let mut non_timeline_pdus = services
			.rooms
			.timeline
			.pdus_until(sender_user, room_id, PduCount::max())
			.await?
			.ready_take_while(|(pducount, _)| pducount > &roomsincecount);

		// Take the last events for the timeline
		timeline_pdus = non_timeline_pdus
			.by_ref()
			.take(usize_from_u64_truncated(limit))
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.rev()
			.collect::<Vec<_>>();

		// They /sync response doesn't always return all messages, so we say the output
		// is limited unless there are events in non_timeline_pdus
		non_timeline_pdus.next().await.is_some()
	} else {
		timeline_pdus = Vec::new();
		false
	};
	Ok((timeline_pdus, limited))
}

async fn share_encrypted_room(
	services: &Services, sender_user: &UserId, user_id: &UserId, ignore_room: Option<&RoomId>,
) -> bool {
	services
		.rooms
		.user
		.get_shared_rooms(sender_user, user_id)
		.ready_filter(|&room_id| Some(room_id) != ignore_room)
		.any(|other_room_id| {
			services
				.rooms
				.state_accessor
				.is_encrypted_room(other_room_id)
		})
		.await
}
