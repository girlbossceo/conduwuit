pub mod console;
mod create;
mod grant;
mod startup;

use std::{
	future::Future,
	pin::Pin,
	sync::{Arc, RwLock as StdRwLock, Weak},
};

use async_trait::async_trait;
use conduit::{debug, err, error, error::default_log, pdu::PduBuilder, Error, PduEvent, Result, Server};
pub use create::create_admin_room;
use futures::{FutureExt, TryFutureExt};
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

use crate::{account_data, globals, rooms, rooms::state::RoomMutexGuard, Dep};

pub struct Service {
	services: Services,
	sender: Sender<CommandInput>,
	receiver: Mutex<Receiver<CommandInput>>,
	pub handle: RwLock<Option<Processor>>,
	pub complete: StdRwLock<Option<Completer>>,
	#[cfg(feature = "console")]
	pub console: Arc<console::Console>,
}

struct Services {
	server: Arc<Server>,
	globals: Dep<globals::Service>,
	alias: Dep<rooms::alias::Service>,
	timeline: Dep<rooms::timeline::Service>,
	state: Dep<rooms::state::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
	account_data: Dep<account_data::Service>,
	services: StdRwLock<Option<Weak<crate::Services>>>,
}

/// Inputs to a command are a multi-line string and optional reply_id.
#[derive(Debug)]
pub struct CommandInput {
	pub command: String,
	pub reply_id: Option<OwnedEventId>,
}

/// Prototype of the tab-completer. The input is buffered text when tab
/// asserted; the output will fully replace the input buffer.
pub type Completer = fn(&str) -> String;

/// Prototype of the command processor. This is a callback supplied by the
/// reloadable admin module.
pub type Processor = fn(Arc<crate::Services>, CommandInput) -> ProcessorFuture;

/// Return type of the processor
pub type ProcessorFuture = Pin<Box<dyn Future<Output = ProcessorResult> + Send>>;

/// Result wrapping of a command's handling. Both variants are complete message
/// events which have digested any prior errors. The wrapping preserves whether
/// the command failed without interpreting the text. Ok(None) outputs are
/// dropped to produce no response.
pub type ProcessorResult = Result<Option<CommandOutput>, CommandOutput>;

/// Alias for the output structure.
pub type CommandOutput = RoomMessageEventContent;

/// Maximum number of commands which can be queued for dispatch.
const COMMAND_QUEUE_LIMIT: usize = 512;

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let (sender, receiver) = loole::bounded(COMMAND_QUEUE_LIMIT);
		Ok(Arc::new(Self {
			services: Services {
				server: args.server.clone(),
				globals: args.depend::<globals::Service>("globals"),
				alias: args.depend::<rooms::alias::Service>("rooms::alias"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
				state: args.depend::<rooms::state::Service>("rooms::state"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
				account_data: args.depend::<account_data::Service>("account_data"),
				services: None.into(),
			},
			sender,
			receiver: Mutex::new(receiver),
			handle: RwLock::new(None),
			complete: StdRwLock::new(None),
			#[cfg(feature = "console")]
			console: console::Console::new(&args),
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		let receiver = self.receiver.lock().await;
		let mut signals = self.services.server.signal.subscribe();

		self.startup_execute().await?;
		self.console_auto_start().await;

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

		self.console_auto_stop().await; //TODO: not unwind safe

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
	/// Sends markdown message (not an m.notice for notification reasons) to the
	/// admin room as the admin user.
	pub async fn send_text(&self, body: &str) {
		self.send_message(RoomMessageEventContent::text_markdown(body))
			.await
			.ok();
	}

	/// Sends a message to the admin room as the admin user (see send_text() for
	/// convenience).
	pub async fn send_message(&self, message_content: RoomMessageEventContent) -> Result<()> {
		let user_id = &self.services.globals.server_user;
		let room_id = self.get_admin_room().await?;
		self.respond_to_room(message_content, &room_id, user_id)
			.boxed()
			.await
	}

	/// Posts a command to the command processor queue and returns. Processing
	/// will take place on the service worker's task asynchronously. Errors if
	/// the queue is full.
	pub fn command(&self, command: String, reply_id: Option<OwnedEventId>) -> Result<()> {
		self.sender
			.send(CommandInput {
				command,
				reply_id,
			})
			.map_err(|e| err!("Failed to enqueue admin command: {e:?}"))
	}

	/// Dispatches a comamnd to the processor on the current task and waits for
	/// completion.
	pub async fn command_in_place(&self, command: String, reply_id: Option<OwnedEventId>) -> ProcessorResult {
		self.process_command(CommandInput {
			command,
			reply_id,
		})
		.await
	}

	/// Invokes the tab-completer to complete the command. When unavailable,
	/// None is returned.
	pub fn complete_command(&self, command: &str) -> Option<String> {
		self.complete
			.read()
			.expect("locked for reading")
			.map(|complete| complete(command))
	}

	async fn handle_signal(&self, #[allow(unused_variables)] sig: &'static str) {
		#[cfg(feature = "console")]
		self.console.handle_signal(sig).await;
	}

	async fn handle_command(&self, command: CommandInput) {
		match self.process_command(command).await {
			Ok(None) => debug!("Command successful with no response"),
			Ok(Some(output)) | Err(output) => self
				.handle_response(output)
				.boxed()
				.await
				.unwrap_or_else(default_log),
		}
	}

	async fn process_command(&self, command: CommandInput) -> ProcessorResult {
		let handle = &self
			.handle
			.read()
			.await
			.expect("Admin module is not loaded");

		let services = self
			.services
			.services
			.read()
			.expect("locked")
			.as_ref()
			.and_then(Weak::upgrade)
			.expect("Services self-reference not initialized.");

		handle(services, command).await
	}

	/// Checks whether a given user is an admin of this server
	pub async fn user_is_admin(&self, user_id: &UserId) -> bool {
		let Ok(admin_room) = self.get_admin_room().await else {
			return false;
		};

		self.services
			.state_cache
			.is_joined(user_id, &admin_room)
			.await
	}

	/// Gets the room ID of the admin room
	///
	/// Errors are propagated from the database, and will have None if there is
	/// no admin room
	pub async fn get_admin_room(&self) -> Result<OwnedRoomId> {
		let room_id = self
			.services
			.alias
			.resolve_local_alias(&self.services.globals.admin_alias)
			.await?;

		self.services
			.state_cache
			.is_joined(&self.services.globals.server_user, &room_id)
			.await
			.then_some(room_id)
			.ok_or_else(|| err!(Request(NotFound("Admin user not joined to admin room"))))
	}

	async fn handle_response(&self, content: RoomMessageEventContent) -> Result<()> {
		let Some(Relation::Reply {
			in_reply_to,
		}) = content.relates_to.as_ref()
		else {
			return Ok(());
		};

		let Ok(pdu) = self.services.timeline.get_pdu(&in_reply_to.event_id).await else {
			error!(
				event_id = ?in_reply_to.event_id,
				"Missing admin command in_reply_to event"
			);
			return Ok(());
		};

		let response_sender = if self.is_admin_room(&pdu.room_id).await {
			&self.services.globals.server_user
		} else {
			&pdu.sender
		};

		self.respond_to_room(content, &pdu.room_id, response_sender)
			.await
	}

	async fn respond_to_room(
		&self, content: RoomMessageEventContent, room_id: &RoomId, user_id: &UserId,
	) -> Result<()> {
		assert!(self.user_is_admin(user_id).await, "sender is not admin");

		let response_pdu = PduBuilder {
			event_type: TimelineEventType::RoomMessage,
			content: to_raw_value(&content).expect("event is valid, we just created it"),
			unsigned: None,
			state_key: None,
			redacts: None,
			timestamp: None,
		};

		let state_lock = self.services.state.mutex.lock(room_id).await;
		if let Err(e) = self
			.services
			.timeline
			.build_and_append_pdu(response_pdu, user_id, room_id, &state_lock)
			.await
		{
			self.handle_response_error(e, room_id, user_id, &state_lock)
				.await
				.unwrap_or_else(default_log);
		}

		Ok(())
	}

	async fn handle_response_error(
		&self, e: Error, room_id: &RoomId, user_id: &UserId, state_lock: &RoomMutexGuard,
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
			timestamp: None,
		};

		self.services
			.timeline
			.build_and_append_pdu(response_pdu, user_id, room_id, state_lock)
			.await?;

		Ok(())
	}

	pub async fn is_admin_command(&self, pdu: &PduEvent, body: &str) -> bool {
		// Server-side command-escape with public echo
		let is_escape = body.starts_with('\\');
		let is_public_escape = is_escape && body.trim_start_matches('\\').starts_with("!admin");

		// Admin command with public echo (in admin room)
		let server_user = &self.services.globals.server_user;
		let is_public_prefix = body.starts_with("!admin") || body.starts_with(server_user.as_str());

		// Expected backward branch
		if !is_public_escape && !is_public_prefix {
			return false;
		}

		// only allow public escaped commands by local admins
		if is_public_escape && !self.services.globals.user_is_local(&pdu.sender) {
			return false;
		}

		// Check if server-side command-escape is disabled by configuration
		if is_public_escape && !self.services.globals.config.admin_escape_commands {
			return false;
		}

		// Prevent unescaped !admin from being used outside of the admin room
		if is_public_prefix && !self.is_admin_room(&pdu.room_id).await {
			return false;
		}

		// Only senders who are admin can proceed
		if !self.user_is_admin(&pdu.sender).await {
			return false;
		}

		// This will evaluate to false if the emergency password is set up so that
		// the administrator can execute commands as conduit
		let emergency_password_set = self.services.globals.emergency_password().is_some();
		let from_server = pdu.sender == *server_user && !emergency_password_set;
		if from_server && self.is_admin_room(&pdu.room_id).await {
			return false;
		}

		// Authentic admin command
		true
	}

	#[must_use]
	pub async fn is_admin_room(&self, room_id_: &RoomId) -> bool {
		self.get_admin_room()
			.map_ok(|room_id| room_id == room_id_)
			.await
			.unwrap_or(false)
	}

	/// Sets the self-reference to crate::Services which will provide context to
	/// the admin commands.
	pub(super) fn set_services(&self, services: &Option<Arc<crate::Services>>) {
		let receiver = &mut *self.services.services.write().expect("locked for writing");
		let weak = services.as_ref().map(Arc::downgrade);
		*receiver = weak;
	}
}
