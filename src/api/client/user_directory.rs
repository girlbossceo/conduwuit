use axum::extract::State;
use futures::{pin_mut, StreamExt};
use ruma::{
	api::client::user_directory::search_users,
	events::{
		room::join_rules::{JoinRule, RoomJoinRulesEventContent},
		StateEventType,
	},
};

use crate::{Result, Ruma};

/// # `POST /_matrix/client/r0/user_directory/search`
///
/// Searches all known users for a match.
///
/// - Hides any local users that aren't in any public rooms (i.e. those that
///   have the join rule set to public) and don't share a room with the sender
pub(crate) async fn search_users_route(
	State(services): State<crate::State>, body: Ruma<search_users::v3::Request>,
) -> Result<search_users::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let limit = usize::try_from(body.limit).unwrap_or(10); // default limit is 10

	let users = services.users.stream().filter_map(|user_id| async {
		// Filter out buggy users (they should not exist, but you never know...)
		let user = search_users::v3::User {
			user_id: user_id.to_owned(),
			display_name: services.users.displayname(user_id).await.ok(),
			avatar_url: services.users.avatar_url(user_id).await.ok(),
		};

		let user_id_matches = user
			.user_id
			.to_string()
			.to_lowercase()
			.contains(&body.search_term.to_lowercase());

		let user_displayname_matches = user
			.display_name
			.as_ref()
			.filter(|name| {
				name.to_lowercase()
					.contains(&body.search_term.to_lowercase())
			})
			.is_some();

		if !user_id_matches && !user_displayname_matches {
			return None;
		}

		// It's a matching user, but is the sender allowed to see them?
		let mut user_visible = false;

		let user_is_in_public_rooms = services
			.rooms
			.state_cache
			.rooms_joined(&user.user_id)
			.any(|room| async move {
				services
					.rooms
					.state_accessor
					.room_state_get(room, &StateEventType::RoomJoinRules, "")
					.await
					.map_or(false, |event| {
						serde_json::from_str(event.content.get())
							.map_or(false, |r: RoomJoinRulesEventContent| r.join_rule == JoinRule::Public)
					})
			})
			.await;

		if user_is_in_public_rooms {
			user_visible = true;
		} else {
			let user_is_in_shared_rooms = services
				.rooms
				.user
				.has_shared_rooms(sender_user, &user.user_id)
				.await;

			if user_is_in_shared_rooms {
				user_visible = true;
			}
		}

		user_visible.then_some(user)
	});

	pin_mut!(users);

	let limited = users.by_ref().next().await.is_some();

	let results = users.take(limit).collect().await;

	Ok(search_users::v3::Response {
		results,
		limited,
	})
}
