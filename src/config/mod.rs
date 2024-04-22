use std::{
	collections::BTreeMap,
	fmt::{self, Write as _},
	net::{IpAddr, Ipv4Addr, SocketAddr},
	path::PathBuf,
};

use either::{
	Either,
	Either::{Left, Right},
};
use figment::{
	providers::{Env, Format, Toml},
	Figment,
};
use itertools::Itertools;
use regex::RegexSet;
use ruma::{
	api::client::discovery::discover_support::ContactRole, OwnedRoomId, OwnedServerName, OwnedUserId, RoomVersionId,
};
use serde::{de::IgnoredAny, Deserialize};
use tracing::{debug, error, warn};
use url::Url;

use self::{check::check, proxy::ProxyConfig};
use crate::utils::error::Error;

pub(crate) mod check;
mod proxy;

#[derive(Deserialize, Clone, Debug)]
#[serde(transparent)]
pub struct ListeningPort {
	#[serde(with = "either::serde_untagged")]
	pub ports: Either<u16, Vec<u16>>,
}

/// all the config options for conduwuit
#[derive(Clone, Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct Config {
	/// [`IpAddr`] conduwuit will listen on (can be IPv4 or IPv6)
	#[serde(default = "default_address")]
	pub address: IpAddr,
	/// default TCP port(s) conduwuit will listen on
	#[serde(default = "default_port")]
	pub port: ListeningPort,
	pub tls: Option<TlsConfig>,
	pub unix_socket_path: Option<PathBuf>,
	#[serde(default = "default_unix_socket_perms")]
	pub unix_socket_perms: u32,
	pub server_name: OwnedServerName,
	#[serde(default = "default_database_backend")]
	pub database_backend: String,
	pub database_path: PathBuf,
	pub database_backup_path: Option<PathBuf>,
	#[serde(default = "default_database_backups_to_keep")]
	pub database_backups_to_keep: i16,
	#[serde(default = "default_db_cache_capacity_mb")]
	pub db_cache_capacity_mb: f64,
	#[serde(default = "default_new_user_displayname_suffix")]
	pub new_user_displayname_suffix: String,
	#[serde(default)]
	pub allow_check_for_updates: bool,

	#[serde(default = "default_pdu_cache_capacity")]
	pub pdu_cache_capacity: u32,
	#[serde(default = "default_conduit_cache_capacity_modifier")]
	pub conduit_cache_capacity_modifier: f64,
	#[serde(default = "default_auth_chain_cache_capacity")]
	pub auth_chain_cache_capacity: u32,
	#[serde(default = "default_shorteventid_cache_capacity")]
	pub shorteventid_cache_capacity: u32,
	#[serde(default = "default_eventidshort_cache_capacity")]
	pub eventidshort_cache_capacity: u32,
	#[serde(default = "default_shortstatekey_cache_capacity")]
	pub shortstatekey_cache_capacity: u32,
	#[serde(default = "default_statekeyshort_cache_capacity")]
	pub statekeyshort_cache_capacity: u32,
	#[serde(default = "default_server_visibility_cache_capacity")]
	pub server_visibility_cache_capacity: u32,
	#[serde(default = "default_user_visibility_cache_capacity")]
	pub user_visibility_cache_capacity: u32,
	#[serde(default = "default_stateinfo_cache_capacity")]
	pub stateinfo_cache_capacity: u32,
	#[serde(default = "default_roomid_spacehierarchy_cache_capacity")]
	pub roomid_spacehierarchy_cache_capacity: u32,

	#[serde(default = "default_cleanup_second_interval")]
	pub cleanup_second_interval: u32,
	#[serde(default = "default_dns_cache_entries")]
	pub dns_cache_entries: u32,
	#[serde(default = "default_dns_min_ttl")]
	pub dns_min_ttl: u64,
	#[serde(default = "default_dns_min_ttl_nxdomain")]
	pub dns_min_ttl_nxdomain: u64,
	#[serde(default = "default_dns_attempts")]
	pub dns_attempts: u16,
	#[serde(default = "default_dns_timeout")]
	pub dns_timeout: u64,
	#[serde(default = "true_fn")]
	pub dns_tcp_fallback: bool,
	#[serde(default = "true_fn")]
	pub query_all_nameservers: bool,
	#[serde(default = "default_max_request_size")]
	pub max_request_size: u32,
	#[serde(default = "default_max_concurrent_requests")]
	pub max_concurrent_requests: u16,
	#[serde(default = "default_max_fetch_prev_events")]
	pub max_fetch_prev_events: u16,
	#[serde(default = "default_request_conn_timeout")]
	pub request_conn_timeout: u64,
	#[serde(default = "default_request_timeout")]
	pub request_timeout: u64,
	#[serde(default = "default_request_total_timeout")]
	pub request_total_timeout: u64,
	#[serde(default = "default_request_idle_timeout")]
	pub request_idle_timeout: u64,
	#[serde(default = "default_request_idle_per_host")]
	pub request_idle_per_host: u16,
	#[serde(default = "default_well_known_conn_timeout")]
	pub well_known_conn_timeout: u64,
	#[serde(default = "default_well_known_timeout")]
	pub well_known_timeout: u64,
	#[serde(default = "default_federation_timeout")]
	pub federation_timeout: u64,
	#[serde(default = "default_federation_idle_timeout")]
	pub federation_idle_timeout: u64,
	#[serde(default = "default_federation_idle_per_host")]
	pub federation_idle_per_host: u16,
	#[serde(default = "default_sender_timeout")]
	pub sender_timeout: u64,
	#[serde(default = "default_sender_idle_timeout")]
	pub sender_idle_timeout: u64,
	#[serde(default = "default_appservice_timeout")]
	pub appservice_timeout: u64,
	#[serde(default = "default_appservice_idle_timeout")]
	pub appservice_idle_timeout: u64,
	#[serde(default = "default_pusher_idle_timeout")]
	pub pusher_idle_timeout: u64,
	#[serde(default)]
	pub allow_registration: bool,
	#[serde(default)]
	pub yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse: bool,
	pub registration_token: Option<String>,
	#[serde(default = "true_fn")]
	pub allow_encryption: bool,
	#[serde(default = "true_fn")]
	pub allow_federation: bool,
	#[serde(default)]
	pub allow_public_room_directory_over_federation: bool,
	#[serde(default)]
	pub allow_public_room_directory_without_auth: bool,
	#[serde(default)]
	pub lockdown_public_room_directory: bool,
	#[serde(default)]
	pub allow_device_name_federation: bool,
	#[serde(default = "true_fn")]
	pub allow_profile_lookup_federation_requests: bool,
	#[serde(default = "true_fn")]
	pub allow_room_creation: bool,
	#[serde(default = "true_fn")]
	pub allow_unstable_room_versions: bool,
	#[serde(default = "default_default_room_version")]
	pub default_room_version: RoomVersionId,
	#[serde(default)]
	pub well_known: WellKnownConfig,
	#[serde(default)]
	pub allow_jaeger: bool,
	#[serde(default)]
	pub tracing_flame: bool,
	#[serde(default)]
	pub proxy: ProxyConfig,
	pub jwt_secret: Option<String>,
	#[serde(default = "default_trusted_servers")]
	pub trusted_servers: Vec<OwnedServerName>,
	#[serde(default = "true_fn")]
	pub query_trusted_key_servers_first: bool,
	#[serde(default = "default_log")]
	pub log: String,
	#[serde(default)]
	pub turn_username: String,
	#[serde(default)]
	pub turn_password: String,
	#[serde(default = "Vec::new")]
	pub turn_uris: Vec<String>,
	#[serde(default)]
	pub turn_secret: String,
	#[serde(default = "default_turn_ttl")]
	pub turn_ttl: u64,

	#[serde(default = "Vec::new")]
	pub auto_join_rooms: Vec<OwnedRoomId>,

	#[serde(default = "default_rocksdb_log_level")]
	pub rocksdb_log_level: String,
	#[serde(default)]
	pub rocksdb_log_stderr: bool,
	#[serde(default = "default_rocksdb_max_log_file_size")]
	pub rocksdb_max_log_file_size: usize,
	#[serde(default = "default_rocksdb_log_time_to_roll")]
	pub rocksdb_log_time_to_roll: usize,
	#[serde(default)]
	pub rocksdb_optimize_for_spinning_disks: bool,
	#[serde(default = "default_rocksdb_parallelism_threads")]
	pub rocksdb_parallelism_threads: usize,
	#[serde(default = "default_rocksdb_max_log_files")]
	pub rocksdb_max_log_files: usize,
	#[serde(default = "default_rocksdb_compression_algo")]
	pub rocksdb_compression_algo: String,
	#[serde(default = "default_rocksdb_compression_level")]
	pub rocksdb_compression_level: i32,
	#[serde(default = "default_rocksdb_bottommost_compression_level")]
	pub rocksdb_bottommost_compression_level: i32,
	#[serde(default)]
	pub rocksdb_bottommost_compression: bool,
	#[serde(default = "default_rocksdb_recovery_mode")]
	pub rocksdb_recovery_mode: u8,
	#[serde(default)]
	pub rocksdb_repair: bool,
	#[serde(default)]
	pub rocksdb_read_only: bool,
	#[serde(default)]
	pub rocksdb_periodic_cleanup: bool,

	pub emergency_password: Option<String>,

	#[serde(default = "default_notification_push_path")]
	pub notification_push_path: String,

	#[serde(default = "true_fn")]
	pub allow_local_presence: bool,
	#[serde(default = "true_fn")]
	pub allow_incoming_presence: bool,
	#[serde(default = "true_fn")]
	pub allow_outgoing_presence: bool,
	#[serde(default = "default_presence_idle_timeout_s")]
	pub presence_idle_timeout_s: u64,
	#[serde(default = "default_presence_offline_timeout_s")]
	pub presence_offline_timeout_s: u64,
	#[serde(default = "true_fn")]
	pub presence_timeout_remote_users: bool,

	#[serde(default = "true_fn")]
	pub allow_incoming_read_receipts: bool,
	#[serde(default = "true_fn")]
	pub allow_outgoing_read_receipts: bool,

	#[serde(default = "true_fn")]
	pub allow_outgoing_typing: bool,
	#[serde(default = "true_fn")]
	pub allow_incoming_typing: bool,
	#[serde(default = "default_typing_federation_timeout_s")]
	pub typing_federation_timeout_s: u64,
	#[serde(default = "default_typing_client_timeout_min_s")]
	pub typing_client_timeout_min_s: u64,
	#[serde(default = "default_typing_client_timeout_max_s")]
	pub typing_client_timeout_max_s: u64,

	#[serde(default)]
	pub zstd_compression: bool,
	#[serde(default)]
	pub gzip_compression: bool,
	#[serde(default)]
	pub brotli_compression: bool,

	#[serde(default)]
	pub allow_guest_registration: bool,
	#[serde(default)]
	pub log_guest_registrations: bool,
	#[serde(default)]
	pub allow_guests_auto_join_rooms: bool,

	#[serde(default = "Vec::new")]
	pub prevent_media_downloads_from: Vec<OwnedServerName>,
	#[serde(default = "Vec::new")]
	pub forbidden_remote_server_names: Vec<OwnedServerName>,
	#[serde(default = "Vec::new")]
	pub forbidden_remote_room_directory_server_names: Vec<OwnedServerName>,

	#[serde(default = "default_ip_range_denylist")]
	pub ip_range_denylist: Vec<String>,

	#[serde(default = "Vec::new")]
	pub url_preview_domain_contains_allowlist: Vec<String>,
	#[serde(default = "Vec::new")]
	pub url_preview_domain_explicit_allowlist: Vec<String>,
	#[serde(default = "Vec::new")]
	pub url_preview_domain_explicit_denylist: Vec<String>,
	#[serde(default = "Vec::new")]
	pub url_preview_url_contains_allowlist: Vec<String>,
	#[serde(default = "default_url_preview_max_spider_size")]
	pub url_preview_max_spider_size: usize,
	#[serde(default)]
	pub url_preview_check_root_domain: bool,

	#[serde(default = "RegexSet::empty")]
	#[serde(with = "serde_regex")]
	pub forbidden_alias_names: RegexSet,

	#[serde(default = "RegexSet::empty")]
	#[serde(with = "serde_regex")]
	pub forbidden_usernames: RegexSet,

	#[serde(default = "true_fn")]
	pub startup_netburst: bool,
	#[serde(default = "default_startup_netburst_keep")]
	pub startup_netburst_keep: i64,

	#[serde(default)]
	pub block_non_admin_invites: bool,

	#[serde(default)]
	pub sentry: bool,
	#[serde(default)]
	pub sentry_send_server_name: bool,
	#[serde(default = "default_sentry_traces_sample_rate")]
	pub sentry_traces_sample_rate: f32,

	#[serde(flatten)]
	#[allow(clippy::zero_sized_map_values)] // this is a catchall, the map shouldn't be zero at runtime
	pub catchall: BTreeMap<String, IgnoredAny>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
	pub certs: String,
	pub key: String,
	#[serde(default)]
	/// Whether to listen and allow for HTTP and HTTPS connections (insecure!)
	/// Only works / does something if the `axum_dual_protocol` feature flag was
	/// built
	pub dual_protocol: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct WellKnownConfig {
	pub client: Option<Url>,
	pub server: Option<OwnedServerName>,
	pub support_page: Option<Url>,
	pub support_role: Option<ContactRole>,
	pub support_email: Option<String>,
	pub support_mxid: Option<OwnedUserId>,
}

const DEPRECATED_KEYS: &[&str] = &[
	"cache_capacity",
	"well_known_client",
	"well_known_server",
	"well_known_support_page",
	"well_known_support_role",
	"well_known_support_email",
	"well_known_support_mxid",
];

impl Config {
	/// Initialize config
	pub fn new(path: Option<PathBuf>) -> Result<Self, Error> {
		let raw_config = if let Some(config_file_env) = Env::var("CONDUIT_CONFIG") {
			Figment::new()
				.merge(Toml::file(config_file_env).nested())
				.merge(Env::prefixed("CONDUIT_").global())
		} else if let Some(config_file_arg) = path {
			Figment::new()
				.merge(Toml::file(config_file_arg).nested())
				.merge(Env::prefixed("CONDUIT_").global())
		} else {
			Figment::new().merge(Env::prefixed("CONDUIT_").global())
		};

		let config = match raw_config.extract::<Config>() {
			Err(e) => return Err(Error::BadConfig(format!("{e}"))),
			Ok(config) => config,
		};

		// don't start if we're listening on both UNIX sockets and TCP at same time
		if config.is_dual_listening(&raw_config) {
			return Err(Error::bad_config("dual listening on UNIX and TCP sockets not allowed."));
		};

		Ok(config)
	}

	/// Iterates over all the keys in the config file and warns if there is a
	/// deprecated key specified
	pub fn warn_deprecated(&self) {
		debug!("Checking for deprecated config keys");
		let mut was_deprecated = false;
		for key in self
			.catchall
			.keys()
			.filter(|key| DEPRECATED_KEYS.iter().any(|s| s == key))
		{
			warn!("Config parameter \"{}\" is deprecated, ignoring.", key);
			was_deprecated = true;
		}

		if was_deprecated {
			warn!(
				"Read conduwuit config documentation at https://conduwuit.puppyirl.gay/configuration.html and check \
				 your configuration if any new configuration parameters should be adjusted"
			);
		}
	}

	/// iterates over all the catchall keys (unknown config options) and warns
	/// if there are any.
	pub fn warn_unknown_key(&self) {
		debug!("Checking for unknown config keys");
		for key in self
			.catchall
			.keys()
			.filter(|key| "config".to_owned().ne(key.to_owned()) /* "config" is expected */)
		{
			warn!("Config parameter \"{}\" is unknown to conduwuit, ignoring.", key);
		}
	}

	/// Checks the presence of the `address` and `unix_socket_path` keys in the
	/// raw_config, exiting the process if both keys were detected.
	fn is_dual_listening(&self, raw_config: &Figment) -> bool {
		let check_address = raw_config.find_value("address");
		let check_unix_socket = raw_config.find_value("unix_socket_path");

		// are the check_address and check_unix_socket keys both Ok (specified) at the
		// same time?
		if check_address.is_ok() && check_unix_socket.is_ok() {
			error!("TOML keys \"address\" and \"unix_socket_path\" were both defined. Please specify only one option.");
			return true;
		}

		false
	}

	#[must_use]
	pub fn get_bind_addrs(&self) -> Vec<SocketAddr> {
		match &self.port.ports {
			Left(port) => {
				// Left is only 1 value, so make a vec with 1 value only
				let port_vec = [port];

				port_vec
					.iter()
					.copied()
					.map(|port| SocketAddr::from((self.address, *port)))
					.collect::<Vec<_>>()
			},
			Right(ports) => ports
				.iter()
				.copied()
				.map(|port| SocketAddr::from((self.address, port)))
				.collect::<Vec<_>>(),
		}
	}

	pub fn check(&self) -> Result<(), Error> { check(self) }
}

impl fmt::Display for Config {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Prepare a list of config values to show
		let lines = [
			("Server name", self.server_name.host()),
			("Database backend", &self.database_backend),
			("Database path", &self.database_path.to_string_lossy()),
			(
				"Database backup path",
				match &self.database_backup_path {
					Some(path) => path.to_str().unwrap(),
					None => "",
				},
			),
			("Database backups to keep", &self.database_backups_to_keep.to_string()),
			("Database cache capacity (MB)", &self.db_cache_capacity_mb.to_string()),
			("Cache capacity modifier", &self.conduit_cache_capacity_modifier.to_string()),
			("PDU cache capacity", &self.pdu_cache_capacity.to_string()),
			("Auth chain cache capacity", &self.auth_chain_cache_capacity.to_string()),
			("Short eventid cache capacity", &self.shorteventid_cache_capacity.to_string()),
			("Eventid short cache capacity", &self.eventidshort_cache_capacity.to_string()),
			("Short statekey cache capacity", &self.shortstatekey_cache_capacity.to_string()),
			("Statekey short cache capacity", &self.statekeyshort_cache_capacity.to_string()),
			(
				"Server visibility cache capacity",
				&self.server_visibility_cache_capacity.to_string(),
			),
			(
				"User visibility cache capacity",
				&self.user_visibility_cache_capacity.to_string(),
			),
			("Stateinfo cache capacity", &self.stateinfo_cache_capacity.to_string()),
			(
				"Roomid space hierarchy cache capacity",
				&self.roomid_spacehierarchy_cache_capacity.to_string(),
			),
			("Cleanup interval in seconds", &self.cleanup_second_interval.to_string()),
			("DNS cache entry limit", &self.dns_cache_entries.to_string()),
			("DNS minimum ttl", &self.dns_min_ttl.to_string()),
			("DNS minimum nxdomain ttl", &self.dns_min_ttl_nxdomain.to_string()),
			("DNS attempts", &self.dns_attempts.to_string()),
			("DNS timeout", &self.dns_timeout.to_string()),
			("DNS fallback to TCP", &self.dns_tcp_fallback.to_string()),
			("Query all nameservers", &self.query_all_nameservers.to_string()),
			("Maximum request size (bytes)", &self.max_request_size.to_string()),
			("Maximum concurrent requests", &self.max_concurrent_requests.to_string()),
			("Request connect timeout", &self.request_conn_timeout.to_string()),
			("Request timeout", &self.request_timeout.to_string()),
			("Request total timeout", &self.request_total_timeout.to_string()),
			("Idle connections per host", &self.request_idle_per_host.to_string()),
			("Request pool idle timeout", &self.request_idle_timeout.to_string()),
			("Well_known connect timeout", &self.well_known_conn_timeout.to_string()),
			("Well_known timeout", &self.well_known_timeout.to_string()),
			("Federation timeout", &self.federation_timeout.to_string()),
			("Federation pool idle per host", &self.federation_idle_per_host.to_string()),
			("Federation pool idle timeout", &self.federation_idle_timeout.to_string()),
			("Sender timeout", &self.sender_timeout.to_string()),
			("Sender pool idle timeout", &self.sender_idle_timeout.to_string()),
			("Appservice timeout", &self.appservice_timeout.to_string()),
			("Appservice pool idle timeout", &self.appservice_idle_timeout.to_string()),
			("Pusher pool idle timeout", &self.pusher_idle_timeout.to_string()),
			("Allow registration", &self.allow_registration.to_string()),
			(
				"Registration token",
				match self.registration_token {
					Some(_) => "set",
					None => "not set (open registration!)",
				},
			),
			(
				"Allow guest registration (inherently false if allow registration is false)",
				&self.allow_guest_registration.to_string(),
			),
			(
				"Log guest registrations in admin room",
				&self.log_guest_registrations.to_string(),
			),
			(
				"Allow guests to auto join rooms",
				&self.allow_guests_auto_join_rooms.to_string(),
			),
			("New user display name suffix", &self.new_user_displayname_suffix),
			("Allow encryption", &self.allow_encryption.to_string()),
			("Allow federation", &self.allow_federation.to_string()),
			(
				"Allow incoming federated presence requests (updates)",
				&self.allow_incoming_presence.to_string(),
			),
			(
				"Allow outgoing federated presence requests (updates)",
				&self.allow_outgoing_presence.to_string(),
			),
			(
				"Allow local presence requests (updates)",
				&self.allow_local_presence.to_string(),
			),
			(
				"Allow incoming remote read receipts",
				&self.allow_incoming_read_receipts.to_string(),
			),
			(
				"Allow outgoing remote read receipts",
				&self.allow_outgoing_read_receipts.to_string(),
			),
			(
				"Block non-admin room invites (local and remote, admins can still send and receive invites)",
				&self.block_non_admin_invites.to_string(),
			),
			("Allow outgoing federated typing", &self.allow_outgoing_typing.to_string()),
			("Allow incoming federated typing", &self.allow_incoming_typing.to_string()),
			(
				"Incoming federated typing timeout",
				&self.typing_federation_timeout_s.to_string(),
			),
			("Client typing timeout minimum", &self.typing_client_timeout_min_s.to_string()),
			("Client typing timeout maxmimum", &self.typing_client_timeout_max_s.to_string()),
			("Allow device name federation", &self.allow_device_name_federation.to_string()),
			(
				"Allow incoming profile lookup federation requests",
				&self.allow_profile_lookup_federation_requests.to_string(),
			),
			("Notification push path", &self.notification_push_path),
			("Allow room creation", &self.allow_room_creation.to_string()),
			(
				"Allow public room directory over federation",
				&self.allow_public_room_directory_over_federation.to_string(),
			),
			(
				"Allow public room directory without authentication",
				&self.allow_public_room_directory_without_auth.to_string(),
			),
			(
				"Lockdown public room directory (only allow admins to publish)",
				&self.lockdown_public_room_directory.to_string(),
			),
			(
				"JWT secret",
				match self.jwt_secret {
					Some(_) => "set",
					None => "not set",
				},
			),
			("Trusted key servers", {
				let mut lst = vec![];
				for server in &self.trusted_servers {
					lst.push(server.host());
				}
				&lst.join(", ")
			}),
			(
				"Query Trusted Key Servers First",
				&self.query_trusted_key_servers_first.to_string(),
			),
			(
				"TURN username",
				if self.turn_username.is_empty() {
					"not set"
				} else {
					&self.turn_username
				},
			),
			("TURN password", {
				if self.turn_password.is_empty() {
					"not set"
				} else {
					"set"
				}
			}),
			("TURN secret", {
				if self.turn_secret.is_empty() {
					"not set"
				} else {
					"set"
				}
			}),
			("Turn TTL", &self.turn_ttl.to_string()),
			("Turn URIs", {
				let mut lst = vec![];
				for item in self.turn_uris.iter().cloned().enumerate() {
					let (_, uri): (usize, String) = item;
					lst.push(uri);
				}
				&lst.join(", ")
			}),
			("Auto Join Rooms", {
				let mut lst = vec![];
				for room in &self.auto_join_rooms {
					lst.push(room);
				}
				&lst.into_iter().join(", ")
			}),
			#[cfg(feature = "zstd_compression")]
			("Zstd HTTP Compression", &self.zstd_compression.to_string()),
			#[cfg(feature = "gzip_compression")]
			("Gzip HTTP Compression", &self.gzip_compression.to_string()),
			#[cfg(feature = "brotli_compression")]
			("Brotli HTTP Compression", &self.brotli_compression.to_string()),
			#[cfg(feature = "rocksdb")]
			("RocksDB database LOG level", &self.rocksdb_log_level),
			#[cfg(feature = "rocksdb")]
			("RocksDB database LOG to stderr", &self.rocksdb_log_stderr.to_string()),
			#[cfg(feature = "rocksdb")]
			("RocksDB database LOG time-to-roll", &self.rocksdb_log_time_to_roll.to_string()),
			#[cfg(feature = "rocksdb")]
			("RocksDB Max LOG Files", &self.rocksdb_max_log_files.to_string()),
			#[cfg(feature = "rocksdb")]
			(
				"RocksDB database max LOG file size",
				&self.rocksdb_max_log_file_size.to_string(),
			),
			#[cfg(feature = "rocksdb")]
			(
				"RocksDB database optimize for spinning disks",
				&self.rocksdb_optimize_for_spinning_disks.to_string(),
			),
			#[cfg(feature = "rocksdb")]
			("RocksDB Parallelism Threads", &self.rocksdb_parallelism_threads.to_string()),
			#[cfg(feature = "rocksdb")]
			("RocksDB Compression Algorithm", &self.rocksdb_compression_algo),
			#[cfg(feature = "rocksdb")]
			("RocksDB Compression Level", &self.rocksdb_compression_level.to_string()),
			#[cfg(feature = "rocksdb")]
			(
				"RocksDB Bottommost Compression Level",
				&self.rocksdb_bottommost_compression_level.to_string(),
			),
			#[cfg(feature = "rocksdb")]
			(
				"RocksDB Bottommost Level Compression",
				&self.rocksdb_bottommost_compression.to_string(),
			),
			#[cfg(feature = "rocksdb")]
			("RocksDB Recovery Mode", &self.rocksdb_recovery_mode.to_string()),
			("RocksDB Repair Mode", &self.rocksdb_repair.to_string()),
			("RocksDB Read-only Mode", &self.rocksdb_read_only.to_string()),
			("RocksDB Periodic Cleanup", &self.rocksdb_periodic_cleanup.to_string()),
			("Prevent Media Downloads From", {
				let mut lst = vec![];
				for domain in &self.prevent_media_downloads_from {
					lst.push(domain.host());
				}
				&lst.join(", ")
			}),
			("Forbidden Remote Server Names (\"Global\" ACLs)", {
				let mut lst = vec![];
				for domain in &self.forbidden_remote_server_names {
					lst.push(domain.host());
				}
				&lst.join(", ")
			}),
			("Forbidden Remote Room Directory Server Names", {
				let mut lst = vec![];
				for domain in &self.forbidden_remote_room_directory_server_names {
					lst.push(domain.host());
				}
				&lst.join(", ")
			}),
			("Outbound Request IP Range Denylist", {
				let mut lst = vec![];
				for item in self.ip_range_denylist.iter().cloned().enumerate() {
					let (_, ip): (usize, String) = item;
					lst.push(ip);
				}
				&lst.join(", ")
			}),
			("Forbidden usernames", {
				&self.forbidden_usernames.patterns().iter().join(", ")
			}),
			("Forbidden room aliases", {
				&self.forbidden_alias_names.patterns().iter().join(", ")
			}),
			(
				"URL preview domain contains allowlist",
				&self.url_preview_domain_contains_allowlist.join(", "),
			),
			(
				"URL preview domain explicit allowlist",
				&self.url_preview_domain_explicit_allowlist.join(", "),
			),
			(
				"URL preview domain explicit denylist",
				&self.url_preview_domain_explicit_denylist.join(", "),
			),
			(
				"URL preview URL contains allowlist",
				&self.url_preview_url_contains_allowlist.join(", "),
			),
			("URL preview maximum spider size", &self.url_preview_max_spider_size.to_string()),
			("URL preview check root domain", &self.url_preview_check_root_domain.to_string()),
			(
				"Allow check for updates / announcements check",
				&self.allow_check_for_updates.to_string(),
			),
			("Enable netburst on startup", &self.startup_netburst.to_string()),
			#[cfg(feature = "sentry_telemetry")]
			("Sentry.io reporting and tracing", &self.sentry.to_string()),
			#[cfg(feature = "sentry_telemetry")]
			("Sentry.io send server_name in logs", &self.sentry_send_server_name.to_string()),
			#[cfg(feature = "sentry_telemetry")]
			("Sentry.io tracing sample rate", &self.sentry_traces_sample_rate.to_string()),
			(
				"Well-known server name",
				&if let Some(server) = &self.well_known.server {
					server.to_string()
				} else {
					String::new()
				},
			),
			(
				"Well-known support email",
				&if let Some(support_email) = &self.well_known.support_email {
					support_email.to_string()
				} else {
					String::new()
				},
			),
			(
				"Well-known support Matrix ID",
				&if let Some(support_mxid) = &self.well_known.support_mxid {
					support_mxid.to_string()
				} else {
					String::new()
				},
			),
			(
				"Well-known support role",
				&if let Some(support_role) = &self.well_known.support_role {
					support_role.to_string()
				} else {
					String::new()
				},
			),
			(
				"Well-known support page/URL",
				&if let Some(support_page) = &self.well_known.support_page {
					support_page.to_string()
				} else {
					String::new()
				},
			),
		];

		let mut msg: String = "Active config values:\n\n".to_owned();

		for line in lines.into_iter().enumerate() {
			let _ = writeln!(msg, "{}: {}", line.1 .0, line.1 .1);
		}

		write!(f, "{msg}")
	}
}

fn true_fn() -> bool { true }

fn default_address() -> IpAddr { Ipv4Addr::LOCALHOST.into() }

fn default_port() -> ListeningPort {
	ListeningPort {
		ports: Left(8008),
	}
}

fn default_unix_socket_perms() -> u32 { 660 }

fn default_database_backups_to_keep() -> i16 { 1 }

fn default_database_backend() -> String { "rocksdb".to_owned() }

fn default_db_cache_capacity_mb() -> f64 { 256.0 }

fn default_pdu_cache_capacity() -> u32 { 150_000 }

fn default_conduit_cache_capacity_modifier() -> f64 { 1.0 }

fn default_auth_chain_cache_capacity() -> u32 { 100_000 }

fn default_shorteventid_cache_capacity() -> u32 { 500_000 }

fn default_eventidshort_cache_capacity() -> u32 { 100_000 }

fn default_shortstatekey_cache_capacity() -> u32 { 100_000 }

fn default_statekeyshort_cache_capacity() -> u32 { 100_000 }

fn default_server_visibility_cache_capacity() -> u32 { 100 }

fn default_user_visibility_cache_capacity() -> u32 { 100 }

fn default_stateinfo_cache_capacity() -> u32 { 100 }

fn default_roomid_spacehierarchy_cache_capacity() -> u32 { 100 }

fn default_cleanup_second_interval() -> u32 {
	1800 // every 30 minutes
}

fn default_dns_cache_entries() -> u32 { 12288 }

fn default_dns_min_ttl() -> u64 { 60 * 180 }

fn default_dns_min_ttl_nxdomain() -> u64 { 60 * 60 * 24 }

fn default_dns_attempts() -> u16 { 10 }

fn default_dns_timeout() -> u64 { 10 }

fn default_max_request_size() -> u32 {
	20 * 1024 * 1024 // Default to 20 MB
}

fn default_max_concurrent_requests() -> u16 { 500 }

fn default_request_conn_timeout() -> u64 { 10 }

fn default_request_timeout() -> u64 { 35 }

fn default_request_total_timeout() -> u64 { 320 }

fn default_request_idle_timeout() -> u64 { 5 }

fn default_request_idle_per_host() -> u16 { 1 }

fn default_well_known_conn_timeout() -> u64 { 6 }

fn default_well_known_timeout() -> u64 { 10 }

fn default_federation_timeout() -> u64 { 300 }

fn default_federation_idle_timeout() -> u64 { 25 }

fn default_federation_idle_per_host() -> u16 { 1 }

fn default_sender_timeout() -> u64 { 180 }

fn default_sender_idle_timeout() -> u64 { 180 }

fn default_appservice_timeout() -> u64 { 120 }

fn default_appservice_idle_timeout() -> u64 { 300 }

fn default_pusher_idle_timeout() -> u64 { 15 }

fn default_max_fetch_prev_events() -> u16 { 100_u16 }

fn default_trusted_servers() -> Vec<OwnedServerName> { vec![OwnedServerName::try_from("matrix.org").unwrap()] }

fn default_log() -> String {
	// do debug logging by default for debug builds
	if cfg!(debug_assertions) {
		"debug".to_owned()
	} else {
		"info".to_owned()
	}
}

fn default_notification_push_path() -> String { "/_matrix/push/v1/notify".to_owned() }

fn default_turn_ttl() -> u64 { 60 * 60 * 24 }

fn default_presence_idle_timeout_s() -> u64 { 5 * 60 }

fn default_presence_offline_timeout_s() -> u64 { 30 * 60 }

fn default_typing_federation_timeout_s() -> u64 { 30 }

fn default_typing_client_timeout_min_s() -> u64 { 15 }

fn default_typing_client_timeout_max_s() -> u64 { 45 }

fn default_rocksdb_recovery_mode() -> u8 { 1 }

fn default_rocksdb_log_level() -> String { "error".to_owned() }

fn default_rocksdb_log_time_to_roll() -> usize { 0 }

fn default_rocksdb_max_log_files() -> usize { 3 }

fn default_rocksdb_max_log_file_size() -> usize {
	// 4 megabytes
	4 * 1024 * 1024
}

fn default_rocksdb_parallelism_threads() -> usize { 0 }

fn default_rocksdb_compression_algo() -> String { "zstd".to_owned() }

/// Default RocksDB compression level is 32767, which is internally read by
/// RocksDB as the default magic number and translated to the library's default
/// compression level as they all differ. See their `kDefaultCompressionLevel`.
#[allow(clippy::doc_markdown)]
fn default_rocksdb_compression_level() -> i32 { 32767 }

/// Default RocksDB compression level is 32767, which is internally read by
/// RocksDB as the default magic number and translated to the library's default
/// compression level as they all differ. See their `kDefaultCompressionLevel`.
#[allow(clippy::doc_markdown)]
fn default_rocksdb_bottommost_compression_level() -> i32 { 32767 }

// I know, it's a great name
pub(crate) fn default_default_room_version() -> RoomVersionId { RoomVersionId::V10 }

fn default_ip_range_denylist() -> Vec<String> {
	vec![
		"127.0.0.0/8".to_owned(),
		"10.0.0.0/8".to_owned(),
		"172.16.0.0/12".to_owned(),
		"192.168.0.0/16".to_owned(),
		"100.64.0.0/10".to_owned(),
		"192.0.0.0/24".to_owned(),
		"169.254.0.0/16".to_owned(),
		"192.88.99.0/24".to_owned(),
		"198.18.0.0/15".to_owned(),
		"192.0.2.0/24".to_owned(),
		"198.51.100.0/24".to_owned(),
		"203.0.113.0/24".to_owned(),
		"224.0.0.0/4".to_owned(),
		"::1/128".to_owned(),
		"fe80::/10".to_owned(),
		"fc00::/7".to_owned(),
		"2001:db8::/32".to_owned(),
		"ff00::/8".to_owned(),
		"fec0::/10".to_owned(),
	]
}

fn default_url_preview_max_spider_size() -> usize {
	384_000 // 384KB
}

fn default_new_user_displayname_suffix() -> String { "🏳️‍⚧️".to_owned() }

fn default_sentry_traces_sample_rate() -> f32 { 0.15 }

fn default_startup_netburst_keep() -> i64 { 50 }
