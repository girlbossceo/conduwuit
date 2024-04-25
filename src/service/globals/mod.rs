use std::{
	collections::{BTreeMap, HashMap},
	fs,
	future::Future,
	path::PathBuf,
	sync::{
		atomic::{self, AtomicBool},
		Arc,
	},
	time::Instant,
};

use argon2::Argon2;
use base64::{engine::general_purpose, Engine as _};
pub(crate) use data::Data;
use hickory_resolver::TokioAsyncResolver;
use ipaddress::IPAddress;
use regex::RegexSet;
use ruma::{
	api::{
		client::{discovery::discover_support::ContactRole, sync::sync_events},
		federation::discovery::{ServerSigningKeys, VerifyKey},
	},
	serde::Base64,
	DeviceId, OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedServerName, OwnedServerSigningKeyId, OwnedUserId,
	RoomVersionId, ServerName, UserId,
};
use tokio::sync::{broadcast, watch::Receiver, Mutex, RwLock};
use tracing::{error, info, trace};
use url::Url;

use crate::{services, Config, LogLevelReloadHandles, Result};

mod client;
mod data;
mod resolver;

type RateLimitState = (Instant, u32); // Time if last failed try, number of failed tries
type SyncHandle = (
	Option<String>,                                      // since
	Receiver<Option<Result<sync_events::v3::Response>>>, // rx
);

pub(crate) struct Service<'a> {
	pub(crate) db: &'static dyn Data,

	pub(crate) tracing_reload_handle: LogLevelReloadHandles,
	pub(crate) config: Config,
	pub(crate) cidr_range_denylist: Vec<IPAddress>,
	keypair: Arc<ruma::signatures::Ed25519KeyPair>,
	jwt_decoding_key: Option<jsonwebtoken::DecodingKey>,
	pub(crate) resolver: Arc<resolver::Resolver>,
	pub(crate) client: client::Client,
	pub(crate) stable_room_versions: Vec<RoomVersionId>,
	pub(crate) unstable_room_versions: Vec<RoomVersionId>,
	pub(crate) bad_event_ratelimiter: Arc<RwLock<HashMap<OwnedEventId, RateLimitState>>>,
	pub(crate) bad_signature_ratelimiter: Arc<RwLock<HashMap<Vec<String>, RateLimitState>>>,
	pub(crate) bad_query_ratelimiter: Arc<RwLock<HashMap<OwnedServerName, RateLimitState>>>,
	pub(crate) sync_receivers: RwLock<HashMap<(OwnedUserId, OwnedDeviceId), SyncHandle>>,
	pub(crate) roomid_mutex_insert: RwLock<HashMap<OwnedRoomId, Arc<Mutex<()>>>>,
	pub(crate) roomid_mutex_state: RwLock<HashMap<OwnedRoomId, Arc<Mutex<()>>>>,
	pub(crate) roomid_mutex_federation: RwLock<HashMap<OwnedRoomId, Arc<Mutex<()>>>>, // this lock will be held longer
	pub(crate) roomid_federationhandletime: RwLock<HashMap<OwnedRoomId, (OwnedEventId, Instant)>>,
	pub(crate) stateres_mutex: Arc<Mutex<()>>,
	pub(crate) rotate: RotationHandler,

	pub(crate) shutdown: AtomicBool,
	pub(crate) argon: Argon2<'a>,
}

/// Handles "rotation" of long-polling requests. "Rotation" in this context is
/// similar to "rotation" of log files and the like.
///
/// This is utilized to have sync workers return early and release read locks on
/// the database.
pub(crate) struct RotationHandler(broadcast::Sender<()>, ());

impl RotationHandler {
	fn new() -> Self {
		let (s, _r) = broadcast::channel(1);
		Self(s, ())
	}

	pub(crate) fn watch(&self) -> impl Future<Output = ()> {
		let mut r = self.0.subscribe();

		async move {
			_ = r.recv().await;
		}
	}

	fn fire(&self) { _ = self.0.send(()); }
}

impl Default for RotationHandler {
	fn default() -> Self { Self::new() }
}

impl Service<'_> {
	pub(crate) fn load(
		db: &'static dyn Data, config: &Config, tracing_reload_handle: LogLevelReloadHandles,
	) -> Result<Self> {
		let keypair = db.load_keypair();

		let keypair = match keypair {
			Ok(k) => k,
			Err(e) => {
				error!("Keypair invalid. Deleting...");
				db.remove_keypair()?;
				return Err(e);
			},
		};

		let jwt_decoding_key = config
			.jwt_secret
			.as_ref()
			.map(|secret| jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()));

		let resolver = Arc::new(resolver::Resolver::new(config));

		// Supported and stable room versions
		let stable_room_versions = vec![
			RoomVersionId::V6,
			RoomVersionId::V7,
			RoomVersionId::V8,
			RoomVersionId::V9,
			RoomVersionId::V10,
			RoomVersionId::V11,
		];
		// Experimental, partially supported room versions
		let unstable_room_versions = vec![RoomVersionId::V2, RoomVersionId::V3, RoomVersionId::V4, RoomVersionId::V5];

		// 19456 Kib blocks, iterations = 2, parallelism = 1 for more info https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#argon2id
		let argon = Argon2::new(
			argon2::Algorithm::Argon2id,
			argon2::Version::default(),
			argon2::Params::new(19456, 2, 1, None).expect("valid parameters"),
		);

		let mut cidr_range_denylist = Vec::new();
		for cidr in config.ip_range_denylist.clone() {
			let cidr = IPAddress::parse(cidr).expect("valid cidr range");
			trace!("Denied CIDR range: {:?}", cidr);
			cidr_range_denylist.push(cidr);
		}

		let mut s = Self {
			tracing_reload_handle,
			db,
			config: config.clone(),
			cidr_range_denylist,
			keypair: Arc::new(keypair),
			resolver: resolver.clone(),
			client: client::Client::new(config, &resolver),
			jwt_decoding_key,
			stable_room_versions,
			unstable_room_versions,
			bad_event_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
			bad_signature_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
			bad_query_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
			roomid_mutex_state: RwLock::new(HashMap::new()),
			roomid_mutex_insert: RwLock::new(HashMap::new()),
			roomid_mutex_federation: RwLock::new(HashMap::new()),
			roomid_federationhandletime: RwLock::new(HashMap::new()),
			stateres_mutex: Arc::new(Mutex::new(())),
			sync_receivers: RwLock::new(HashMap::new()),
			rotate: RotationHandler::new(),
			shutdown: AtomicBool::new(false),
			argon,
		};

		fs::create_dir_all(s.get_media_folder())?;

		if !s
			.supported_room_versions()
			.contains(&s.config.default_room_version)
		{
			error!(config=?s.config.default_room_version, fallback=?crate::config::default_default_room_version(), "Room version in config isn't supported, falling back to default version");
			s.config.default_room_version = crate::config::default_default_room_version();
		};

		Ok(s)
	}

	/// Returns this server's keypair.
	pub(crate) fn keypair(&self) -> &ruma::signatures::Ed25519KeyPair { &self.keypair }

	#[tracing::instrument(skip(self))]
	pub(crate) fn next_count(&self) -> Result<u64> { self.db.next_count() }

	#[tracing::instrument(skip(self))]
	pub(crate) fn current_count(&self) -> Result<u64> { self.db.current_count() }

	#[tracing::instrument(skip(self))]
	pub(crate) fn last_check_for_updates_id(&self) -> Result<u64> { self.db.last_check_for_updates_id() }

	#[tracing::instrument(skip(self))]
	pub(crate) fn update_check_for_updates_id(&self, id: u64) -> Result<()> { self.db.update_check_for_updates_id(id) }

	pub(crate) async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result<()> {
		self.db.watch(user_id, device_id).await
	}

	pub(crate) fn cleanup(&self) -> Result<()> { self.db.cleanup() }

	/// TODO: use this?
	#[allow(dead_code)]
	pub(crate) fn flush(&self) -> Result<()> { self.db.flush() }

	pub(crate) fn server_name(&self) -> &ServerName { self.config.server_name.as_ref() }

	pub(crate) fn max_request_size(&self) -> u32 { self.config.max_request_size }

	pub(crate) fn max_fetch_prev_events(&self) -> u16 { self.config.max_fetch_prev_events }

	pub(crate) fn allow_registration(&self) -> bool { self.config.allow_registration }

	pub(crate) fn allow_guest_registration(&self) -> bool { self.config.allow_guest_registration }

	pub(crate) fn allow_guests_auto_join_rooms(&self) -> bool { self.config.allow_guests_auto_join_rooms }

	pub(crate) fn log_guest_registrations(&self) -> bool { self.config.log_guest_registrations }

	pub(crate) fn allow_encryption(&self) -> bool { self.config.allow_encryption }

	pub(crate) fn allow_federation(&self) -> bool { self.config.allow_federation }

	pub(crate) fn allow_public_room_directory_over_federation(&self) -> bool {
		self.config.allow_public_room_directory_over_federation
	}

	pub(crate) fn allow_device_name_federation(&self) -> bool { self.config.allow_device_name_federation }

	pub(crate) fn allow_room_creation(&self) -> bool { self.config.allow_room_creation }

	pub(crate) fn allow_unstable_room_versions(&self) -> bool { self.config.allow_unstable_room_versions }

	pub(crate) fn default_room_version(&self) -> RoomVersionId { self.config.default_room_version.clone() }

	pub(crate) fn new_user_displayname_suffix(&self) -> &String { &self.config.new_user_displayname_suffix }

	pub(crate) fn allow_check_for_updates(&self) -> bool { self.config.allow_check_for_updates }

	pub(crate) fn trusted_servers(&self) -> &[OwnedServerName] { &self.config.trusted_servers }

	pub(crate) fn query_trusted_key_servers_first(&self) -> bool { self.config.query_trusted_key_servers_first }

	pub(crate) fn dns_resolver(&self) -> &TokioAsyncResolver { &self.resolver.resolver }

	pub(crate) fn actual_destinations(&self) -> &Arc<RwLock<resolver::WellKnownMap>> { &self.resolver.destinations }

	pub(crate) fn jwt_decoding_key(&self) -> Option<&jsonwebtoken::DecodingKey> { self.jwt_decoding_key.as_ref() }

	pub(crate) fn turn_password(&self) -> &String { &self.config.turn_password }

	pub(crate) fn turn_ttl(&self) -> u64 { self.config.turn_ttl }

	pub(crate) fn turn_uris(&self) -> &[String] { &self.config.turn_uris }

	pub(crate) fn turn_username(&self) -> &String { &self.config.turn_username }

	pub(crate) fn turn_secret(&self) -> &String { &self.config.turn_secret }

	pub(crate) fn allow_profile_lookup_federation_requests(&self) -> bool {
		self.config.allow_profile_lookup_federation_requests
	}

	pub(crate) fn notification_push_path(&self) -> &String { &self.config.notification_push_path }

	pub(crate) fn emergency_password(&self) -> &Option<String> { &self.config.emergency_password }

	pub(crate) fn url_preview_domain_contains_allowlist(&self) -> &Vec<String> {
		&self.config.url_preview_domain_contains_allowlist
	}

	pub(crate) fn url_preview_domain_explicit_allowlist(&self) -> &Vec<String> {
		&self.config.url_preview_domain_explicit_allowlist
	}

	pub(crate) fn url_preview_domain_explicit_denylist(&self) -> &Vec<String> {
		&self.config.url_preview_domain_explicit_denylist
	}

	pub(crate) fn url_preview_url_contains_allowlist(&self) -> &Vec<String> {
		&self.config.url_preview_url_contains_allowlist
	}

	pub(crate) fn url_preview_max_spider_size(&self) -> usize { self.config.url_preview_max_spider_size }

	pub(crate) fn url_preview_check_root_domain(&self) -> bool { self.config.url_preview_check_root_domain }

	pub(crate) fn forbidden_alias_names(&self) -> &RegexSet { &self.config.forbidden_alias_names }

	pub(crate) fn forbidden_usernames(&self) -> &RegexSet { &self.config.forbidden_usernames }

	pub(crate) fn allow_local_presence(&self) -> bool { self.config.allow_local_presence }

	pub(crate) fn allow_incoming_presence(&self) -> bool { self.config.allow_incoming_presence }

	pub(crate) fn allow_outgoing_presence(&self) -> bool { self.config.allow_outgoing_presence }

	pub(crate) fn allow_incoming_read_receipts(&self) -> bool { self.config.allow_incoming_read_receipts }

	pub(crate) fn allow_outgoing_read_receipts(&self) -> bool { self.config.allow_outgoing_read_receipts }

	pub(crate) fn prevent_media_downloads_from(&self) -> &[OwnedServerName] {
		&self.config.prevent_media_downloads_from
	}

	pub(crate) fn forbidden_remote_room_directory_server_names(&self) -> &[OwnedServerName] {
		&self.config.forbidden_remote_room_directory_server_names
	}

	pub(crate) fn well_known_support_page(&self) -> &Option<Url> { &self.config.well_known.support_page }

	pub(crate) fn well_known_support_role(&self) -> &Option<ContactRole> { &self.config.well_known.support_role }

	pub(crate) fn well_known_support_email(&self) -> &Option<String> { &self.config.well_known.support_email }

	pub(crate) fn well_known_support_mxid(&self) -> &Option<OwnedUserId> { &self.config.well_known.support_mxid }

	pub(crate) fn block_non_admin_invites(&self) -> bool { self.config.block_non_admin_invites }

	pub(crate) fn supported_room_versions(&self) -> Vec<RoomVersionId> {
		let mut room_versions: Vec<RoomVersionId> = vec![];
		room_versions.extend(self.stable_room_versions.clone());
		if self.allow_unstable_room_versions() {
			room_versions.extend(self.unstable_room_versions.clone());
		};
		room_versions
	}

	/// TODO: the key valid until timestamp (`valid_until_ts`) is only honored
	/// in room version > 4
	///
	/// Remove the outdated keys and insert the new ones.
	///
	/// This doesn't actually check that the keys provided are newer than the
	/// old set.
	pub(crate) fn add_signing_key(
		&self, origin: &ServerName, new_keys: ServerSigningKeys,
	) -> Result<BTreeMap<OwnedServerSigningKeyId, VerifyKey>> {
		self.db.add_signing_key(origin, new_keys)
	}

	/// This returns an empty `Ok(BTreeMap<..>)` when there are no keys found
	/// for the server.
	pub(crate) fn signing_keys_for(&self, origin: &ServerName) -> Result<BTreeMap<OwnedServerSigningKeyId, VerifyKey>> {
		let mut keys = self.db.signing_keys_for(origin)?;
		if origin == self.server_name() {
			keys.insert(
				format!("ed25519:{}", services().globals.keypair().version())
					.try_into()
					.expect("found invalid server signing keys in DB"),
				VerifyKey {
					key: Base64::new(self.keypair.public_key().to_vec()),
				},
			);
		}

		Ok(keys)
	}

	pub(crate) fn database_version(&self) -> Result<u64> { self.db.database_version() }

	pub(crate) fn bump_database_version(&self, new_version: u64) -> Result<()> {
		self.db.bump_database_version(new_version)
	}

	pub(crate) fn get_media_folder(&self) -> PathBuf {
		let mut r = PathBuf::new();
		r.push(self.config.database_path.clone());
		r.push("media");
		r
	}

	/// new SHA256 file name media function, requires "sha256_media" feature
	/// flag enabled and database migrated uses SHA256 hash of the base64 key as
	/// the file name
	#[cfg(feature = "sha256_media")]
	pub(crate) fn get_media_file_new(&self, key: &[u8]) -> PathBuf {
		let mut r = PathBuf::new();
		r.push(self.config.database_path.clone());
		r.push("media");
		// Using the hash of the base64 key as the filename
		// This is to prevent the total length of the path from exceeding the maximum
		// length in most filesystems
		r.push(general_purpose::URL_SAFE_NO_PAD.encode(<sha2::Sha256 as sha2::Digest>::digest(key)));
		r
	}

	/// old base64 file name media function
	/// This is the old version of `get_media_file` that uses the full base64
	/// key as the filename.
	pub(crate) fn get_media_file(&self, key: &[u8]) -> PathBuf {
		let mut r = PathBuf::new();
		r.push(self.config.database_path.clone());
		r.push("media");
		r.push(general_purpose::URL_SAFE_NO_PAD.encode(key));
		r
	}

	pub(crate) fn well_known_client(&self) -> &Option<Url> { &self.config.well_known.client }

	pub(crate) fn well_known_server(&self) -> &Option<OwnedServerName> { &self.config.well_known.server }

	pub(crate) fn unix_socket_path(&self) -> &Option<PathBuf> { &self.config.unix_socket_path }

	pub(crate) fn valid_cidr_range(&self, ip: &IPAddress) -> bool {
		for cidr in &self.cidr_range_denylist {
			if cidr.includes(ip) {
				return false;
			}
		}

		true
	}

	pub(crate) fn shutdown(&self) {
		self.shutdown.store(true, atomic::Ordering::Relaxed);
		// On shutdown

		if self.unix_socket_path().is_some() {
			match &self.unix_socket_path() {
				Some(path) => {
					fs::remove_file(path).unwrap();
				},
				None => error!(
					"Unable to remove socket file at {:?} during shutdown.",
					&self.unix_socket_path()
				),
			};
		};

		info!(target: "shutdown-sync", "Received shutdown notification, notifying sync helpers...");
		services().globals.rotate.fire();
	}
}
