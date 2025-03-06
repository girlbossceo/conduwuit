mod data;

use std::{
	collections::HashMap,
	fmt::Write,
	sync::{Arc, RwLock},
	time::Instant,
};

use conduwuit::{error, utils::bytes::pretty, Result, Server};
use data::Data;
use regex::RegexSet;
use ruma::{OwnedEventId, OwnedRoomAliasId, OwnedServerName, OwnedUserId, ServerName, UserId};

use crate::service;

pub struct Service {
	pub db: Data,
	server: Arc<Server>,

	pub bad_event_ratelimiter: Arc<RwLock<HashMap<OwnedEventId, RateLimitState>>>,
	pub server_user: OwnedUserId,
	pub admin_alias: OwnedRoomAliasId,
	pub turn_secret: String,
	pub registration_token: Option<String>,
}

type RateLimitState = (Instant, u32); // Time if last failed try, number of failed tries

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let db = Data::new(&args);
		let config = &args.server.config;

		let turn_secret =
			config
				.turn_secret_file
				.as_ref()
				.map_or(config.turn_secret.clone(), |path| {
					std::fs::read_to_string(path).unwrap_or_else(|e| {
						error!("Failed to read the TURN secret file: {e}");

						config.turn_secret.clone()
					})
				});

		let registration_token = config.registration_token_file.as_ref().map_or(
			config.registration_token.clone(),
			|path| {
				let Ok(token) = std::fs::read_to_string(path).inspect_err(|e| {
					error!("Failed to read the registration token file: {e}");
				}) else {
					return config.registration_token.clone();
				};

				Some(token)
			},
		);

		Ok(Arc::new(Self {
			db,
			server: args.server.clone(),
			bad_event_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
			admin_alias: OwnedRoomAliasId::try_from(format!("#admins:{}", &args.server.name))
				.expect("#admins:server_name is valid alias name"),
			server_user: UserId::parse_with_server_name(
				String::from("conduwuit"),
				&args.server.name,
			)
			.expect("@conduit:server_name is valid"),
			turn_secret,
			registration_token,
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result {
		let (ber_count, ber_bytes) = self.bad_event_ratelimiter.read()?.iter().fold(
			(0_usize, 0_usize),
			|(mut count, mut bytes), (event_id, _)| {
				bytes = bytes.saturating_add(event_id.capacity());
				bytes = bytes.saturating_add(size_of::<RateLimitState>());
				count = count.saturating_add(1);
				(count, bytes)
			},
		);

		writeln!(out, "bad_event_ratelimiter: {ber_count} ({})", pretty(ber_bytes))?;

		Ok(())
	}

	fn clear_cache(&self) {
		self.bad_event_ratelimiter
			.write()
			.expect("locked for writing")
			.clear();
	}

	fn name(&self) -> &str { service::make_name(std::module_path!()) }
}

impl Service {
	#[inline]
	pub fn next_count(&self) -> Result<u64> { self.db.next_count() }

	#[inline]
	pub fn current_count(&self) -> Result<u64> { Ok(self.db.current_count()) }

	#[inline]
	pub fn server_name(&self) -> &ServerName { self.server.name.as_ref() }

	pub fn allow_registration(&self) -> bool { self.server.config.allow_registration }

	pub fn allow_guest_registration(&self) -> bool { self.server.config.allow_guest_registration }

	pub fn allow_guests_auto_join_rooms(&self) -> bool {
		self.server.config.allow_guests_auto_join_rooms
	}

	pub fn log_guest_registrations(&self) -> bool { self.server.config.log_guest_registrations }

	pub fn allow_encryption(&self) -> bool { self.server.config.allow_encryption }

	pub fn allow_federation(&self) -> bool { self.server.config.allow_federation }

	pub fn allow_public_room_directory_over_federation(&self) -> bool {
		self.server
			.config
			.allow_public_room_directory_over_federation
	}

	pub fn allow_device_name_federation(&self) -> bool {
		self.server.config.allow_device_name_federation
	}

	pub fn allow_room_creation(&self) -> bool { self.server.config.allow_room_creation }

	pub fn new_user_displayname_suffix(&self) -> &String {
		&self.server.config.new_user_displayname_suffix
	}

	pub fn allow_check_for_updates(&self) -> bool { self.server.config.allow_check_for_updates }

	pub fn trusted_servers(&self) -> &[OwnedServerName] { &self.server.config.trusted_servers }

	pub fn turn_password(&self) -> &String { &self.server.config.turn_password }

	pub fn turn_ttl(&self) -> u64 { self.server.config.turn_ttl }

	pub fn turn_uris(&self) -> &[String] { &self.server.config.turn_uris }

	pub fn turn_username(&self) -> &String { &self.server.config.turn_username }

	pub fn notification_push_path(&self) -> &String { &self.server.config.notification_push_path }

	pub fn emergency_password(&self) -> &Option<String> { &self.server.config.emergency_password }

	pub fn url_preview_domain_contains_allowlist(&self) -> &Vec<String> {
		&self.server.config.url_preview_domain_contains_allowlist
	}

	pub fn url_preview_domain_explicit_allowlist(&self) -> &Vec<String> {
		&self.server.config.url_preview_domain_explicit_allowlist
	}

	pub fn url_preview_domain_explicit_denylist(&self) -> &Vec<String> {
		&self.server.config.url_preview_domain_explicit_denylist
	}

	pub fn url_preview_url_contains_allowlist(&self) -> &Vec<String> {
		&self.server.config.url_preview_url_contains_allowlist
	}

	pub fn url_preview_max_spider_size(&self) -> usize {
		self.server.config.url_preview_max_spider_size
	}

	pub fn url_preview_check_root_domain(&self) -> bool {
		self.server.config.url_preview_check_root_domain
	}

	pub fn forbidden_alias_names(&self) -> &RegexSet { &self.server.config.forbidden_alias_names }

	pub fn forbidden_usernames(&self) -> &RegexSet { &self.server.config.forbidden_usernames }

	pub fn allow_local_presence(&self) -> bool { self.server.config.allow_local_presence }

	pub fn allow_incoming_presence(&self) -> bool { self.server.config.allow_incoming_presence }

	pub fn allow_outgoing_presence(&self) -> bool { self.server.config.allow_outgoing_presence }

	pub fn allow_incoming_read_receipts(&self) -> bool {
		self.server.config.allow_incoming_read_receipts
	}

	pub fn allow_outgoing_read_receipts(&self) -> bool {
		self.server.config.allow_outgoing_read_receipts
	}

	pub fn block_non_admin_invites(&self) -> bool { self.server.config.block_non_admin_invites }

	/// checks if `user_id` is local to us via server_name comparison
	#[inline]
	pub fn user_is_local(&self, user_id: &UserId) -> bool {
		self.server_is_ours(user_id.server_name())
	}

	#[inline]
	pub fn server_is_ours(&self, server_name: &ServerName) -> bool {
		server_name == self.server_name()
	}

	#[inline]
	pub fn is_read_only(&self) -> bool { self.db.db.is_read_only() }
}
