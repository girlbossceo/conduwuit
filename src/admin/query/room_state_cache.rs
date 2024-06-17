use ruma::events::room::message::RoomMessageEventContent;

use super::RoomStateCache;
use crate::{services, Result};

pub(super) async fn room_state_cache(subcommand: RoomStateCache) -> Result<RoomMessageEventContent> {
	match subcommand {
		RoomStateCache::ServerInRoom {
			server,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let result = services()
				.rooms
				.state_cache
				.server_in_room(&server, &room_id);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{result:#?}\n```"
			)))
		},
		RoomStateCache::RoomServers {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services()
				.rooms
				.state_cache
				.room_servers(&room_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::ServerRooms {
			server,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services().rooms.state_cache.server_rooms(&server).collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomMembers {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services()
				.rooms
				.state_cache
				.room_members(&room_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::LocalUsersInRoom {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services()
				.rooms
				.state_cache
				.local_users_in_room(&room_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::ActiveLocalUsersInRoom {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services()
				.rooms
				.state_cache
				.active_local_users_in_room(&room_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomJoinedCount {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().rooms.state_cache.room_joined_count(&room_id);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomInvitedCount {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().rooms.state_cache.room_invited_count(&room_id);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomUserOnceJoined {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services()
				.rooms
				.state_cache
				.room_useroncejoined(&room_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomMembersInvited {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services()
				.rooms
				.state_cache
				.room_members_invited(&room_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::GetInviteCount {
			room_id,
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.rooms
				.state_cache
				.get_invite_count(&room_id, &user_id);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::GetLeftCount {
			room_id,
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.rooms
				.state_cache
				.get_left_count(&room_id, &user_id);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomsJoined {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services()
				.rooms
				.state_cache
				.rooms_joined(&user_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomsInvited {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services()
				.rooms
				.state_cache
				.rooms_invited(&user_id)
				.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::RoomsLeft {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Result<Vec<_>> = services().rooms.state_cache.rooms_left(&user_id).collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCache::InviteState {
			user_id,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.rooms
				.state_cache
				.invite_state(&user_id, &room_id);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
	}
}
