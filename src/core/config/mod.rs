use std::{
	collections::{BTreeMap, BTreeSet},
	fmt,
	net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
	path::PathBuf,
};

use either::{
	Either,
	Either::{Left, Right},
};
use figment::providers::{Env, Format, Toml};
pub use figment::{value::Value as FigmentValue, Figment};
use itertools::Itertools;
use regex::RegexSet;
use ruma::{
	api::client::discovery::discover_support::ContactRole, OwnedRoomId, OwnedServerName, OwnedUserId, RoomVersionId,
};
use serde::{de::IgnoredAny, Deserialize};
use url::Url;

pub use self::check::check;
use self::proxy::ProxyConfig;
use crate::{error::Error, utils::sys, Err, Result};

pub mod check;
pub mod proxy;

/// all the config options for conduwuit
#[derive(Clone, Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct Config {
	/// [`IpAddr`] conduwuit will listen on (can be IPv4 or IPv6)
	#[serde(default = "default_address")]
	address: ListeningAddr,
	/// default TCP port(s) conduwuit will listen on
	#[serde(default = "default_port")]
	port: ListeningPort,
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
	#[serde(default = "default_cache_capacity_modifier", alias = "conduit_cache_capacity_modifier")]
	pub cache_capacity_modifier: f64,
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
	#[serde(default)]
	pub query_over_tcp_only: bool,
	#[serde(default = "default_ip_lookup_strategy")]
	pub ip_lookup_strategy: u8,

	#[serde(default = "default_max_request_size")]
	pub max_request_size: usize,
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
	#[serde(default = "default_sender_retry_backoff_limit")]
	pub sender_retry_backoff_limit: u64,
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
	pub federation_loopback: bool,
	#[serde(default)]
	pub allow_public_room_directory_over_federation: bool,
	#[serde(default)]
	pub allow_public_room_directory_without_auth: bool,
	#[serde(default)]
	pub turn_allow_guests: bool,
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
	#[serde(default = "default_jaeger_filter")]
	pub jaeger_filter: String,
	#[serde(default)]
	pub tracing_flame: bool,
	#[serde(default = "default_tracing_flame_filter")]
	pub tracing_flame_filter: String,
	#[serde(default = "default_tracing_flame_output_path")]
	pub tracing_flame_output_path: String,
	#[serde(default)]
	pub proxy: ProxyConfig,
	pub jwt_secret: Option<String>,
	#[serde(default = "default_trusted_servers")]
	pub trusted_servers: Vec<OwnedServerName>,
	#[serde(default = "true_fn")]
	pub query_trusted_key_servers_first: bool,
	#[serde(default = "default_log")]
	pub log: String,
	#[serde(default = "default_openid_token_ttl")]
	pub openid_token_ttl: u64,
	#[serde(default)]
	pub turn_username: String,
	#[serde(default)]
	pub turn_password: String,
	#[serde(default = "Vec::new")]
	pub turn_uris: Vec<String>,
	#[serde(default)]
	pub turn_secret: String,
	pub turn_secret_file: Option<PathBuf>,
	#[serde(default = "default_turn_ttl")]
	pub turn_ttl: u64,

	#[serde(default = "Vec::new")]
	pub auto_join_rooms: Vec<OwnedRoomId>,
	#[serde(default)]
	pub auto_deactivate_banned_room_attempts: bool,

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
	#[serde(default = "true_fn")]
	pub rocksdb_direct_io: bool,
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
	pub rocksdb_secondary: bool,
	#[serde(default)]
	pub rocksdb_compaction_prio_idle: bool,
	#[serde(default = "true_fn")]
	pub rocksdb_compaction_ioprio_idle: bool,
	#[serde(default = "true_fn")]
	pub rocksdb_compaction: bool,
	#[serde(default = "default_rocksdb_stats_level")]
	pub rocksdb_stats_level: u8,

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

	#[serde(default = "true_fn")]
	pub allow_legacy_media: bool,
	#[serde(default = "true_fn")]
	pub freeze_legacy_media: bool,
	#[serde(default = "true_fn")]
	pub media_startup_check: bool,
	#[serde(default)]
	pub media_compat_file_link: bool,
	#[serde(default)]
	pub prune_missing_media: bool,
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
	#[serde(default = "true_fn")]
	pub admin_escape_commands: bool,
	#[serde(default)]
	pub admin_console_automatic: bool,
	#[serde(default)]
	pub admin_execute: Vec<String>,
	#[serde(default)]
	pub admin_execute_errors_ignore: bool,
	#[serde(default = "default_admin_log_capture")]
	pub admin_log_capture: String,
	#[serde(default = "default_admin_room_tag")]
	pub admin_room_tag: String,

	#[serde(default)]
	pub sentry: bool,
	#[serde(default = "default_sentry_endpoint")]
	pub sentry_endpoint: Option<Url>,
	#[serde(default)]
	pub sentry_send_server_name: bool,
	#[serde(default = "default_sentry_traces_sample_rate")]
	pub sentry_traces_sample_rate: f32,
	#[serde(default)]
	pub sentry_attach_stacktrace: bool,
	#[serde(default = "true_fn")]
	pub sentry_send_panic: bool,
	#[serde(default = "true_fn")]
	pub sentry_send_error: bool,
	#[serde(default = "default_sentry_filter")]
	pub sentry_filter: String,

	#[serde(default)]
	pub tokio_console: bool,

	#[serde(default)]
	pub test: BTreeSet<String>,

	#[serde(flatten)]
	#[allow(clippy::zero_sized_map_values)] // this is a catchall, the map shouldn't be zero at runtime
	catchall: BTreeMap<String, IgnoredAny>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
	pub certs: String,
	pub key: String,
	#[serde(default)]
	/// Whether to listen and allow for HTTP and HTTPS connections (insecure!)
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

#[derive(Deserialize, Clone, Debug)]
#[serde(transparent)]
struct ListeningPort {
	#[serde(with = "either::serde_untagged")]
	ports: Either<u16, Vec<u16>>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(transparent)]
struct ListeningAddr {
	#[serde(with = "either::serde_untagged")]
	addrs: Either<IpAddr, Vec<IpAddr>>,
}

const DEPRECATED_KEYS: &[&str; 9] = &[
	"cache_capacity",
	"conduit_cache_capacity_modifier",
	"max_concurrent_requests",
	"well_known_client",
	"well_known_server",
	"well_known_support_page",
	"well_known_support_role",
	"well_known_support_email",
	"well_known_support_mxid",
];

impl Config {
	/// Pre-initialize config
	pub fn load(paths: &Option<Vec<PathBuf>>) -> Result<Figment> {
		let raw_config = if let Some(config_file_env) = Env::var("CONDUIT_CONFIG") {
			Figment::new().merge(Toml::file(config_file_env).nested())
		} else if let Some(config_file_arg) = Env::var("CONDUWUIT_CONFIG") {
			Figment::new().merge(Toml::file(config_file_arg).nested())
		} else if let Some(config_file_args) = paths {
			let mut figment = Figment::new();

			for config in config_file_args {
				figment = figment.merge(Toml::file(config).nested());
			}

			figment
		} else {
			Figment::new()
		};

		Ok(raw_config
			.merge(Env::prefixed("CONDUIT_").global().split("__"))
			.merge(Env::prefixed("CONDUWUIT_").global().split("__")))
	}

	/// Finalize config
	pub fn new(raw_config: &Figment) -> Result<Self> {
		let config = match raw_config.extract::<Self>() {
			Err(e) => return Err!("There was a problem with your configuration file: {e}"),
			Ok(config) => config,
		};

		// don't start if we're listening on both UNIX sockets and TCP at same time
		check::is_dual_listening(raw_config)?;

		Ok(config)
	}

	#[must_use]
	pub fn get_bind_addrs(&self) -> Vec<SocketAddr> {
		let mut addrs = Vec::with_capacity(
			self.get_bind_hosts()
				.len()
				.saturating_add(self.get_bind_ports().len()),
		);
		for host in &self.get_bind_hosts() {
			for port in &self.get_bind_ports() {
				addrs.push(SocketAddr::new(*host, *port));
			}
		}

		addrs
	}

	fn get_bind_hosts(&self) -> Vec<IpAddr> {
		match &self.address.addrs {
			Left(addr) => vec![*addr],
			Right(addrs) => addrs.clone(),
		}
	}

	fn get_bind_ports(&self) -> Vec<u16> {
		match &self.port.ports {
			Left(port) => vec![*port],
			Right(ports) => ports.clone(),
		}
	}

	pub fn check(&self) -> Result<(), Error> { check(self) }
}

impl fmt::Display for Config {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Active config values:\n\n").expect("wrote line to formatter stream");
		let mut line = |key: &str, val: &str| {
			writeln!(f, "{key}: {val}").expect("wrote line to formatter stream");
		};

		line("Server name", self.server_name.host());
		line("Database backend", &self.database_backend);
		line("Database path", &self.database_path.to_string_lossy());
		line(
			"Database backup path",
			self.database_backup_path
				.as_ref()
				.map_or("", |path| path.to_str().unwrap_or("")),
		);
		line("Database backups to keep", &self.database_backups_to_keep.to_string());
		line("Database cache capacity (MB)", &self.db_cache_capacity_mb.to_string());
		line("Cache capacity modifier", &self.cache_capacity_modifier.to_string());
		line("PDU cache capacity", &self.pdu_cache_capacity.to_string());
		line("Auth chain cache capacity", &self.auth_chain_cache_capacity.to_string());
		line("Short eventid cache capacity", &self.shorteventid_cache_capacity.to_string());
		line("Eventid short cache capacity", &self.eventidshort_cache_capacity.to_string());
		line("Short statekey cache capacity", &self.shortstatekey_cache_capacity.to_string());
		line("Statekey short cache capacity", &self.statekeyshort_cache_capacity.to_string());
		line(
			"Server visibility cache capacity",
			&self.server_visibility_cache_capacity.to_string(),
		);
		line(
			"User visibility cache capacity",
			&self.user_visibility_cache_capacity.to_string(),
		);
		line("Stateinfo cache capacity", &self.stateinfo_cache_capacity.to_string());
		line(
			"Roomid space hierarchy cache capacity",
			&self.roomid_spacehierarchy_cache_capacity.to_string(),
		);
		line("DNS cache entry limit", &self.dns_cache_entries.to_string());
		line("DNS minimum TTL", &self.dns_min_ttl.to_string());
		line("DNS minimum NXDOMAIN TTL", &self.dns_min_ttl_nxdomain.to_string());
		line("DNS attempts", &self.dns_attempts.to_string());
		line("DNS timeout", &self.dns_timeout.to_string());
		line("DNS fallback to TCP", &self.dns_tcp_fallback.to_string());
		line("DNS query over TCP only", &self.query_over_tcp_only.to_string());
		line("Query all nameservers", &self.query_all_nameservers.to_string());
		line("Maximum request size (bytes)", &self.max_request_size.to_string());
		line("Sender retry backoff limit", &self.sender_retry_backoff_limit.to_string());
		line("Request connect timeout", &self.request_conn_timeout.to_string());
		line("Request timeout", &self.request_timeout.to_string());
		line("Request total timeout", &self.request_total_timeout.to_string());
		line("Idle connections per host", &self.request_idle_per_host.to_string());
		line("Request pool idle timeout", &self.request_idle_timeout.to_string());
		line("Well_known connect timeout", &self.well_known_conn_timeout.to_string());
		line("Well_known timeout", &self.well_known_timeout.to_string());
		line("Federation timeout", &self.federation_timeout.to_string());
		line("Federation pool idle per host", &self.federation_idle_per_host.to_string());
		line("Federation pool idle timeout", &self.federation_idle_timeout.to_string());
		line("Sender timeout", &self.sender_timeout.to_string());
		line("Sender pool idle timeout", &self.sender_idle_timeout.to_string());
		line("Appservice timeout", &self.appservice_timeout.to_string());
		line("Appservice pool idle timeout", &self.appservice_idle_timeout.to_string());
		line("Pusher pool idle timeout", &self.pusher_idle_timeout.to_string());
		line("Allow registration", &self.allow_registration.to_string());
		line(
			"Registration token",
			if self.registration_token.is_some() {
				"set"
			} else {
				"not set (open registration!)"
			},
		);
		line(
			"Allow guest registration (inherently false if allow registration is false)",
			&self.allow_guest_registration.to_string(),
		);
		line(
			"Log guest registrations in admin room",
			&self.log_guest_registrations.to_string(),
		);
		line(
			"Allow guests to auto join rooms",
			&self.allow_guests_auto_join_rooms.to_string(),
		);
		line("New user display name suffix", &self.new_user_displayname_suffix);
		line("Allow encryption", &self.allow_encryption.to_string());
		line("Allow federation", &self.allow_federation.to_string());
		line("Federation loopback", &self.federation_loopback.to_string());
		line(
			"Allow incoming federated presence requests (updates)",
			&self.allow_incoming_presence.to_string(),
		);
		line(
			"Allow outgoing federated presence requests (updates)",
			&self.allow_outgoing_presence.to_string(),
		);
		line(
			"Allow local presence requests (updates)",
			&self.allow_local_presence.to_string(),
		);
		line(
			"Allow incoming remote read receipts",
			&self.allow_incoming_read_receipts.to_string(),
		);
		line(
			"Allow outgoing remote read receipts",
			&self.allow_outgoing_read_receipts.to_string(),
		);
		line(
			"Block non-admin room invites (local and remote, admins can still send and receive invites)",
			&self.block_non_admin_invites.to_string(),
		);
		line("Enable admin escape commands", &self.admin_escape_commands.to_string());
		line(
			"Activate admin console after startup",
			&self.admin_console_automatic.to_string(),
		);
		line("Execute admin commands after startup", &self.admin_execute.join(", "));
		line(
			"Continue startup even if some commands fail",
			&self.admin_execute_errors_ignore.to_string(),
		);
		line("Filter for admin command log capture", &self.admin_log_capture);
		line("Admin room tag", &self.admin_room_tag);
		line("Allow outgoing federated typing", &self.allow_outgoing_typing.to_string());
		line("Allow incoming federated typing", &self.allow_incoming_typing.to_string());
		line(
			"Incoming federated typing timeout",
			&self.typing_federation_timeout_s.to_string(),
		);
		line("Client typing timeout minimum", &self.typing_client_timeout_min_s.to_string());
		line("Client typing timeout maxmimum", &self.typing_client_timeout_max_s.to_string());
		line("Allow device name federation", &self.allow_device_name_federation.to_string());
		line(
			"Allow incoming profile lookup federation requests",
			&self.allow_profile_lookup_federation_requests.to_string(),
		);
		line(
			"Auto deactivate banned room join attempts",
			&self.auto_deactivate_banned_room_attempts.to_string(),
		);
		line("Notification push path", &self.notification_push_path);
		line("Allow room creation", &self.allow_room_creation.to_string());
		line(
			"Allow public room directory over federation",
			&self.allow_public_room_directory_over_federation.to_string(),
		);
		line(
			"Allow public room directory without authentication",
			&self.allow_public_room_directory_without_auth.to_string(),
		);
		line(
			"Lockdown public room directory (only allow admins to publish)",
			&self.lockdown_public_room_directory.to_string(),
		);
		line(
			"JWT secret",
			match self.jwt_secret {
				Some(_) => "set",
				None => "not set",
			},
		);
		line(
			"Trusted key servers",
			&self
				.trusted_servers
				.iter()
				.map(|server| server.host())
				.join(", "),
		);
		line(
			"Query Trusted Key Servers First",
			&self.query_trusted_key_servers_first.to_string(),
		);
		line("OpenID Token TTL", &self.openid_token_ttl.to_string());
		line(
			"TURN username",
			if self.turn_username.is_empty() {
				"not set"
			} else {
				&self.turn_username
			},
		);
		line("TURN password", {
			if self.turn_password.is_empty() {
				"not set"
			} else {
				"set"
			}
		});
		line("TURN secret", {
			if self.turn_secret.is_empty() && self.turn_secret_file.is_none() {
				"not set"
			} else {
				"set"
			}
		});
		line("TURN secret file path", {
			self.turn_secret_file
				.as_ref()
				.map_or("", |path| path.to_str().unwrap_or_default())
		});
		line("Turn TTL", &self.turn_ttl.to_string());
		line("Turn URIs", {
			let mut lst = Vec::with_capacity(self.turn_uris.len());
			for item in self.turn_uris.iter().cloned().enumerate() {
				let (_, uri): (usize, String) = item;
				lst.push(uri);
			}
			&lst.join(", ")
		});
		line("Auto Join Rooms", {
			let mut lst = Vec::with_capacity(self.auto_join_rooms.len());
			for room in &self.auto_join_rooms {
				lst.push(room);
			}
			&lst.into_iter().join(", ")
		});
		line("Zstd HTTP Compression", &self.zstd_compression.to_string());
		line("Gzip HTTP Compression", &self.gzip_compression.to_string());
		line("Brotli HTTP Compression", &self.brotli_compression.to_string());
		line("RocksDB database LOG level", &self.rocksdb_log_level);
		line("RocksDB database LOG to stderr", &self.rocksdb_log_stderr.to_string());
		line("RocksDB database LOG time-to-roll", &self.rocksdb_log_time_to_roll.to_string());
		line("RocksDB Max LOG Files", &self.rocksdb_max_log_files.to_string());
		line(
			"RocksDB database max LOG file size",
			&self.rocksdb_max_log_file_size.to_string(),
		);
		line(
			"RocksDB database optimize for spinning disks",
			&self.rocksdb_optimize_for_spinning_disks.to_string(),
		);
		line("RocksDB Direct-IO", &self.rocksdb_direct_io.to_string());
		line("RocksDB Parallelism Threads", &self.rocksdb_parallelism_threads.to_string());
		line("RocksDB Compression Algorithm", &self.rocksdb_compression_algo);
		line("RocksDB Compression Level", &self.rocksdb_compression_level.to_string());
		line(
			"RocksDB Bottommost Compression Level",
			&self.rocksdb_bottommost_compression_level.to_string(),
		);
		line(
			"RocksDB Bottommost Level Compression",
			&self.rocksdb_bottommost_compression.to_string(),
		);
		line("RocksDB Recovery Mode", &self.rocksdb_recovery_mode.to_string());
		line("RocksDB Repair Mode", &self.rocksdb_repair.to_string());
		line("RocksDB Read-only Mode", &self.rocksdb_read_only.to_string());
		line("RocksDB Secondary Mode", &self.rocksdb_secondary.to_string());
		line(
			"RocksDB Compaction Idle Priority",
			&self.rocksdb_compaction_prio_idle.to_string(),
		);
		line(
			"RocksDB Compaction Idle IOPriority",
			&self.rocksdb_compaction_ioprio_idle.to_string(),
		);
		line("RocksDB Compaction enabled", &self.rocksdb_compaction.to_string());
		line("RocksDB Statistics level", &self.rocksdb_stats_level.to_string());
		line("Media integrity checks on startup", &self.media_startup_check.to_string());
		line("Media compatibility filesystem links", &self.media_compat_file_link.to_string());
		line("Prune missing media from database", &self.prune_missing_media.to_string());
		line("Allow legacy (unauthenticated) media", &self.allow_legacy_media.to_string());
		line("Freeze legacy (unauthenticated) media", &self.freeze_legacy_media.to_string());
		line("Prevent Media Downloads From", {
			let mut lst = Vec::with_capacity(self.prevent_media_downloads_from.len());
			for domain in &self.prevent_media_downloads_from {
				lst.push(domain.host());
			}
			&lst.join(", ")
		});
		line("Forbidden Remote Server Names (\"Global\" ACLs)", {
			let mut lst = Vec::with_capacity(self.forbidden_remote_server_names.len());
			for domain in &self.forbidden_remote_server_names {
				lst.push(domain.host());
			}
			&lst.join(", ")
		});
		line("Forbidden Remote Room Directory Server Names", {
			let mut lst = Vec::with_capacity(self.forbidden_remote_room_directory_server_names.len());
			for domain in &self.forbidden_remote_room_directory_server_names {
				lst.push(domain.host());
			}
			&lst.join(", ")
		});
		line("Outbound Request IP Range (CIDR) Denylist", {
			let mut lst = Vec::with_capacity(self.ip_range_denylist.len());
			for item in self.ip_range_denylist.iter().cloned().enumerate() {
				let (_, ip): (usize, String) = item;
				lst.push(ip);
			}
			&lst.join(", ")
		});
		line("Forbidden usernames", {
			&self.forbidden_usernames.patterns().iter().join(", ")
		});
		line("Forbidden room aliases", {
			&self.forbidden_alias_names.patterns().iter().join(", ")
		});
		line(
			"URL preview domain contains allowlist",
			&self.url_preview_domain_contains_allowlist.join(", "),
		);
		line(
			"URL preview domain explicit allowlist",
			&self.url_preview_domain_explicit_allowlist.join(", "),
		);
		line(
			"URL preview domain explicit denylist",
			&self.url_preview_domain_explicit_denylist.join(", "),
		);
		line(
			"URL preview URL contains allowlist",
			&self.url_preview_url_contains_allowlist.join(", "),
		);
		line("URL preview maximum spider size", &self.url_preview_max_spider_size.to_string());
		line("URL preview check root domain", &self.url_preview_check_root_domain.to_string());
		line(
			"Allow check for updates / announcements check",
			&self.allow_check_for_updates.to_string(),
		);
		line("Enable netburst on startup", &self.startup_netburst.to_string());
		#[cfg(feature = "sentry_telemetry")]
		line("Sentry.io reporting and tracing", &self.sentry.to_string());
		#[cfg(feature = "sentry_telemetry")]
		line("Sentry.io send server_name in logs", &self.sentry_send_server_name.to_string());
		#[cfg(feature = "sentry_telemetry")]
		line("Sentry.io tracing sample rate", &self.sentry_traces_sample_rate.to_string());
		line("Sentry.io attach stacktrace", &self.sentry_attach_stacktrace.to_string());
		line("Sentry.io send panics", &self.sentry_send_panic.to_string());
		line("Sentry.io send errors", &self.sentry_send_error.to_string());
		line("Sentry.io tracing filter", &self.sentry_filter);
		line(
			"Well-known server name",
			self.well_known
				.server
				.as_ref()
				.map_or("", |server| server.as_str()),
		);
		line(
			"Well-known client URL",
			self.well_known
				.client
				.as_ref()
				.map_or("", |url| url.as_str()),
		);
		line(
			"Well-known support email",
			self.well_known
				.support_email
				.as_ref()
				.map_or("", |str| str.as_ref()),
		);
		line(
			"Well-known support Matrix ID",
			self.well_known
				.support_mxid
				.as_ref()
				.map_or("", |mxid| mxid.as_str()),
		);
		line(
			"Well-known support role",
			self.well_known
				.support_role
				.as_ref()
				.map_or("", |role| role.as_str()),
		);
		line(
			"Well-known support page/URL",
			self.well_known
				.support_page
				.as_ref()
				.map_or("", |url| url.as_str()),
		);
		line("Enable the tokio-console", &self.tokio_console.to_string());

		Ok(())
	}
}

fn true_fn() -> bool { true }

fn default_address() -> ListeningAddr {
	ListeningAddr {
		addrs: Right(vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]),
	}
}

fn default_port() -> ListeningPort {
	ListeningPort {
		ports: Left(8008),
	}
}

fn default_unix_socket_perms() -> u32 { 660 }

fn default_database_backups_to_keep() -> i16 { 1 }

fn default_database_backend() -> String { "rocksdb".to_owned() }

fn default_db_cache_capacity_mb() -> f64 { 128.0 + parallelism_scaled_f64(64.0) }

fn default_pdu_cache_capacity() -> u32 { parallelism_scaled_u32(10_000).saturating_add(100_000) }

fn default_cache_capacity_modifier() -> f64 { 1.0 }

fn default_auth_chain_cache_capacity() -> u32 { parallelism_scaled_u32(10_000).saturating_add(100_000) }

fn default_shorteventid_cache_capacity() -> u32 { parallelism_scaled_u32(50_000).saturating_add(100_000) }

fn default_eventidshort_cache_capacity() -> u32 { parallelism_scaled_u32(25_000).saturating_add(100_000) }

fn default_shortstatekey_cache_capacity() -> u32 { parallelism_scaled_u32(10_000).saturating_add(100_000) }

fn default_statekeyshort_cache_capacity() -> u32 { parallelism_scaled_u32(10_000).saturating_add(100_000) }

fn default_server_visibility_cache_capacity() -> u32 { parallelism_scaled_u32(500) }

fn default_user_visibility_cache_capacity() -> u32 { parallelism_scaled_u32(1000) }

fn default_stateinfo_cache_capacity() -> u32 { parallelism_scaled_u32(1000) }

fn default_roomid_spacehierarchy_cache_capacity() -> u32 { parallelism_scaled_u32(1000) }

fn default_dns_cache_entries() -> u32 { 32768 }

fn default_dns_min_ttl() -> u64 { 60 * 180 }

fn default_dns_min_ttl_nxdomain() -> u64 { 60 * 60 * 24 * 3 }

fn default_dns_attempts() -> u16 { 10 }

fn default_dns_timeout() -> u64 { 10 }

fn default_ip_lookup_strategy() -> u8 { 5 }

fn default_max_request_size() -> usize {
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

fn default_appservice_timeout() -> u64 { 35 }

fn default_appservice_idle_timeout() -> u64 { 300 }

fn default_pusher_idle_timeout() -> u64 { 15 }

fn default_max_fetch_prev_events() -> u16 { 100_u16 }

fn default_tracing_flame_filter() -> String {
	cfg!(debug_assertions)
		.then_some("trace,h2=off")
		.unwrap_or("info")
		.to_owned()
}

fn default_jaeger_filter() -> String {
	cfg!(debug_assertions)
		.then_some("trace,h2=off")
		.unwrap_or("info")
		.to_owned()
}

fn default_tracing_flame_output_path() -> String { "./tracing.folded".to_owned() }

fn default_trusted_servers() -> Vec<OwnedServerName> { vec![OwnedServerName::try_from("matrix.org").unwrap()] }

/// do debug logging by default for debug builds
#[must_use]
pub fn default_log() -> String {
	cfg!(debug_assertions)
		.then_some("debug")
		.unwrap_or("info")
		.to_owned()
}

fn default_notification_push_path() -> String { "/_matrix/push/v1/notify".to_owned() }

fn default_openid_token_ttl() -> u64 { 60 * 60 }

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

fn default_rocksdb_stats_level() -> u8 { 1 }

// I know, it's a great name
#[must_use]
pub fn default_default_room_version() -> RoomVersionId { RoomVersionId::V10 }

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
	Url::parse("https://fe2eb4536aa04949e28eff3128d64757@o4506996327251968.ingest.us.sentry.io/4506996334657536").ok()
}

fn default_sentry_traces_sample_rate() -> f32 { 0.15 }

fn default_sentry_filter() -> String { "info".to_owned() }

fn default_startup_netburst_keep() -> i64 { 50 }

fn default_admin_log_capture() -> String {
	cfg!(debug_assertions)
		.then_some("debug")
		.unwrap_or("info")
		.to_owned()
}

fn default_admin_room_tag() -> String { "m.server_notice".to_owned() }

#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
fn parallelism_scaled_f64(val: f64) -> f64 { val * (sys::available_parallelism() as f64) }

fn parallelism_scaled_u32(val: u32) -> u32 {
	let val = val.try_into().expect("failed to cast u32 to usize");
	parallelism_scaled(val).try_into().unwrap_or(u32::MAX)
}

fn parallelism_scaled(val: usize) -> usize { val.saturating_mul(sys::available_parallelism()) }
