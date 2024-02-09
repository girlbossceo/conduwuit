use std::{
    collections::BTreeMap,
    fmt,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
};

use figment::Figment;

use itertools::Itertools;
use regex::RegexSet;
use ruma::{OwnedServerName, RoomVersionId};
use serde::{de::IgnoredAny, Deserialize};
use tracing::{debug, error, warn};

mod proxy;

use self::proxy::ProxyConfig;

/// all the config options for conduwuit
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// [`IpAddr`] conduwuit will listen on (can be IPv4 or IPv6)
    #[serde(default = "default_address")]
    pub address: IpAddr,
    /// default TCP port conduwuit will listen on
    #[serde(default = "default_port")]
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub unix_socket_path: Option<PathBuf>,
    #[serde(default = "default_unix_socket_perms")]
    pub unix_socket_perms: u32,
    pub server_name: OwnedServerName,
    #[serde(default = "default_database_backend")]
    pub database_backend: String,
    pub database_path: String,
    #[serde(default = "default_db_cache_capacity_mb")]
    pub db_cache_capacity_mb: f64,
    #[serde(default = "true_fn")]
    pub enable_lightning_bolt: bool,
    #[serde(default = "true_fn")]
    pub allow_check_for_updates: bool,
    #[serde(default = "default_conduit_cache_capacity_modifier")]
    pub conduit_cache_capacity_modifier: f64,
    #[serde(default = "default_pdu_cache_capacity")]
    pub pdu_cache_capacity: u32,
    #[serde(default = "default_cleanup_second_interval")]
    pub cleanup_second_interval: u32,
    #[serde(default = "default_max_request_size")]
    pub max_request_size: u32,
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: u16,
    #[serde(default = "default_max_fetch_prev_events")]
    pub max_fetch_prev_events: u16,
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
    pub allow_device_name_federation: bool,
    #[serde(default = "true_fn")]
    pub allow_room_creation: bool,
    #[serde(default = "true_fn")]
    pub allow_unstable_room_versions: bool,
    #[serde(default = "default_default_room_version")]
    pub default_room_version: RoomVersionId,
    pub well_known_client: Option<String>,
    pub well_known_server: Option<String>,
    #[serde(default)]
    pub allow_jaeger: bool,
    #[serde(default)]
    pub tracing_flame: bool,
    #[serde(default)]
    pub proxy: ProxyConfig,
    pub jwt_secret: Option<String>,
    #[serde(default = "default_trusted_servers")]
    pub trusted_servers: Vec<OwnedServerName>,
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

    #[serde(default = "default_rocksdb_log_level")]
    pub rocksdb_log_level: String,
    #[serde(default = "default_rocksdb_max_log_file_size")]
    pub rocksdb_max_log_file_size: usize,
    #[serde(default = "default_rocksdb_log_time_to_roll")]
    pub rocksdb_log_time_to_roll: usize,
    #[serde(default)]
    pub rocksdb_optimize_for_spinning_disks: bool,

    pub emergency_password: Option<String>,

    #[serde(default = "default_notification_push_path")]
    pub notification_push_path: String,

    #[serde(default)]
    pub allow_local_presence: bool,
    #[serde(default)]
    pub allow_incoming_presence: bool,
    #[serde(default)]
    pub allow_outgoing_presence: bool,
    #[serde(default = "default_presence_idle_timeout_s")]
    pub presence_idle_timeout_s: u64,
    #[serde(default = "default_presence_offline_timeout_s")]
    pub presence_offline_timeout_s: u64,

    #[serde(default)]
    pub zstd_compression: bool,

    #[serde(default)]
    pub allow_guest_registration: bool,

    #[serde(default = "Vec::new")]
    pub prevent_media_downloads_from: Vec<OwnedServerName>,

    #[serde(default = "default_ip_range_denylist")]
    pub ip_range_denylist: Vec<String>,

    #[serde(default = "RegexSet::empty")]
    #[serde(with = "serde_regex")]
    pub forbidden_room_names: RegexSet,

    #[serde(default = "RegexSet::empty")]
    #[serde(with = "serde_regex")]
    pub forbidden_usernames: RegexSet,

    #[serde(flatten)]
    pub catchall: BTreeMap<String, IgnoredAny>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub certs: String,
    pub key: String,
}

const DEPRECATED_KEYS: &[&str] = &["cache_capacity"];

impl Config {
    /// Iterates over all the keys in the config file and warns if there is a deprecated key specified
    pub fn warn_deprecated(&self) {
        debug!("Checking for deprecated config keys");
        let mut was_deprecated = false;
        for key in self
            .catchall
            .keys()
            .filter(|key| DEPRECATED_KEYS.iter().any(|s| s == key))
        {
            warn!("Config parameter \"{}\" is deprecated.", key);
            was_deprecated = true;
        }

        if was_deprecated {
            warn!("Read conduit documentation and check your configuration if any new configuration parameters should be adjusted");
        }
    }

    /// iterates over all the catchall keys (unknown config options) and warns if there are any.
    pub fn warn_unknown_key(&self) {
        debug!("Checking for unknown config keys");
        for key in self.catchall.keys().filter(
            |key| "config".to_owned().ne(key.to_owned()), /* "config" is expected */
        ) {
            warn!("Config parameter \"{}\" is unknown to conduwuit.", key);
        }
    }

    /// Checks the presence of the `address` and `unix_socket_path` keys in the raw_config, exiting the process if both keys were detected.
    pub fn is_dual_listening(&self, raw_config: Figment) -> bool {
        let check_address = raw_config.find_value("address");
        let check_unix_socket = raw_config.find_value("unix_socket_path");

        // are the check_address and check_unix_socket keys both Ok (specified) at the same time?
        if check_address.is_ok() && check_unix_socket.is_ok() {
            error!("TOML keys \"address\" and \"unix_socket_path\" were both defined. Please specify only one option.");
            return true;
        }

        false
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Prepare a list of config values to show
        let lines = [
            ("Server name", self.server_name.host()),
            ("Database backend", &self.database_backend),
            ("Database path", &self.database_path),
            (
                "Database cache capacity (MB)",
                &self.db_cache_capacity_mb.to_string(),
            ),
            (
                "Cache capacity modifier",
                &self.conduit_cache_capacity_modifier.to_string(),
            ),
            ("PDU cache capacity", &self.pdu_cache_capacity.to_string()),
            (
                "Cleanup interval in seconds",
                &self.cleanup_second_interval.to_string(),
            ),
            ("Maximum request size", &self.max_request_size.to_string()),
            (
                "Maximum concurrent requests",
                &self.max_concurrent_requests.to_string(),
            ),
            (
                "Allow registration (open registration)",
                &self.allow_registration.to_string(),
            ),
            (
                "Allow guest registration",
                &self.allow_guest_registration.to_string(),
            ),
            (
                "Enabled lightning bolt",
                &self.enable_lightning_bolt.to_string(),
            ),
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
                "Allow device name federation",
                &self.allow_device_name_federation.to_string(),
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
                "JWT secret",
                match self.jwt_secret {
                    Some(_) => "set",
                    None => "not set",
                },
            ),
            ("Trusted servers", {
                let mut lst = vec![];
                for server in &self.trusted_servers {
                    lst.push(server.host());
                }
                &lst.join(", ")
            }),
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
            (
                "zstd Response Body Compression",
                &self.zstd_compression.to_string(),
            ),
            ("RocksDB database log level", &self.rocksdb_log_level),
            (
                "RocksDB database log time-to-roll",
                &self.rocksdb_log_time_to_roll.to_string(),
            ),
            (
                "RocksDB database max log file size",
                &self.rocksdb_max_log_file_size.to_string(),
            ),
            (
                "RocksDB database optimize for spinning disks",
                &self.rocksdb_optimize_for_spinning_disks.to_string(),
            ),
            ("Prevent Media Downloads From", {
                let mut lst = vec![];
                for domain in &self.prevent_media_downloads_from {
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
            ("Forbidden room names", {
                &self.forbidden_room_names.patterns().iter().join(", ")
            }),
        ];

        let mut msg: String = "Active config values:\n\n".to_owned();

        for line in lines.into_iter().enumerate() {
            msg += &format!("{}: {}\n", line.1 .0, line.1 .1);
        }

        write!(f, "{msg}")
    }
}

fn true_fn() -> bool {
    true
}

fn default_address() -> IpAddr {
    Ipv4Addr::LOCALHOST.into()
}

fn default_port() -> u16 {
    8000
}

fn default_unix_socket_perms() -> u32 {
    660
}

fn default_database_backend() -> String {
    "rocksdb".to_owned()
}

fn default_db_cache_capacity_mb() -> f64 {
    300.0
}

fn default_conduit_cache_capacity_modifier() -> f64 {
    1.0
}

fn default_pdu_cache_capacity() -> u32 {
    150_000
}

fn default_cleanup_second_interval() -> u32 {
    60 // every minute
}

fn default_max_request_size() -> u32 {
    20 * 1024 * 1024 // Default to 20 MB
}

fn default_max_concurrent_requests() -> u16 {
    100
}

fn default_max_fetch_prev_events() -> u16 {
    100_u16
}

fn default_trusted_servers() -> Vec<OwnedServerName> {
    vec![OwnedServerName::try_from("matrix.org").unwrap()]
}

fn default_log() -> String {
    "warn,state_res=warn".to_owned()
}

fn default_notification_push_path() -> String {
    "/_matrix/push/v1/notify".to_owned()
}

fn default_turn_ttl() -> u64 {
    60 * 60 * 24
}

fn default_presence_idle_timeout_s() -> u64 {
    5 * 60
}

fn default_presence_offline_timeout_s() -> u64 {
    30 * 60
}

fn default_rocksdb_log_level() -> String {
    "warn".to_owned()
}

fn default_rocksdb_log_time_to_roll() -> usize {
    0
}

// I know, it's a great name
pub(crate) fn default_default_room_version() -> RoomVersionId {
    RoomVersionId::V10
}

fn default_rocksdb_max_log_file_size() -> usize {
    // 4 megabytes
    4 * 1024 * 1024
}

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
