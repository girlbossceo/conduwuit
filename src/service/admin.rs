use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use conduit::{Error, Result};
use ruma::{
	api::client::error::ErrorKind,
	events::{
		room::{
			canonical_alias::RoomCanonicalAliasEventContent,
			create::RoomCreateEventContent,
			guest_access::{GuestAccess, RoomGuestAccessEventContent},
			history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
			join_rules::{JoinRule, RoomJoinRulesEventContent},
			member::{MembershipState, RoomMemberEventContent},
			message::RoomMessageEventContent,
			name::RoomNameEventContent,
			power_levels::RoomPowerLevelsEventContent,
			topic::RoomTopicEventContent,
		},
		TimelineEventType,
	},
	EventId, OwnedRoomAliasId, OwnedRoomId, OwnedUserId, RoomAliasId, RoomId, RoomVersionId, UserId,
};
use serde_json::value::to_raw_value;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::{error, warn};

use crate::{pdu::PduBuilder, services};

pub type HandlerResult = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
pub type Handler = fn(AdminRoomEvent, OwnedRoomId, OwnedUserId) -> HandlerResult;

pub struct Service {
	sender: loole::Sender<AdminRoomEvent>,
	receiver: Mutex<loole::Receiver<AdminRoomEvent>>,
	handler_join: Mutex<Option<JoinHandle<()>>>,
	pub handle: Mutex<Option<Handler>>,
}

#[derive(Debug)]
pub enum AdminRoomEvent {
	ProcessMessage(String, Arc<EventId>),
	SendMessage(RoomMessageEventContent),
}

impl Service {
	#[must_use]
	pub fn build() -> Arc<Self> {
		let (sender, receiver) = loole::unbounded();
		Arc::new(Self {
			sender,
			receiver: Mutex::new(receiver),
			handler_join: Mutex::new(None),
			handle: Mutex::new(None),
		})
	}

	pub async fn start_handler(self: &Arc<Self>) {
		let self_ = Arc::clone(self);
		let handle = services().server.runtime().spawn(async move {
			self_
				.handler()
				.await
				.expect("Failed to initialize admin room handler");
		});

		_ = self.handler_join.lock().await.insert(handle);
	}

	async fn handler(self: &Arc<Self>) -> Result<()> {
		let receiver = self.receiver.lock().await;
		let Ok(Some(admin_room)) = Self::get_admin_room().await else {
			return Ok(());
		};
		let server_name = services().globals.server_name();
		let server_user = UserId::parse(format!("@conduit:{server_name}")).expect("server's username is valid");

		loop {
			debug_assert!(!receiver.is_closed(), "channel closed");
			tokio::select! {
				event = receiver.recv_async() => match event {
					Ok(event) => self.receive(event, &admin_room, &server_user).await?,
					Err(_e) => return Ok(()),
				}
			}
		}
	}

	pub async fn close(&self) {
		self.interrupt();
		if let Some(handler_join) = self.handler_join.lock().await.take() {
			if let Err(e) = handler_join.await {
				error!("Failed to shutdown: {e:?}");
			}
		}
	}

	pub fn interrupt(&self) {
		if !self.sender.is_closed() {
			self.sender.close();
		}
	}

	pub async fn send_message(&self, message_content: RoomMessageEventContent) {
		self.send(AdminRoomEvent::SendMessage(message_content))
			.await;
	}

	pub async fn process_message(&self, room_message: String, event_id: Arc<EventId>) {
		self.send(AdminRoomEvent::ProcessMessage(room_message, event_id))
			.await;
	}

	async fn receive(&self, event: AdminRoomEvent, room: &OwnedRoomId, user: &UserId) -> Result<(), Error> {
		if let Some(handle) = self.handle.lock().await.as_ref() {
			handle(event, room.clone(), user.into()).await
		} else {
			Err(Error::Err("Admin module is not loaded.".into()))
		}
	}

	async fn send(&self, message: AdminRoomEvent) {
		debug_assert!(!self.sender.is_full(), "channel full");
		debug_assert!(!self.sender.is_closed(), "channel closed");
		self.sender.send(message).expect("message sent");
	}

	/// Gets the room ID of the admin room
	///
	/// Errors are propagated from the database, and will have None if there is
	/// no admin room
	pub async fn get_admin_room() -> Result<Option<OwnedRoomId>> {
		let admin_room_alias: Box<RoomAliasId> = format!("#admins:{}", services().globals.server_name())
			.try_into()
			.expect("#admins:server_name is a valid alias name");

		services()
			.rooms
			.alias
			.resolve_local_alias(&admin_room_alias)
	}

	/// Create the admin room.
	///
	/// Users in this room are considered admins by conduit, and the room can be
	/// used to issue admin commands by talking to the server user inside it.
	pub async fn create_admin_room(&self) -> Result<()> {
		let room_id = RoomId::new(services().globals.server_name());

		services().rooms.short.get_or_create_shortroomid(&room_id)?;

		let mutex_state = Arc::clone(
			services()
				.globals
				.roomid_mutex_state
				.write()
				.await
				.entry(room_id.clone())
				.or_default(),
		);
		let state_lock = mutex_state.lock().await;

		// Create a user for the server
		let server_user = UserId::parse_with_server_name("conduit", services().globals.server_name())
			.expect("@conduit:server_name is valid");

		services().users.create(&server_user, None)?;

		let room_version = services().globals.default_room_version();
		let mut content = match room_version {
			RoomVersionId::V1
			| RoomVersionId::V2
			| RoomVersionId::V3
			| RoomVersionId::V4
			| RoomVersionId::V5
			| RoomVersionId::V6
			| RoomVersionId::V7
			| RoomVersionId::V8
			| RoomVersionId::V9
			| RoomVersionId::V10 => RoomCreateEventContent::new_v1(server_user.clone()),
			RoomVersionId::V11 => RoomCreateEventContent::new_v11(),
			_ => {
				warn!("Unexpected or unsupported room version {}", room_version);
				return Err(Error::BadRequest(
					ErrorKind::BadJson,
					"Unexpected or unsupported room version found",
				));
			},
		};

		content.federate = true;
		content.predecessor = None;
		content.room_version = room_version;

		// 1. The room create event
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomCreate,
					content: to_raw_value(&content).expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 2. Make conduit bot join
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomMember,
					content: to_raw_value(&RoomMemberEventContent {
						membership: MembershipState::Join,
						displayname: None,
						avatar_url: None,
						is_direct: None,
						third_party_invite: None,
						blurhash: None,
						reason: None,
						join_authorized_via_users_server: None,
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(server_user.to_string()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 3. Power levels
		let mut users = BTreeMap::new();
		users.insert(server_user.clone(), 100.into());

		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomPowerLevels,
					content: to_raw_value(&RoomPowerLevelsEventContent {
						users,
						..Default::default()
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 4.1 Join Rules
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomJoinRules,
					content: to_raw_value(&RoomJoinRulesEventContent::new(JoinRule::Invite))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 4.2 History Visibility
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomHistoryVisibility,
					content: to_raw_value(&RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 4.3 Guest Access
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomGuestAccess,
					content: to_raw_value(&RoomGuestAccessEventContent::new(GuestAccess::Forbidden))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 5. Events implied by name and topic
		let room_name = format!("{} Admin Room", services().globals.server_name());
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomName,
					content: to_raw_value(&RoomNameEventContent::new(room_name))
						.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomTopic,
					content: to_raw_value(&RoomTopicEventContent {
						topic: format!("Manage {}", services().globals.server_name()),
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// 6. Room alias
		let alias: OwnedRoomAliasId = format!("#admins:{}", services().globals.server_name())
			.try_into()
			.expect("#admins:server_name is a valid alias name");

		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomCanonicalAlias,
					content: to_raw_value(&RoomCanonicalAliasEventContent {
						alias: Some(alias.clone()),
						alt_aliases: Vec::new(),
					})
					.expect("event is valid, we just created it"),
					unsigned: None,
					state_key: Some(String::new()),
					redacts: None,
				},
				&server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		services().rooms.alias.set_alias(&alias, &room_id)?;

		Ok(())
	}

	/// Invite the user to the conduit admin room.
	///
	/// In conduit, this is equivalent to granting admin privileges.
	pub async fn make_user_admin(&self, user_id: &UserId, displayname: String) -> Result<()> {
		if let Some(room_id) = Self::get_admin_room().await? {
			let mutex_state = Arc::clone(
				services()
					.globals
					.roomid_mutex_state
					.write()
					.await
					.entry(room_id.clone())
					.or_default(),
			);
			let state_lock = mutex_state.lock().await;

			// Use the server user to grant the new admin's power level
			let server_user = UserId::parse_with_server_name("conduit", services().globals.server_name())
				.expect("@conduit:server_name is valid");

			// Invite and join the real user
			services()
				.rooms
				.timeline
				.build_and_append_pdu(
					PduBuilder {
						event_type: TimelineEventType::RoomMember,
						content: to_raw_value(&RoomMemberEventContent {
							membership: MembershipState::Invite,
							displayname: None,
							avatar_url: None,
							is_direct: None,
							third_party_invite: None,
							blurhash: None,
							reason: None,
							join_authorized_via_users_server: None,
						})
						.expect("event is valid, we just created it"),
						unsigned: None,
						state_key: Some(user_id.to_string()),
						redacts: None,
					},
					&server_user,
					&room_id,
					&state_lock,
				)
				.await?;
			services()
				.rooms
				.timeline
				.build_and_append_pdu(
					PduBuilder {
						event_type: TimelineEventType::RoomMember,
						content: to_raw_value(&RoomMemberEventContent {
							membership: MembershipState::Join,
							displayname: Some(displayname),
							avatar_url: None,
							is_direct: None,
							third_party_invite: None,
							blurhash: None,
							reason: None,
							join_authorized_via_users_server: None,
						})
						.expect("event is valid, we just created it"),
						unsigned: None,
						state_key: Some(user_id.to_string()),
						redacts: None,
					},
					user_id,
					&room_id,
					&state_lock,
				)
				.await?;

			// Set power level
			let mut users = BTreeMap::new();
			users.insert(server_user.clone(), 100.into());
			users.insert(user_id.to_owned(), 100.into());

			services()
				.rooms
				.timeline
				.build_and_append_pdu(
					PduBuilder {
						event_type: TimelineEventType::RoomPowerLevels,
						content: to_raw_value(&RoomPowerLevelsEventContent {
							users,
							..Default::default()
						})
						.expect("event is valid, we just created it"),
						unsigned: None,
						state_key: Some(String::new()),
						redacts: None,
					},
					&server_user,
					&room_id,
					&state_lock,
				)
				.await?;

			// Send welcome message
			services().rooms.timeline.build_and_append_pdu(
  			PduBuilder {
                event_type: TimelineEventType::RoomMessage,
                content: to_raw_value(&RoomMessageEventContent::text_html(
                        format!("## Thank you for trying out conduwuit!\n\nconduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.\n\nHelpful links:\n> Git and Documentation: https://github.com/girlbossceo/conduwuit\n> Report issues: https://github.com/girlbossceo/conduwuit/issues\n\nFor a list of available commands, send the following message in this room: `@conduit:{}: --help`\n\nHere are some rooms you can join (by typing the command):\n\nconduwuit room (Ask questions and get notified on updates):\n`/join #conduwuit:puppygock.gay`", services().globals.server_name()),
                        format!("<h2>Thank you for trying out conduwuit!</h2>\n<p>conduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.</p>\n<p>Helpful links:</p>\n<blockquote>\n<p>Git and Documentation: https://github.com/girlbossceo/conduwuit<br>Report issues: https://github.com/girlbossceo/conduwuit/issues</p>\n</blockquote>\n<p>For a list of available commands, send the following message in this room: <code>@conduit:{}: --help</code></p>\n<p>Here are some rooms you can join (by typing the command):</p>\n<p>conduwuit room (Ask questions and get notified on updates):<br><code>/join #conduwuit:puppygock.gay</code></p>\n", services().globals.server_name()),
                ))
                .expect("event is valid, we just created it"),
                unsigned: None,
                state_key: None,
                redacts: None,
            },
            &server_user,
            &room_id,
            &state_lock,
        ).await?;

			Ok(())
		} else {
			Ok(())
		}
	}
}
