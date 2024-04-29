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
struct ListeningPort {
	#[serde(with = "either::serde_untagged")]
	ports: Either<u16, Vec<u16>>,
}

/// all the config options for conduwuit
#[derive(Clone, Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Config {
	/// [`IpAddr`] conduwuit will listen on (can be IPv4 or IPv6)
	#[serde(default = "default_address")]
	pub(crate) address: IpAddr,
	/// default TCP port(s) conduwuit will listen on
	#[serde(default = "default_port")]
	port: ListeningPort,
	pub(crate) tls: Option<TlsConfig>,
	pub(crate) unix_socket_path: Option<PathBuf>,
	#[serde(default = "default_unix_socket_perms")]
	pub(crate) unix_socket_perms: u32,
	pub(crate) server_name: OwnedServerName,
	#[serde(default = "default_database_backend")]
	pub(crate) database_backend: String,
	pub(crate) database_path: PathBuf,
	pub(crate) database_backup_path: Option<PathBuf>,
	#[serde(default = "default_database_backups_to_keep")]
	pub(crate) database_backups_to_keep: i16,
	#[serde(default = "default_db_cache_capacity_mb")]
	pub(crate) db_cache_capacity_mb: f64,
	#[serde(default = "default_new_user_displayname_suffix")]
	pub(crate) new_user_displayname_suffix: String,
	#[serde(default)]
	pub(crate) allow_check_for_updates: bool,

	#[serde(default = "default_pdu_cache_capacity")]
	pub(crate) pdu_cache_capacity: u32,
	#[serde(default = "default_conduit_cache_capacity_modifier")]
	pub(crate) conduit_cache_capacity_modifier: f64,
	#[serde(default = "default_auth_chain_cache_capacity")]
	pub(crate) auth_chain_cache_capacity: u32,
	#[serde(default = "default_shorteventid_cache_capacity")]
	pub(crate) shorteventid_cache_capacity: u32,
	#[serde(default = "default_eventidshort_cache_capacity")]
	pub(crate) eventidshort_cache_capacity: u32,
	#[serde(default = "default_shortstatekey_cache_capacity")]
	pub(crate) shortstatekey_cache_capacity: u32,
	#[serde(default = "default_statekeyshort_cache_capacity")]
	pub(crate) statekeyshort_cache_capacity: u32,
	#[serde(default = "default_server_visibility_cache_capacity")]
	pub(crate) server_visibility_cache_capacity: u32,
	#[serde(default = "default_user_visibility_cache_capacity")]
	pub(crate) user_visibility_cache_capacity: u32,
	#[serde(default = "default_stateinfo_cache_capacity")]
	pub(crate) stateinfo_cache_capacity: u32,
	#[serde(default = "default_roomid_spacehierarchy_cache_capacity")]
	pub(crate) roomid_spacehierarchy_cache_capacity: u32,

	#[serde(default = "default_cleanup_second_interval")]
	pub(crate) cleanup_second_interval: u32,

	#[serde(default = "default_dns_cache_entries")]
	pub(crate) dns_cache_entries: u32,
	#[serde(default = "default_dns_min_ttl")]
	pub(crate) dns_min_ttl: u64,
	#[serde(default = "default_dns_min_ttl_nxdomain")]
	pub(crate) dns_min_ttl_nxdomain: u64,
	#[serde(default = "default_dns_attempts")]
	pub(crate) dns_attempts: u16,
	#[serde(default = "default_dns_timeout")]
	pub(crate) dns_timeout: u64,
	#[serde(default = "true_fn")]
	pub(crate) dns_tcp_fallback: bool,
	#[serde(default = "true_fn")]
	pub(crate) query_all_nameservers: bool,
	#[serde(default = "default_ip_lookup_strategy")]
	pub(crate) ip_lookup_strategy: u8,

	#[serde(default = "default_max_request_size")]
	pub(crate) max_request_size: u32,
	#[serde(default = "default_max_fetch_prev_events")]
	pub(crate) max_fetch_prev_events: u16,

	#[serde(default = "default_request_conn_timeout")]
	pub(crate) request_conn_timeout: u64,
	#[serde(default = "default_request_timeout")]
	pub(crate) request_timeout: u64,
	#[serde(default = "default_request_total_timeout")]
	pub(crate) request_total_timeout: u64,
	#[serde(default = "default_request_idle_timeout")]
	pub(crate) request_idle_timeout: u64,
	#[serde(default = "default_request_idle_per_host")]
	pub(crate) request_idle_per_host: u16,
	#[serde(default = "default_well_known_conn_timeout")]
	pub(crate) well_known_conn_timeout: u64,
	#[serde(default = "default_well_known_timeout")]
	pub(crate) well_known_timeout: u64,
	#[serde(default = "default_federation_timeout")]
	pub(crate) federation_timeout: u64,
	#[serde(default = "default_federation_idle_timeout")]
	pub(crate) federation_idle_timeout: u64,
	#[serde(default = "default_federation_idle_per_host")]
	pub(crate) federation_idle_per_host: u16,
	#[serde(default = "default_sender_timeout")]
	pub(crate) sender_timeout: u64,
	#[serde(default = "default_sender_idle_timeout")]
	pub(crate) sender_idle_timeout: u64,
	#[serde(default = "default_sender_retry_backoff_limit")]
	pub(crate) sender_retry_backoff_limit: u64,
	#[serde(default = "default_appservice_timeout")]
	pub(crate) appservice_timeout: u64,
	#[serde(default = "default_appservice_idle_timeout")]
	pub(crate) appservice_idle_timeout: u64,
	#[serde(default = "default_pusher_idle_timeout")]
	pub(crate) pusher_idle_timeout: u64,

	#[serde(default)]
	pub(crate) allow_registration: bool,
	#[serde(default)]
	pub(crate) yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse: bool,
	pub(crate) registration_token: Option<String>,
	#[serde(default = "true_fn")]
	pub(crate) allow_encryption: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_federation: bool,
	#[serde(default)]
	pub(crate) allow_public_room_directory_over_federation: bool,
	#[serde(default)]
	pub(crate) allow_public_room_directory_without_auth: bool,
	#[serde(default)]
	pub(crate) lockdown_public_room_directory: bool,
	#[serde(default)]
	pub(crate) allow_device_name_federation: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_profile_lookup_federation_requests: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_room_creation: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_unstable_room_versions: bool,
	#[serde(default = "default_default_room_version")]
	pub(crate) default_room_version: RoomVersionId,
	#[serde(default)]
	pub(crate) well_known: WellKnownConfig,
	#[serde(default)]
	#[cfg(feature = "perf_measurements")]
	pub(crate) allow_jaeger: bool,
	#[serde(default)]
	#[cfg(feature = "perf_measurements")]
	pub(crate) tracing_flame: bool,
	#[serde(default = "default_tracing_flame_filter")]
	#[cfg(feature = "perf_measurements")]
	pub(crate) tracing_flame_filter: String,
	#[serde(default = "default_tracing_flame_output_path")]
	#[cfg(feature = "perf_measurements")]
	pub(crate) tracing_flame_output_path: String,
	#[serde(default)]
	pub(crate) proxy: ProxyConfig,
	pub(crate) jwt_secret: Option<String>,
	#[serde(default = "default_trusted_servers")]
	pub(crate) trusted_servers: Vec<OwnedServerName>,
	#[serde(default = "true_fn")]
	pub(crate) query_trusted_key_servers_first: bool,
	#[serde(default = "default_log")]
	pub(crate) log: String,
	#[serde(default)]
	pub(crate) turn_username: String,
	#[serde(default)]
	pub(crate) turn_password: String,
	#[serde(default = "Vec::new")]
	pub(crate) turn_uris: Vec<String>,
	#[serde(default)]
	pub(crate) turn_secret: String,
	#[serde(default = "default_turn_ttl")]
	pub(crate) turn_ttl: u64,

	#[serde(default = "Vec::new")]
	pub(crate) auto_join_rooms: Vec<OwnedRoomId>,

	#[serde(default = "default_rocksdb_log_level")]
	pub(crate) rocksdb_log_level: String,
	#[serde(default)]
	pub(crate) rocksdb_log_stderr: bool,
	#[serde(default = "default_rocksdb_max_log_file_size")]
	pub(crate) rocksdb_max_log_file_size: usize,
	#[serde(default = "default_rocksdb_log_time_to_roll")]
	pub(crate) rocksdb_log_time_to_roll: usize,
	#[serde(default)]
	pub(crate) rocksdb_optimize_for_spinning_disks: bool,
	#[serde(default = "default_rocksdb_parallelism_threads")]
	pub(crate) rocksdb_parallelism_threads: usize,
	#[serde(default = "default_rocksdb_max_log_files")]
	pub(crate) rocksdb_max_log_files: usize,
	#[serde(default = "default_rocksdb_compression_algo")]
	pub(crate) rocksdb_compression_algo: String,
	#[serde(default = "default_rocksdb_compression_level")]
	pub(crate) rocksdb_compression_level: i32,
	#[serde(default = "default_rocksdb_bottommost_compression_level")]
	pub(crate) rocksdb_bottommost_compression_level: i32,
	#[serde(default)]
	pub(crate) rocksdb_bottommost_compression: bool,
	#[serde(default = "default_rocksdb_recovery_mode")]
	pub(crate) rocksdb_recovery_mode: u8,
	#[serde(default)]
	pub(crate) rocksdb_repair: bool,
	#[serde(default)]
	pub(crate) rocksdb_read_only: bool,
	#[serde(default)]
	pub(crate) rocksdb_periodic_cleanup: bool,
	#[serde(default)]
	pub(crate) rocksdb_compaction_prio_idle: bool,
	#[serde(default = "true_fn")]
	pub(crate) rocksdb_compaction_ioprio_idle: bool,

	pub(crate) emergency_password: Option<String>,

	#[serde(default = "default_notification_push_path")]
	pub(crate) notification_push_path: String,

	#[serde(default = "true_fn")]
	pub(crate) allow_local_presence: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_incoming_presence: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_outgoing_presence: bool,
	#[serde(default = "default_presence_idle_timeout_s")]
	pub(crate) presence_idle_timeout_s: u64,
	#[serde(default = "default_presence_offline_timeout_s")]
	pub(crate) presence_offline_timeout_s: u64,
	#[serde(default = "true_fn")]
	pub(crate) presence_timeout_remote_users: bool,

	#[serde(default = "true_fn")]
	pub(crate) allow_incoming_read_receipts: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_outgoing_read_receipts: bool,

	#[serde(default = "true_fn")]
	pub(crate) allow_outgoing_typing: bool,
	#[serde(default = "true_fn")]
	pub(crate) allow_incoming_typing: bool,
	#[serde(default = "default_typing_federation_timeout_s")]
	pub(crate) typing_federation_timeout_s: u64,
	#[serde(default = "default_typing_client_timeout_min_s")]
	pub(crate) typing_client_timeout_min_s: u64,
	#[serde(default = "default_typing_client_timeout_max_s")]
	pub(crate) typing_client_timeout_max_s: u64,

	#[serde(default)]
	pub(crate) zstd_compression: bool,
	#[serde(default)]
	pub(crate) gzip_compression: bool,
	#[serde(default)]
	pub(crate) brotli_compression: bool,

	#[serde(default)]
	pub(crate) allow_guest_registration: bool,
	#[serde(default)]
	pub(crate) log_guest_registrations: bool,
	#[serde(default)]
	pub(crate) allow_guests_auto_join_rooms: bool,

	#[serde(default = "Vec::new")]
	pub(crate) prevent_media_downloads_from: Vec<OwnedServerName>,
	#[serde(default = "Vec::new")]
	pub(crate) forbidden_remote_server_names: Vec<OwnedServerName>,
	#[serde(default = "Vec::new")]
	pub(crate) forbidden_remote_room_directory_server_names: Vec<OwnedServerName>,

	#[serde(default = "default_ip_range_denylist")]
	pub(crate) ip_range_denylist: Vec<String>,

	#[serde(default = "Vec::new")]
	pub(crate) url_preview_domain_contains_allowlist: Vec<String>,
	#[serde(default = "Vec::new")]
	pub(crate) url_preview_domain_explicit_allowlist: Vec<String>,
	#[serde(default = "Vec::new")]
	pub(crate) url_preview_domain_explicit_denylist: Vec<String>,
	#[serde(default = "Vec::new")]
	pub(crate) url_preview_url_contains_allowlist: Vec<String>,
	#[serde(default = "default_url_preview_max_spider_size")]
	pub(crate) url_preview_max_spider_size: usize,
	#[serde(default)]
	pub(crate) url_preview_check_root_domain: bool,

	#[serde(default = "RegexSet::empty")]
	#[serde(with = "serde_regex")]
	pub(crate) forbidden_alias_names: RegexSet,

	#[serde(default = "RegexSet::empty")]
	#[serde(with = "serde_regex")]
	pub(crate) forbidden_usernames: RegexSet,

	#[serde(default = "true_fn")]
	pub(crate) startup_netburst: bool,
	#[serde(default = "default_startup_netburst_keep")]
	pub(crate) startup_netburst_keep: i64,

	#[serde(default)]
	pub(crate) block_non_admin_invites: bool,

	#[serde(default)]
	pub(crate) sentry: bool,
	#[serde(default = "default_sentry_endpoint")]
	pub(crate) sentry_endpoint: Option<Url>,
	#[serde(default)]
	pub(crate) sentry_send_server_name: bool,
	#[serde(default = "default_sentry_traces_sample_rate")]
	pub(crate) sentry_traces_sample_rate: f32,

	#[serde(flatten)]
	#[allow(clippy::zero_sized_map_values)] // this is a catchall, the map shouldn't be zero at runtime
	catchall: BTreeMap<String, IgnoredAny>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TlsConfig {
	pub(crate) certs: String,
	pub(crate) key: String,
	#[serde(default)]
	/// Whether to listen and allow for HTTP and HTTPS connections (insecure!)
	/// Only works / does something if the `axum_dual_protocol` feature flag was
	/// built
	pub(crate) dual_protocol: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub(crate) struct WellKnownConfig {
	pub(crate) client: Option<Url>,
	pub(crate) server: Option<OwnedServerName>,
	pub(crate) support_page: Option<Url>,
	pub(crate) support_role: Option<ContactRole>,
	pub(crate) support_email: Option<String>,
	pub(crate) support_mxid: Option<OwnedUserId>,
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
	pub(crate) fn new(path: Option<PathBuf>) -> Result<Self, Error> {
		let raw_config = if let Some(config_file_env) = Env::var("CONDUIT_CONFIG") {
			Figment::new()
				.merge(Toml::file(config_file_env).nested())
				.merge(Env::prefixed("CONDUIT_").global())
				.merge(Env::prefixed("CONDUWUIT_").global())
		} else if let Some(config_file_arg) = Env::var("CONDUWUIT_CONFIG") {
			Figment::new()
				.merge(Toml::file(config_file_arg).nested())
				.merge(Env::prefixed("CONDUIT_").global())
				.merge(Env::prefixed("CONDUWUIT_").global())
		} else if let Some(config_file_arg) = path {
			Figment::new()
				.merge(Toml::file(config_file_arg).nested())
				.merge(Env::prefixed("CONDUIT_").global())
				.merge(Env::prefixed("CONDUWUIT_").global())
		} else {
			Figment::new()
				.merge(Env::prefixed("CONDUIT_").global())
				.merge(Env::prefixed("CONDUWUIT_").global())
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
	pub(crate) fn warn_deprecated(&self) {
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
	pub(crate) fn warn_unknown_key(&self) {
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
	pub(crate) fn get_bind_addrs(&self) -> Vec<SocketAddr> {
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

	pub(crate) fn check(&self) -> Result<(), Error> { check(self) }
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
			("DNS minimum TTL", &self.dns_min_ttl.to_string()),
			("DNS minimum NXDOMAIN TTL", &self.dns_min_ttl_nxdomain.to_string()),
			("DNS attempts", &self.dns_attempts.to_string()),
			("DNS timeout", &self.dns_timeout.to_string()),
			("DNS fallback to TCP", &self.dns_tcp_fallback.to_string()),
			("Query all nameservers", &self.query_all_nameservers.to_string()),
			("Maximum request size (bytes)", &self.max_request_size.to_string()),
			("Sender retry backoff limit", &self.sender_retry_backoff_limit.to_string()),
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
			#[cfg(feature = "rocksdb")]
			("RocksDB Repair Mode", &self.rocksdb_repair.to_string()),
			#[cfg(feature = "rocksdb")]
			("RocksDB Read-only Mode", &self.rocksdb_read_only.to_string()),
			#[cfg(feature = "rocksdb")]
			("RocksDB Periodic Cleanup", &self.rocksdb_periodic_cleanup.to_string()),
			#[cfg(feature = "rocksdb")]
			(
				"RocksDB Compaction Idle Priority",
				&self.rocksdb_compaction_prio_idle.to_string(),
			),
			#[cfg(feature = "rocksdb")]
			(
				"RocksDB Compaction Idle IOPriority",
				&self.rocksdb_compaction_ioprio_idle.to_string(),
			),
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

fn default_dns_cache_entries() -> u32 { 32768 }

fn default_dns_min_ttl() -> u64 { 60 * 180 }

fn default_dns_min_ttl_nxdomain() -> u64 { 60 * 60 * 24 * 3 }

fn default_dns_attempts() -> u16 { 10 }

fn default_dns_timeout() -> u64 { 10 }

fn default_ip_lookup_strategy() -> u8 { 5 }

fn default_max_request_size() -> u32 {
	20 * 1024 * 1024 // Default to 20 MB
}

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

fn default_sender_retry_backoff_limit() -> u64 { 86400 }

fn default_appservice_timeout() -> u64 { 120 }

fn default_appservice_idle_timeout() -> u64 { 300 }

fn default_pusher_idle_timeout() -> u64 { 15 }

fn default_max_fetch_prev_events() -> u16 { 100_u16 }

#[cfg(feature = "perf_measurements")]
fn default_tracing_flame_filter() -> String { "trace,h2=off".to_owned() }

#[cfg(feature = "perf_measurements")]
fn default_tracing_flame_output_path() -> String { "./tracing.folded".to_owned() }

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

fn default_sentry_endpoint() -> Option<Url> {
	Url::parse("https://fe2eb4536aa04949e28eff3128d64757@o4506996327251968.ingest.us.sentry.io/4506996334657536")
		.unwrap()
		.into()
}

fn default_sentry_traces_sample_rate() -> f32 { 0.15 }

fn default_startup_netburst_keep() -> i64 { 50 }
