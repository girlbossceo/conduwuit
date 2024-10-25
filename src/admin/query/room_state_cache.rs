use clap::Subcommand;
use conduit::Result;
use futures::StreamExt;
use ruma::{events::room::message::RoomMessageEventContent, RoomId, ServerName, UserId};

use crate::Command;

#[derive(Debug, Subcommand)]
pub(crate) enum RoomStateCacheCommand {
	ServerInRoom {
		server: Box<ServerName>,
		room_id: Box<RoomId>,
	},

	RoomServers {
		room_id: Box<RoomId>,
	},

	ServerRooms {
		server: Box<ServerName>,
	},

	RoomMembers {
		room_id: Box<RoomId>,
	},

	LocalUsersInRoom {
		room_id: Box<RoomId>,
	},

	ActiveLocalUsersInRoom {
		room_id: Box<RoomId>,
	},

	RoomJoinedCount {
		room_id: Box<RoomId>,
	},

	RoomInvitedCount {
		room_id: Box<RoomId>,
	},

	RoomUserOnceJoined {
		room_id: Box<RoomId>,
	},

	RoomMembersInvited {
		room_id: Box<RoomId>,
	},

	GetInviteCount {
		room_id: Box<RoomId>,
		user_id: Box<UserId>,
	},

	GetLeftCount {
		room_id: Box<RoomId>,
		user_id: Box<UserId>,
	},

	RoomsJoined {
		user_id: Box<UserId>,
	},

	RoomsLeft {
		user_id: Box<UserId>,
	},

	RoomsInvited {
		user_id: Box<UserId>,
	},

	InviteState {
		user_id: Box<UserId>,
		room_id: Box<RoomId>,
	},
}

pub(super) async fn process(
	subcommand: RoomStateCacheCommand, context: &Command<'_>,
) -> Result<RoomMessageEventContent> {
	let services = context.services;

	match subcommand {
		RoomStateCacheCommand::ServerInRoom {
			server,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let result = services
				.rooms
				.state_cache
				.server_in_room(&server, &room_id)
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{result:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomServers {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.room_servers(&room_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::ServerRooms {
			server,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.server_rooms(&server)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomMembers {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.room_members(&room_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::LocalUsersInRoom {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.local_users_in_room(&room_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::ActiveLocalUsersInRoom {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.active_local_users_in_room(&room_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomJoinedCount {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services.rooms.state_cache.room_joined_count(&room_id).await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomInvitedCount {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.rooms
				.state_cache
				.room_invited_count(&room_id)
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomUserOnceJoined {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.room_useroncejoined(&room_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomMembersInvited {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.room_members_invited(&room_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::GetInviteCount {
			room_id,
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.rooms
				.state_cache
				.get_invite_count(&room_id, &user_id)
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::GetLeftCount {
			room_id,
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.rooms
				.state_cache
				.get_left_count(&room_id, &user_id)
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomsJoined {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.rooms_joined(&user_id)
				.map(ToOwned::to_owned)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomsInvited {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.rooms_invited(&user_id)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::RoomsLeft {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results: Vec<_> = services
				.rooms
				.state_cache
				.rooms_left(&user_id)
				.collect()
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		RoomStateCacheCommand::InviteState {
			user_id,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.rooms
				.state_cache
				.invite_state(&user_id, &room_id)
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
	}
}
