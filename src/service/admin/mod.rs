pub mod console;
mod create;
mod grant;

use std::{
	future::Future,
	pin::Pin,
	sync::{Arc, RwLock as StdRwLock},
};

use async_trait::async_trait;
use conduit::{debug, error, error::default_log, Error, Result};
pub use create::create_admin_room;
pub use grant::make_user_admin;
use loole::{Receiver, Sender};
use ruma::{
	events::{
		room::message::{Relation, RoomMessageEventContent},
		TimelineEventType,
	},
	OwnedEventId, OwnedRoomId, RoomId, UserId,
};
use serde_json::value::to_raw_value;
use tokio::sync::{Mutex, RwLock};

use crate::{pdu::PduBuilder, rooms::state::RoomMutexGuard, services, user_is_local, PduEvent};

pub struct Service {
	sender: Sender<Command>,
	receiver: Mutex<Receiver<Command>>,
	pub handle: RwLock<Option<Handler>>,
	pub complete: StdRwLock<Option<Completer>>,
	#[cfg(feature = "console")]
	pub console: Arc<console::Console>,
}

#[derive(Debug)]
pub struct Command {
	pub command: String,
	pub reply_id: Option<OwnedEventId>,
}

pub type Completer = fn(&str) -> String;
pub type Handler = fn(Command) -> HandlerResult;
pub type HandlerResult = Pin<Box<dyn Future<Output = CommandResult> + Send>>;
pub type CommandResult = Result<CommandOutput, Error>;
pub type CommandOutput = Option<RoomMessageEventContent>;

const COMMAND_QUEUE_LIMIT: usize = 512;

#[async_trait]
impl crate::Service for Service {
	fn build(_args: crate::Args<'_>) -> Result<Arc<Self>> {
		let (sender, receiver) = loole::bounded(COMMAND_QUEUE_LIMIT);
		Ok(Arc::new(Self {
			sender,
			receiver: Mutex::new(receiver),
			handle: RwLock::new(None),
			complete: StdRwLock::new(None),
			#[cfg(feature = "console")]
			console: console::Console::new(),
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		let receiver = self.receiver.lock().await;
		let mut signals = services().server.signal.subscribe();
		loop {
			tokio::select! {
				command = receiver.recv_async() => match command {
					Ok(command) => self.handle_command(command).await,
					Err(_) => break,
				},
				sig = signals.recv() => match sig {
					Ok(sig) => self.handle_signal(sig).await,
					Err(_) => continue,
				},
			}
		}

		//TODO: not unwind safe
		#[cfg(feature = "console")]
		self.console.close().await;

		Ok(())
	}

	fn interrupt(&self) {
		#[cfg(feature = "console")]
		self.console.interrupt();

		if !self.sender.is_closed() {
			self.sender.close();
		}
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub async fn send_text(&self, body: &str) {
		self.send_message(RoomMessageEventContent::text_markdown(body))
			.await;
	}

	pub async fn send_message(&self, message_content: RoomMessageEventContent) {
		if let Ok(Some(room_id)) = Self::get_admin_room() {
			let user_id = &services().globals.server_user;
			respond_to_room(message_content, &room_id, user_id).await;
		}
	}

	pub async fn command(&self, command: String, reply_id: Option<OwnedEventId>) {
		self.send(Command {
			command,
			reply_id,
		})
		.await;
	}

	pub async fn command_in_place(
		&self, command: String, reply_id: Option<OwnedEventId>,
	) -> Result<Option<RoomMessageEventContent>> {
		self.process_command(Command {
			command,
			reply_id,
		})
		.await
	}

	pub fn complete_command(&self, command: &str) -> Option<String> {
		self.complete
			.read()
			.expect("locked for reading")
			.map(|complete| complete(command))
	}

	async fn send(&self, message: Command) {
		debug_assert!(!self.sender.is_closed(), "channel closed");
		self.sender.send_async(message).await.expect("message sent");
	}

	async fn handle_signal(&self, #[allow(unused_variables)] sig: &'static str) {
		#[cfg(feature = "console")]
		self.console.handle_signal(sig).await;
	}

	async fn handle_command(&self, command: Command) {
		match self.process_command(command).await {
			Ok(Some(output)) => handle_response(output).await,
			Ok(None) => debug!("Command successful with no response"),
			Err(e) => error!("Command processing error: {e}"),
		}
	}

	async fn process_command(&self, command: Command) -> CommandResult {
		if let Some(handle) = self.handle.read().await.as_ref() {
			handle(command).await
		} else {
			Err(Error::Err("Admin module is not loaded.".into()))
		}
	}

	/// Checks whether a given user is an admin of this server
	pub async fn user_is_admin(&self, user_id: &UserId) -> Result<bool> {
		if let Ok(Some(admin_room)) = Self::get_admin_room() {
			services().rooms.state_cache.is_joined(user_id, &admin_room)
		} else {
			Ok(false)
		}
	}

	/// Gets the room ID of the admin room
	///
	/// Errors are propagated from the database, and will have None if there is
	/// no admin room
	pub fn get_admin_room() -> Result<Option<OwnedRoomId>> {
		if let Some(room_id) = services()
			.rooms
			.alias
			.resolve_local_alias(&services().globals.admin_alias)?
		{
			if services()
				.rooms
				.state_cache
				.is_joined(&services().globals.server_user, &room_id)?
			{
				return Ok(Some(room_id));
			}
		}

		Ok(None)
	}
}

async fn handle_response(content: RoomMessageEventContent) {
	let Some(Relation::Reply {
		in_reply_to,
	}) = content.relates_to.as_ref()
	else {
		return;
	};

	let Ok(Some(pdu)) = services().rooms.timeline.get_pdu(&in_reply_to.event_id) else {
		return;
	};

	let response_sender = if is_admin_room(&pdu.room_id) {
		&services().globals.server_user
	} else {
		&pdu.sender
	};

	respond_to_room(content, &pdu.room_id, response_sender).await;
}

async fn respond_to_room(content: RoomMessageEventContent, room_id: &RoomId, user_id: &UserId) {
	assert!(
		services()
			.admin
			.user_is_admin(user_id)
			.await
			.expect("checked user is admin"),
		"sender is not admin"
	);

	let state_lock = services().rooms.state.mutex.lock(room_id).await;
	let response_pdu = PduBuilder {
		event_type: TimelineEventType::RoomMessage,
		content: to_raw_value(&content).expect("event is valid, we just created it"),
		unsigned: None,
		state_key: None,
		redacts: None,
	};

	if let Err(e) = services()
		.rooms
		.timeline
		.build_and_append_pdu(response_pdu, user_id, room_id, &state_lock)
		.await
	{
		handle_response_error(e, room_id, user_id, &state_lock)
			.await
			.unwrap_or_else(default_log);
	}
}

async fn handle_response_error(
	e: Error, room_id: &RoomId, user_id: &UserId, state_lock: &RoomMutexGuard,
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
		.build_and_append_pdu(response_pdu, user_id, room_id, state_lock)
		.await?;

	Ok(())
}

pub async fn is_admin_command(pdu: &PduEvent, body: &str) -> bool {
	// Server-side command-escape with public echo
	let is_escape = body.starts_with('\\');
	let is_public_escape = is_escape && body.trim_start_matches('\\').starts_with("!admin");

	// Admin command with public echo (in admin room)
	let server_user = &services().globals.server_user;
	let is_public_prefix = body.starts_with("!admin") || body.starts_with(server_user.as_str());

	// Expected backward branch
	if !is_public_escape && !is_public_prefix {
		return false;
	}

	// only allow public escaped commands by local admins
	if is_public_escape && !user_is_local(&pdu.sender) {
		return false;
	}

	// Check if server-side command-escape is disabled by configuration
	if is_public_escape && !services().globals.config.admin_escape_commands {
		return false;
	}

	// Prevent unescaped !admin from being used outside of the admin room
	if is_public_prefix && !is_admin_room(&pdu.room_id) {
		return false;
	}

	// Only senders who are admin can proceed
	if !services()
		.admin
		.user_is_admin(&pdu.sender)
		.await
		.unwrap_or(false)
	{
		return false;
	}

	// This will evaluate to false if the emergency password is set up so that
	// the administrator can execute commands as conduit
	let emergency_password_set = services().globals.emergency_password().is_some();
	let from_server = pdu.sender == *server_user && !emergency_password_set;
	if from_server && is_admin_room(&pdu.room_id) {
		return false;
	}

	// Authentic admin command
	true
}

#[must_use]
pub fn is_admin_room(room_id: &RoomId) -> bool {
	if let Ok(Some(admin_room_id)) = Service::get_admin_room() {
		admin_room_id == room_id
	} else {
		false
	}
}
