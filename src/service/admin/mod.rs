pub mod console;
mod create;
mod grant;

use std::{future::Future, pin::Pin, sync::Arc};

use conduit::{Error, Result};
pub use create::create_admin_room;
pub use grant::make_user_admin;
use ruma::{
	events::{room::message::RoomMessageEventContent, TimelineEventType},
	EventId, OwnedRoomId, RoomId, UserId,
};
use serde_json::value::to_raw_value;
use tokio::{
	sync::{Mutex, MutexGuard},
	task::JoinHandle,
};
use tracing::error;

use crate::{pdu::PduBuilder, services};

pub type HandlerResult = Pin<Box<dyn Future<Output = Result<AdminEvent, Error>> + Send>>;
pub type Handler = fn(AdminEvent) -> HandlerResult;

pub struct Service {
	sender: loole::Sender<AdminEvent>,
	receiver: Mutex<loole::Receiver<AdminEvent>>,
	handler_join: Mutex<Option<JoinHandle<()>>>,
	pub handle: Mutex<Option<Handler>>,
	#[cfg(feature = "console")]
	pub console: Arc<console::Console>,
}

#[derive(Debug)]
pub enum AdminEvent {
	Command(String, Option<Arc<EventId>>),
	Reply(Option<RoomMessageEventContent>),
	Notice(RoomMessageEventContent),
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
			#[cfg(feature = "console")]
			console: console::Console::new(),
		})
	}

	pub fn interrupt(&self) {
		#[cfg(feature = "console")]
		self.console.interrupt();

		if !self.sender.is_closed() {
			self.sender.close();
		}
	}

	pub async fn close(&self) {
		self.interrupt();

		#[cfg(feature = "console")]
		self.console.close().await;

		if let Some(handler_join) = self.handler_join.lock().await.take() {
			if let Err(e) = handler_join.await {
				error!("Failed to shutdown: {e:?}");
			}
		}
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
		let mut signals = services().server.signal.subscribe();
		loop {
			debug_assert!(!receiver.is_closed(), "channel closed");
			tokio::select! {
				event = receiver.recv_async() => match event {
					Ok(event) => self.receive(event).await,
					Err(_) => return Ok(()),
				},
				sig = signals.recv() => match sig {
					Ok(sig) => self.handle_signal(sig).await,
					Err(_) => continue,
				},
			}
		}
	}

	pub async fn send_text(&self, body: &str) {
		self.send_message(RoomMessageEventContent::text_plain(body))
			.await;
	}

	pub async fn send_message(&self, message_content: RoomMessageEventContent) {
		self.send(AdminEvent::Notice(message_content)).await;
	}

	pub async fn command(&self, command: String, event_id: Option<Arc<EventId>>) {
		self.send(AdminEvent::Command(command, event_id)).await;
	}

	pub async fn command_in_place(
		&self, command: String, event_id: Option<Arc<EventId>>,
	) -> Result<Option<RoomMessageEventContent>> {
		match self.handle(AdminEvent::Command(command, event_id)).await? {
			AdminEvent::Reply(content) => Ok(content),
			_ => Ok(None),
		}
	}

	async fn send(&self, message: AdminEvent) {
		debug_assert!(!self.sender.is_full(), "channel full");
		debug_assert!(!self.sender.is_closed(), "channel closed");
		self.sender.send(message).expect("message sent");
	}

	async fn receive(&self, event: AdminEvent) {
		if let Ok(AdminEvent::Reply(content)) = self.handle(event).await {
			handle_response(content).await;
		}
	}

	async fn handle(&self, event: AdminEvent) -> Result<AdminEvent, Error> {
		if let Some(handle) = self.handle.lock().await.as_ref() {
			handle(event).await
		} else {
			Err(Error::Err("Admin module is not loaded.".into()))
		}
	}

	async fn handle_signal(&self, #[allow(unused_variables)] sig: &'static str) {
		#[cfg(feature = "console")]
		if sig == "SIGINT" && services().server.running() {
			self.console.start().await;
		}
	}

	/// Checks whether a given user is an admin of this server
	pub async fn user_is_admin(&self, user_id: &UserId) -> Result<bool> {
		let Ok(Some(admin_room)) = Self::get_admin_room() else {
			return Ok(false);
		};

		services().rooms.state_cache.is_joined(user_id, &admin_room)
	}

	/// Gets the room ID of the admin room
	///
	/// Errors are propagated from the database, and will have None if there is
	/// no admin room
	pub fn get_admin_room() -> Result<Option<OwnedRoomId>> {
		services()
			.rooms
			.alias
			.resolve_local_alias(&services().globals.admin_alias)
	}
}

async fn handle_response(content: Option<RoomMessageEventContent>) {
	if let Some(content) = content {
		if let Err(e) = respond_to_room(content).await {
			error!("{e}");
		}
	}
}

async fn respond_to_room(output_content: RoomMessageEventContent) -> Result<()> {
	let Ok(Some(admin_room)) = Service::get_admin_room() else {
		return Ok(());
	};

	let mutex_state = Arc::clone(
		services()
			.globals
			.roomid_mutex_state
			.write()
			.await
			.entry(admin_room.clone())
			.or_default(),
	);
	let state_lock = mutex_state.lock().await;

	let response_pdu = PduBuilder {
		event_type: TimelineEventType::RoomMessage,
		content: to_raw_value(&output_content).expect("event is valid, we just created it"),
		unsigned: None,
		state_key: None,
		redacts: None,
	};

	let server_user = &services().globals.server_user;
	if let Err(e) = services()
		.rooms
		.timeline
		.build_and_append_pdu(response_pdu, server_user, &admin_room, &state_lock)
		.await
	{
		handle_response_error(&e, &admin_room, server_user, &state_lock).await?;
	}

	Ok(())
}

async fn handle_response_error(
	e: &Error, admin_room: &RoomId, server_user: &UserId, state_lock: &MutexGuard<'_, ()>,
) -> Result<()> {
	error!("Failed to build and append admin room response PDU: \"{e}\"");
	let error_room_message = RoomMessageEventContent::text_plain(format!(
		"Failed to build and append admin room PDU: \"{e}\"\n\nThe original admin command may have finished \
		 successfully, but we could not return the output."
	));

	let response_pdu = PduBuilder {
		event_type: TimelineEventType::RoomMessage,
		content: to_raw_value(&error_room_message).expect("event is valid, we just created it"),
		unsigned: None,
		state_key: None,
		redacts: None,
	};

	services()
		.rooms
		.timeline
		.build_and_append_pdu(response_pdu, server_user, admin_room, state_lock)
		.await?;

	Ok(())
}
