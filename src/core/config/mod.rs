pub mod check;
pub mod proxy;

use std::{
	collections::{BTreeMap, BTreeSet},
	fmt,
	net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
	path::PathBuf,
};

use conduit_macros::config_example_generator;
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
use crate::{err, error::Error, utils::sys, Result};

/// all the config options for conduwuit
#[config_example_generator]
#[derive(Clone, Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::bare_urls)]
pub struct Config {
	/// The server_name is the pretty name of this server. It is used as a
	/// suffix for user and room ids. Examples: matrix.org, conduit.rs
	///
	/// The Conduit server needs all /_matrix/ requests to be reachable at
	/// https://your.server.name/ on port 443 (client-server) and 8448 (federation).
	///
	/// If that's not possible for you, you can create /.well-known files to
	/// redirect requests (delegation). See
	/// https://spec.matrix.org/latest/client-server-api/#getwell-knownmatrixclient
	/// and
	/// https://spec.matrix.org/v1.9/server-server-api/#getwell-knownmatrixserver
	/// for more information.
	///
	/// YOU NEED TO EDIT THIS
	pub server_name: OwnedServerName,

	/// Database backend: Only rocksdb is supported.
	/// default address (IPv4 or IPv6) conduwuit will listen on. Generally you
	/// want this to be localhost (127.0.0.1 / ::1). If you are using Docker or
	/// a container NAT networking setup, you likely need this to be 0.0.0.0.
	/// To listen multiple addresses, specify a vector e.g. ["127.0.0.1", "::1"]
	///
	/// default if unspecified is both IPv4 and IPv6 localhost: ["127.0.0.1",
	/// "::1"]
	#[serde(default = "default_address")]
	address: ListeningAddr,

	/// The port(s) conduwuit will be running on. You need to set up a reverse
	/// proxy such as Caddy or Nginx so all requests to /_matrix on port 443
	/// and 8448 will be forwarded to the conduwuit instance running on this
	/// port Docker users: Don't change this, you'll need to map an external
	/// port to this. To listen on multiple ports, specify a vector e.g. [8080,
	/// 8448]
	///
	/// default if unspecified is 8008
	#[serde(default = "default_port")]
	port: ListeningPort,

	pub tls: Option<TlsConfig>,

	/// Uncomment unix_socket_path to listen on a UNIX socket at the specified
	/// path. If listening on a UNIX socket, you must remove/comment the
	/// 'address' key if defined and add your reverse proxy to the 'conduwuit'
	/// group, unless world RW permissions are specified with unix_socket_perms
	/// (666 minimum).
	pub unix_socket_path: Option<PathBuf>,

	#[serde(default = "default_unix_socket_perms")]
	pub unix_socket_perms: u32,

	#[serde(default = "default_database_backend")]
	pub database_backend: String,

	/// This is the only directory where conduwuit will save its data, including
	/// media. Note: this was previously "/var/lib/matrix-conduit"
	pub database_path: PathBuf,

	pub database_backup_path: Option<PathBuf>,

	#[serde(default = "default_database_backups_to_keep")]
	pub database_backups_to_keep: i16,

	/// Set this to any float value in megabytes for conduwuit to tell the
	/// database engine that this much memory is available for database-related
	/// caches.  May be useful if you have significant memory to spare to
	/// increase performance.  Defaults to 256.0
	#[serde(default = "default_db_cache_capacity_mb")]
	pub db_cache_capacity_mb: f64,

	/// Option to control adding arbitrary text to the end of the user's
	/// displayname upon registration with a space before the text. This was the
	/// lightning bolt emoji option, just replaced with support for adding your
	/// own custom text or emojis.  To disable, set this to "" (an empty string)
	/// Defaults to "🏳️⚧️" (trans pride flag)
	#[serde(default = "default_new_user_displayname_suffix")]
	pub new_user_displayname_suffix: String,

	/// If enabled, conduwuit will send a simple GET request periodically to
	/// `https://pupbrain.dev/check-for-updates/stable` for any new
	/// announcements made. Despite the name, this is not an update check
	/// endpoint, it is simply an announcement check endpoint. Defaults to
	/// false.
	#[serde(default)]
	pub allow_check_for_updates: bool,

	#[serde(default = "default_pdu_cache_capacity")]
	pub pdu_cache_capacity: u32,

	/// Set this to any float value to multiply conduwuit's in-memory LRU caches
	/// with. May be useful if you have significant memory to spare to increase
	/// performance.
	///
	/// This was previously called `conduit_cache_capacity_modifier`
	///
	/// Defaults to 1.0.
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

	/// Maximum entries stored in DNS memory-cache. The size of an entry may
	/// vary so please take care if raising this value excessively. Only
	/// decrease this when using an external DNS cache. Please note
	/// that systemd does *not* count as an external cache, even when configured
	/// to do so.
	#[serde(default = "default_dns_cache_entries")]
	pub dns_cache_entries: u32,

	/// Minimum time-to-live in seconds for entries in the DNS cache. The
	/// default may appear high to most administrators; this is by design. Only
	/// decrease this if you are using an external DNS cache.
	#[serde(default = "default_dns_min_ttl")]
	pub dns_min_ttl: u64,

	/// Minimum time-to-live in seconds for NXDOMAIN entries in the DNS cache.
	/// This value is critical for the server to federate efficiently.
	/// NXDOMAIN's are assumed to not be returning to the federation
	/// and aggressively cached rather than constantly rechecked.
	///
	/// Defaults to 3 days as these are *very rarely* false negatives.
	#[serde(default = "default_dns_min_ttl_nxdomain")]
	pub dns_min_ttl_nxdomain: u64,

	/// Number of retries after a timeout.
	#[serde(default = "default_dns_attempts")]
	pub dns_attempts: u16,

	/// The number of seconds to wait for a reply to a DNS query. Please note
	/// that recursive queries can take up to several seconds for some domains,
	/// so this value should not be too low.
	#[serde(default = "default_dns_timeout")]
	pub dns_timeout: u64,

	/// Fallback to TCP on DNS errors. Set this to false if unsupported by
	/// nameserver.
	#[serde(default = "true_fn")]
	pub dns_tcp_fallback: bool,

	/// Enable to query all nameservers until the domain is found. Referred to
	/// as "trust_negative_responses" in hickory_reso> This can avoid useless
	/// DNS queries if the first nameserver responds with NXDOMAIN or an empty
	/// NOERROR response.
	///
	/// The default is to query one nameserver and stop (false).
	#[serde(default = "true_fn")]
	pub query_all_nameservers: bool,

	/// Enables using *only* TCP for querying your specified nameservers instead
	/// of UDP.
	///
	/// You very likely do *not* want this. hickory-resolver already falls back
	/// to TCP on UDP errors. Defaults to false
	#[serde(default)]
	pub query_over_tcp_only: bool,

	/// DNS A/AAAA record lookup strategy
	///
	/// Takes a number of one of the following options:
	/// 1 - Ipv4Only (Only query for A records, no AAAA/IPv6)
	/// 2 - Ipv6Only (Only query for AAAA records, no A/IPv4)
	/// 3 - Ipv4AndIpv6 (Query for A and AAAA records in parallel, uses whatever
	/// returns a successful response first) 4 - Ipv6thenIpv4 (Query for AAAA
	/// record, if that fails then query the A record) 5 - Ipv4thenIpv6 (Query
	/// for A record, if that fails then query the AAAA record)
	///
	/// If you don't have IPv6 networking, then for better performance it may be
	/// suitable to set this to Ipv4Only (1) as you will never ever use the
	/// AAAA record contents even if the AAAA record is successful instead of
	/// the A record.
	///
	/// Defaults to 5 - Ipv4ThenIpv6 as this is the most compatible and IPv4
	/// networking is currently the most prevalent.
	#[serde(default = "default_ip_lookup_strategy")]
	pub ip_lookup_strategy: u8,

	/// Max request size for file uploads
	#[serde(default = "default_max_request_size")]
	pub max_request_size: usize,

	#[serde(default = "default_max_fetch_prev_events")]
	pub max_fetch_prev_events: u16,

	/// Default/base connection timeout.
	/// This is used only by URL previews and update/news endpoint checks
	///
	/// Defaults to 10 seconds
	#[serde(default = "default_request_conn_timeout")]
	pub request_conn_timeout: u64,

	/// Default/base request timeout. The time waiting to receive more data from
	/// another server. This is used only by URL previews, update/news, and
	/// misc endpoint checks
	///
	/// Defaults to 35 seconds
	#[serde(default = "default_request_timeout")]
	pub request_timeout: u64,

	/// Default/base request total timeout. The time limit for a whole request.
	/// This is set very high to not cancel healthy requests while serving as a
	/// backstop. This is used only by URL previews and update/news endpoint
	/// checks
	///
	/// Defaults to 320 seconds
	#[serde(default = "default_request_total_timeout")]
	pub request_total_timeout: u64,

	/// Default/base idle connection pool timeout
	/// This is used only by URL previews and update/news endpoint checks
	///
	/// Defaults to 5 seconds
	#[serde(default = "default_request_idle_timeout")]
	pub request_idle_timeout: u64,

	/// Default/base max idle connections per host
	/// This is used only by URL previews and update/news endpoint checks
	///
	/// Defaults to 1 as generally the same open connection can be re-used
	#[serde(default = "default_request_idle_per_host")]
	pub request_idle_per_host: u16,

	/// Federation well-known resolution connection timeout
	///
	/// Defaults to 6 seconds
	#[serde(default = "default_well_known_conn_timeout")]
	pub well_known_conn_timeout: u64,

	/// Federation HTTP well-known resolution request timeout
	///
	/// Defaults to 10 seconds
	#[serde(default = "default_well_known_timeout")]
	pub well_known_timeout: u64,

	/// Federation client request timeout
	/// You most definitely want this to be high to account for extremely large
	/// room joins, slow homeservers, your own resources etc.
	///
	/// Defaults to 300 seconds
	#[serde(default = "default_federation_timeout")]
	pub federation_timeout: u64,

	/// Federation client idle connection pool timeout
	///
	/// Defaults to 25 seconds
	#[serde(default = "default_federation_idle_timeout")]
	pub federation_idle_timeout: u64,

	/// Federation client max idle connections per host
	///
	/// Defaults to 1 as generally the same open connection can be re-used
	#[serde(default = "default_federation_idle_per_host")]
	pub federation_idle_per_host: u16,

	/// Federation sender request timeout
	/// The time it takes for the remote server to process sent transactions can
	/// take a while.
	///
	/// Defaults to 180 seconds
	#[serde(default = "default_sender_timeout")]
	pub sender_timeout: u64,

	/// Federation sender idle connection pool timeout
	///
	/// Defaults to 180 seconds
	#[serde(default = "default_sender_idle_timeout")]
	pub sender_idle_timeout: u64,

	/// Federation sender transaction retry backoff limit
	///
	/// Defaults to 86400 seconds
	#[serde(default = "default_sender_retry_backoff_limit")]
	pub sender_retry_backoff_limit: u64,

	/// Appservice URL request connection timeout
	///
	/// Defaults to 35 seconds as generally appservices are hosted within the
	/// same network
	#[serde(default = "default_appservice_timeout")]
	pub appservice_timeout: u64,

	/// Appservice URL idle connection pool timeout
	///
	/// Defaults to 300 seconds
	#[serde(default = "default_appservice_idle_timeout")]
	pub appservice_idle_timeout: u64,

	/// Notification gateway pusher idle connection pool timeout
	///
	/// Defaults to 15 seconds
	#[serde(default = "default_pusher_idle_timeout")]
	pub pusher_idle_timeout: u64,

	/// Enables registration. If set to false, no users can register on this
	/// server.
	///
	/// If set to true without a token configured, users can register with no
	/// form of 2nd- step only if you set
	/// `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse` to
	/// true in your config.
	///
	/// If you would like registration only via token reg, please configure
	/// `registration_token` or `registration_token_file`.
	#[serde(default)]
	pub allow_registration: bool,

	#[serde(default)]
	pub yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse: bool,

	/// A static registration token that new users will have to provide when
	/// creating an account. If unset and `allow_registration` is true,
	/// registration is open without any condition. YOU NEED TO EDIT THIS.
	pub registration_token: Option<String>,

	/// Path to a file on the system that gets read for the registration token
	///
	/// conduwuit must be able to access the file, and it must not be empty
	///
	/// no default
	pub registration_token_file: Option<PathBuf>,

	/// controls whether encrypted rooms and events are allowed (default true)
	#[serde(default = "true_fn")]
	pub allow_encryption: bool,

	/// controls whether federation is allowed or not
	/// defaults to true
	#[serde(default = "true_fn")]
	pub allow_federation: bool,

	#[serde(default)]
	pub federation_loopback: bool,

	/// Set this to true to allow your server's public room directory to be
	/// federated. Set this to false to protect against /publicRooms spiders,
	/// but will forbid external users from viewing your server's public room
	/// directory. If federation is disabled entirely (`allow_federation`),
	/// this is inherently false.
	#[serde(default)]
	pub allow_public_room_directory_over_federation: bool,

	/// Set this to true to allow your server's public room directory to be
	/// queried without client authentication (access token) through the Client
	/// APIs. Set this to false to protect against /publicRooms spiders.
	#[serde(default)]
	pub allow_public_room_directory_without_auth: bool,

	/// allow guests/unauthenticated users to access TURN credentials
	///
	/// this is the equivalent of Synapse's `turn_allow_guests` config option.
	/// this allows any unauthenticated user to call
	/// `/_matrix/client/v3/voip/turnServer`.
	///
	/// defaults to false
	#[serde(default)]
	pub turn_allow_guests: bool,

	/// Set this to true to lock down your server's public room directory and
	/// only allow admins to publish rooms to the room directory. Unpublishing
	/// is still allowed by all users with this enabled.
	///
	/// Defaults to false
	#[serde(default)]
	pub lockdown_public_room_directory: bool,

	/// Set this to true to allow federating device display names / allow
	/// external users to see your device display name. If federation is
	/// disabled entirely (`allow_federation`), this is inherently false. For
	/// privacy, this is best disabled.
	#[serde(default)]
	pub allow_device_name_federation: bool,

	/// Config option to allow or disallow incoming federation requests that
	/// obtain the profiles of our local users from
	/// `/_matrix/federation/v1/query/profile`
	///
	/// This is inherently false if `allow_federation` is disabled
	///
	/// Defaults to true
	#[serde(default = "true_fn")]
	pub allow_profile_lookup_federation_requests: bool,

	/// controls whether users are allowed to create rooms.
	/// appservices and admins are always allowed to create rooms
	/// defaults to true
	#[serde(default = "true_fn")]
	pub allow_room_creation: bool,

	/// Set to false to disable users from joining or creating room versions
	/// that aren't 100% officially supported by conduwuit.
	/// conduwuit officially supports room versions 6 - 10. conduwuit has
	/// experimental/unstable support for 3 - 5, and 11. Defaults to true.
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

	/// If the 'perf_measurements' feature is enabled, enables collecting folded
	/// stack trace profile of tracing spans using tracing_flame. The resulting
	/// profile can be visualized with inferno[1], speedscope[2], or a number of
	/// other tools. [1]: https://github.com/jonhoo/inferno
	/// [2]: www.speedscope.app
	#[serde(default)]
	pub tracing_flame: bool,

	#[serde(default = "default_tracing_flame_filter")]
	pub tracing_flame_filter: String,

	#[serde(default = "default_tracing_flame_output_path")]
	pub tracing_flame_output_path: String,

	#[serde(default)]
	pub proxy: ProxyConfig,

	pub jwt_secret: Option<String>,

	/// Servers listed here will be used to gather public keys of other servers
	/// (notary trusted key servers).
	///
	/// (Currently, conduwuit doesn't support batched key requests, so this list
	/// should only contain other Synapse servers) Defaults to `matrix.org`
	#[serde(default = "default_trusted_servers")]
	pub trusted_servers: Vec<OwnedServerName>,

	/// max log level for conduwuit. allows debug, info, warn, or error
	/// see also: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives
	/// **Caveat**:
	/// For release builds, the tracing crate is configured to only implement
	/// levels higher than error to avoid unnecessary overhead in the compiled
	/// binary from trace macros. For debug builds, this restriction is not
	/// applied.
	///
	/// Defaults to "info"
	#[serde(default = "default_log")]
	pub log: String,

	/// controls whether logs will be outputted with ANSI colours
	///
	/// defaults to true
	#[serde(default = "true_fn", alias = "log_colours")]
	pub log_colors: bool,

	/// OpenID token expiration/TTL in seconds
	///
	/// These are the OpenID tokens that are primarily used for Matrix account
	/// integrations, *not* OIDC/OpenID Connect/etc
	///
	/// Defaults to 3600 (1 hour)
	#[serde(default = "default_openid_token_ttl")]
	pub openid_token_ttl: u64,

	/// TURN username to provide the client
	///
	/// no default
	#[serde(default)]
	pub turn_username: String,

	/// TURN password to provide the client
	///
	/// no default
	#[serde(default)]
	pub turn_password: String,

	/// vector list of TURN URIs/servers to use
	///
	/// replace "example.turn.uri" with your TURN domain, such as the coturn
	/// "realm". if using TURN over TLS, replace "turn:" with "turns:"
	///
	/// No default
	#[serde(default = "Vec::new")]
	pub turn_uris: Vec<String>,

	/// TURN secret to use for generating the HMAC-SHA1 hash apart of username
	/// and password generation
	///
	/// this is more secure, but if needed you can use traditional
	/// username/password below.
	///
	/// no default
	#[serde(default)]
	pub turn_secret: String,

	/// TURN secret to use that's read from the file path specified
	///
	/// this takes priority over "turn_secret" first, and falls back to
	/// "turn_secret" if invalid or failed to open.
	///
	/// no default
	pub turn_secret_file: Option<PathBuf>,

	/// TURN TTL
	///
	/// Default is 86400 seconds
	#[serde(default = "default_turn_ttl")]
	pub turn_ttl: u64,

	/// List/vector of room **IDs** that conduwuit will make newly registered
	/// users join. The room IDs specified must be rooms that you have joined
	/// at least once on the server, and must be public.
	///
	/// No default.
	#[serde(default = "Vec::new")]
	pub auto_join_rooms: Vec<OwnedRoomId>,

	/// Config option to automatically deactivate the account of any user who
	/// attempts to join a:
	/// - banned room
	/// - forbidden room alias
	/// - room alias or ID with a forbidden server name
	///
	/// This may be useful if all your banned lists consist of toxic rooms or
	/// servers that no good faith user would ever attempt to join, and
	/// to automatically remediate the problem without any admin user
	/// intervention.
	///
	/// This will also make the user leave all rooms. Federation (e.g. remote
	/// room invites) are ignored here.
	///
	/// Defaults to false as rooms can be banned for non-moderation-related
	/// reasons
	#[serde(default)]
	pub auto_deactivate_banned_room_attempts: bool,

	/// RocksDB log level. This is not the same as conduwuit's log level. This
	/// is the log level for the RocksDB engine/library which show up in your
	/// database folder/path as `LOG` files. Defaults to error. conduwuit will
	/// typically log RocksDB errors as normal.
	#[serde(default = "default_rocksdb_log_level")]
	pub rocksdb_log_level: String,

	#[serde(default)]
	pub rocksdb_log_stderr: bool,

	/// Max RocksDB `LOG` file size before rotating in bytes. Defaults to 4MB.
	#[serde(default = "default_rocksdb_max_log_file_size")]
	pub rocksdb_max_log_file_size: usize,

	/// Time in seconds before RocksDB will forcibly rotate logs. Defaults to 0.
	#[serde(default = "default_rocksdb_log_time_to_roll")]
	pub rocksdb_log_time_to_roll: usize,

	/// Set this to true to use RocksDB config options that are tailored to HDDs
	/// (slower device storage)
	///
	/// It is worth noting that by default, conduwuit will use RocksDB with
	/// Direct IO enabled. *Generally* speaking this improves performance as it
	/// bypasses buffered I/O (system page cache). However there is a potential
	/// chance that Direct IO may cause issues with database operations if your
	/// setup is uncommon. This has been observed with FUSE filesystems, and
	/// possibly ZFS filesystem. RocksDB generally deals/corrects these issues
	/// but it cannot account for all setups. If you experience any weird
	/// RocksDB issues, try enabling this option as it turns off Direct IO and
	/// feel free to report in the conduwuit Matrix room if this option fixes
	/// your DB issues. See https://github.com/facebook/rocksdb/wiki/Direct-IO for more information.
	///
	/// Defaults to false
	#[serde(default)]
	pub rocksdb_optimize_for_spinning_disks: bool,

	/// Enables direct-io to increase database performance. This is enabled by
	/// default. Set this option to false if the database resides on a
	/// filesystem which does not support direct-io.
	#[serde(default = "true_fn")]
	pub rocksdb_direct_io: bool,

	/// Amount of threads that RocksDB will use for parallelism on database
	/// operatons such as cleanup, sync, flush, compaction, etc. Set to 0 to use
	/// all your logical threads.
	///
	/// Defaults to your CPU logical thread count.
	#[serde(default = "default_rocksdb_parallelism_threads")]
	pub rocksdb_parallelism_threads: usize,

	/// Maximum number of LOG files RocksDB will keep. This must *not* be set to
	/// 0. It must be at least 1. Defaults to 3 as these are not very useful.
	#[serde(default = "default_rocksdb_max_log_files")]
	pub rocksdb_max_log_files: usize,

	/// Type of RocksDB database compression to use.
	/// Available options are "zstd", "zlib", "bz2", "lz4", or "none"
	/// It is best to use ZSTD as an overall good balance between
	/// speed/performance, storage, IO amplification, and CPU usage.
	/// For more performance but less compression (more storage used) and less
	/// CPU usage, use LZ4. See https://github.com/facebook/rocksdb/wiki/Compression for more details.
	///
	/// "none" will disable compression.
	///
	/// Defaults to "zstd"
	#[serde(default = "default_rocksdb_compression_algo")]
	pub rocksdb_compression_algo: String,

	/// Level of compression the specified compression algorithm for RocksDB to
	/// use. Default is 32767, which is internally read by RocksDB as the
	/// default magic number and translated to the library's default
	/// compression level as they all differ.
	/// See their `kDefaultCompressionLevel`.
	#[serde(default = "default_rocksdb_compression_level")]
	pub rocksdb_compression_level: i32,

	/// Level of compression the specified compression algorithm for the
	/// bottommost level/data for RocksDB to use. Default is 32767, which is
	/// internally read by RocksDB as the default magic number and translated
	/// to the library's default compression level as they all differ.
	/// See their `kDefaultCompressionLevel`.
	///
	/// Since this is the bottommost level (generally old and least used data),
	/// it may be desirable to have a very high compression level here as it's
	/// lesss likely for this data to be used. Research your chosen compression
	/// algorithm.
	#[serde(default = "default_rocksdb_bottommost_compression_level")]
	pub rocksdb_bottommost_compression_level: i32,

	/// Whether to enable RocksDB "bottommost_compression".
	/// At the expense of more CPU usage, this will further compress the
	/// database to reduce more storage. It is recommended to use ZSTD
	/// compression with this for best compression results. See https://github.com/facebook/rocksdb/wiki/Compression for more details.
	///
	/// Defaults to false as this uses more CPU when compressing.
	#[serde(default)]
	pub rocksdb_bottommost_compression: bool,

	/// Database recovery mode (for RocksDB WAL corruption)
	///
	/// Use this option when the server reports corruption and refuses to start.
	/// Set mode 2 (PointInTime) to cleanly recover from this corruption. The
	/// server will continue from the last good state, several seconds or
	/// minutes prior to the crash. Clients may have to run "clear-cache &
	/// reload" to account for the rollback. Upon success, you may reset the
	/// mode back to default and restart again. Please note in some cases the
	/// corruption error may not be cleared for at least 30 minutes of
	/// operation in PointInTime mode.
	///
	/// As a very last ditch effort, if PointInTime does not fix or resolve
	/// anything, you can try mode 3 (SkipAnyCorruptedRecord) but this will
	/// leave the server in a potentially inconsistent state.
	///
	/// The default mode 1 (TolerateCorruptedTailRecords) will automatically
	/// drop the last entry in the database if corrupted during shutdown, but
	/// nothing more. It is extraordinarily unlikely this will desynchronize
	/// clients. To disable any form of silent rollback set mode 0
	/// (AbsoluteConsistency).
	///
	/// The options are:
	/// 0 = AbsoluteConsistency
	/// 1 = TolerateCorruptedTailRecords (default)
	/// 2 = PointInTime (use me if trying to recover)
	/// 3 = SkipAnyCorruptedRecord (you now voided your Conduwuit warranty)
	///
	/// See https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes for more information
	///
	/// Defaults to 1 (TolerateCorruptedTailRecords)
	#[serde(default = "default_rocksdb_recovery_mode")]
	pub rocksdb_recovery_mode: u8,

	/// Database repair mode (for RocksDB SST corruption)
	///
	/// Use this option when the server reports corruption while running or
	/// panics. If the server refuses to start use the recovery mode options
	/// first. Corruption errors containing the acronym 'SST' which occur after
	/// startup will likely require this option.
	///
	/// - Backing up your database directory is recommended prior to running the
	///   repair.
	/// - Disabling repair mode and restarting the server is recommended after
	///   running the repair.
	///
	/// Defaults to false
	#[serde(default)]
	pub rocksdb_repair: bool,

	#[serde(default)]
	pub rocksdb_read_only: bool,

	#[serde(default)]
	pub rocksdb_secondary: bool,

	/// Enables idle CPU priority for compaction thread. This is not enabled by
	/// default to prevent compaction from falling too far behind on busy
	/// systems.
	#[serde(default)]
	pub rocksdb_compaction_prio_idle: bool,

	/// Enables idle IO priority for compaction thread. This prevents any
	/// unexpected lag in the server's operation and is usually a good idea.
	/// Enabled by default.
	#[serde(default = "true_fn")]
	pub rocksdb_compaction_ioprio_idle: bool,

	#[serde(default = "true_fn")]
	pub rocksdb_compaction: bool,

	/// Level of statistics collection. Some admin commands to display database
	/// statistics may require this option to be set. Database performance may
	/// be impacted by higher settings.
	///
	/// Option is a number ranging from 0 to 6:
	/// 0 = No statistics.
	/// 1 = No statistics in release mode (default).
	/// 2 to 3 = Statistics with no performance impact.
	/// 3 to 5 = Statistics with possible performance impact.
	/// 6 = All statistics.
	///
	/// Defaults to 1 (No statistics, except in debug-mode)
	#[serde(default = "default_rocksdb_stats_level")]
	pub rocksdb_stats_level: u8,

	pub emergency_password: Option<String>,

	#[serde(default = "default_notification_push_path")]
	pub notification_push_path: String,

	/// Config option to control local (your server only) presence
	/// updates/requests. Defaults to true. Note that presence on conduwuit is
	/// very fast unlike Synapse's. If using outgoing presence, this MUST be
	/// enabled.
	#[serde(default = "true_fn")]
	pub allow_local_presence: bool,

	/// Config option to control incoming federated presence updates/requests.
	/// Defaults to true. This option receives presence updates from other
	/// servers, but does not send any unless `allow_outgoing_presence` is true.
	/// Note that presence on conduwuit is very fast unlike Synapse's.
	#[serde(default = "true_fn")]
	pub allow_incoming_presence: bool,

	/// Config option to control outgoing presence updates/requests. Defaults to
	/// true. This option sends presence updates to other servers, but does not
	/// receive any unless `allow_incoming_presence` is true.
	/// Note that presence on conduwuit is very fast unlike Synapse's.
	/// If using outgoing presence, you MUST enable `allow_local_presence` as
	/// well.
	#[serde(default = "true_fn")]
	pub allow_outgoing_presence: bool,

	/// Config option to control how many seconds before presence updates that
	/// you are idle. Defaults to 5 minutes.
	#[serde(default = "default_presence_idle_timeout_s")]
	pub presence_idle_timeout_s: u64,

	/// Config option to control how many seconds before presence updates that
	/// you are offline. Defaults to 30 minutes.
	#[serde(default = "default_presence_offline_timeout_s")]
	pub presence_offline_timeout_s: u64,

	/// Config option to enable the presence idle timer for remote users.
	/// Disabling is offered as an optimization for servers participating in
	/// many large rooms or when resources are limited. Disabling it may cause
	/// incorrect presence states (i.e. stuck online) to be seen for some
	/// remote users. Defaults to true.
	#[serde(default = "true_fn")]
	pub presence_timeout_remote_users: bool,

	/// Config option to control whether we should receive remote incoming read
	/// receipts. Defaults to true.
	#[serde(default = "true_fn")]
	pub allow_incoming_read_receipts: bool,

	/// Config option to control whether we should send read receipts to remote
	/// servers. Defaults to true.
	#[serde(default = "true_fn")]
	pub allow_outgoing_read_receipts: bool,

	/// Config option to control outgoing typing updates to federation. Defaults
	/// to true.
	#[serde(default = "true_fn")]
	pub allow_outgoing_typing: bool,

	/// Config option to control incoming typing updates from federation.
	/// Defaults to true.
	#[serde(default = "true_fn")]
	pub allow_incoming_typing: bool,

	/// Config option to control maximum time federation user can indicate
	/// typing.
	#[serde(default = "default_typing_federation_timeout_s")]
	pub typing_federation_timeout_s: u64,

	/// Config option to control minimum time local client can indicate typing.
	/// This does not override a client's request to stop typing. It only
	/// enforces a minimum value in case of no stop request.
	#[serde(default = "default_typing_client_timeout_min_s")]
	pub typing_client_timeout_min_s: u64,

	/// Config option to control maximum time local client can indicate typing.
	#[serde(default = "default_typing_client_timeout_max_s")]
	pub typing_client_timeout_max_s: u64,

	/// Set this to true for conduwuit to compress HTTP response bodies using
	/// zstd. This option does nothing if conduwuit was not built with
	/// `zstd_compression` feature. Please be aware that enabling HTTP
	/// compression may weaken TLS. Most users should not need to enable this.
	/// See https://breachattack.com/ and https://wikipedia.org/wiki/BREACH
	/// before deciding to enable this.
	#[serde(default)]
	pub zstd_compression: bool,

	/// Set this to true for conduwuit to compress HTTP response bodies using
	/// gzip. This option does nothing if conduwuit was not built with
	/// `gzip_compression` feature. Please be aware that enabling HTTP
	/// compression may weaken TLS. Most users should not need to enable this.
	/// See https://breachattack.com/ and https://wikipedia.org/wiki/BREACH before
	/// deciding to enable this.
	#[serde(default)]
	pub gzip_compression: bool,

	/// Set this to true for conduwuit to compress HTTP response bodies using
	/// brotli. This option does nothing if conduwuit was not built with
	/// `brotli_compression` feature. Please be aware that enabling HTTP
	/// compression may weaken TLS. Most users should not need to enable this.
	/// See https://breachattack.com/ and https://wikipedia.org/wiki/BREACH before
	/// deciding to enable this.
	#[serde(default)]
	pub brotli_compression: bool,

	/// Set to true to allow user type "guest" registrations. Element attempts
	/// to register guest users automatically. Defaults to false
	#[serde(default)]
	pub allow_guest_registration: bool,

	/// Set to true to log guest registrations in the admin room.
	/// Defaults to false as it may be noisy or unnecessary.
	#[serde(default)]
	pub log_guest_registrations: bool,

	/// Set to true to allow guest registrations/users to auto join any rooms
	/// specified in `auto_join_rooms` Defaults to false
	#[serde(default)]
	pub allow_guests_auto_join_rooms: bool,

	/// Config option to control whether the legacy unauthenticated Matrix media
	/// repository endpoints will be enabled. These endpoints consist of:
	/// - /_matrix/media/*/config
	/// - /_matrix/media/*/upload
	/// - /_matrix/media/*/preview_url
	/// - /_matrix/media/*/download/*
	/// - /_matrix/media/*/thumbnail/*
	///
	/// The authenticated equivalent endpoints are always enabled.
	///
	/// Defaults to true for now, but this is highly subject to change, likely
	/// in the next release.
	#[serde(default = "true_fn")]
	pub allow_legacy_media: bool,

	#[serde(default = "true_fn")]
	pub freeze_legacy_media: bool,

	/// Checks consistency of the media directory at startup:
	/// 1. When `media_compat_file_link` is enbled, this check will upgrade
	///    media when switching back and forth between Conduit and Conduwuit.
	///    Both options must be enabled to handle this.
	/// 2. When media is deleted from the directory, this check will also delete
	///    its database entry.
	///
	/// If none of these checks apply to your use cases, and your media
	/// directory is significantly large setting this to false may reduce
	/// startup time.
	///
	/// Enabled by default.
	#[serde(default = "true_fn")]
	pub media_startup_check: bool,

	/// Enable backward-compatibility with Conduit's media directory by creating
	/// symlinks of media. This option is only necessary if you plan on using
	/// Conduit again. Otherwise setting this to false reduces filesystem
	/// clutter and overhead for managing these symlinks in the directory. This
	/// is now disabled by default. You may still return to upstream Conduit
	/// but you have to run Conduwuit at least once with this set to true and
	/// allow the media_startup_check to take place before shutting
	/// down to return to Conduit.
	///
	/// Disabled by default.
	#[serde(default)]
	pub media_compat_file_link: bool,

	/// Prunes missing media from the database as part of the media startup
	/// checks. This means if you delete files from the media directory the
	/// corresponding entries will be removed from the database. This is
	/// disabled by default because if the media directory is accidentally moved
	/// or inaccessible the metadata entries in the database will be lost with
	/// sadness.
	///
	/// Disabled by default.
	#[serde(default)]
	pub prune_missing_media: bool,

	/// Vector list of servers that conduwuit will refuse to download remote
	/// media from. No default.
	#[serde(default = "Vec::new")]
	pub prevent_media_downloads_from: Vec<OwnedServerName>,

	/// List of forbidden server names that we will block incoming AND outgoing
	/// federation with, and block client room joins / remote user invites.
	///
	/// This check is applied on the room ID, room alias, sender server name,
	/// sender user's server name, inbound federation X-Matrix origin, and
	/// outbound federation handler.
	///
	/// Basically "global" ACLs. No default.
	#[serde(default = "Vec::new")]
	pub forbidden_remote_server_names: Vec<OwnedServerName>,

	/// List of forbidden server names that we will block all outgoing federated
	/// room directory requests for. Useful for preventing our users from
	/// wandering into bad servers or spaces. No default.
	#[serde(default = "Vec::new")]
	pub forbidden_remote_room_directory_server_names: Vec<OwnedServerName>,

	/// Vector list of IPv4 and IPv6 CIDR ranges / subnets *in quotes* that you
	/// do not want conduwuit to send outbound requests to. Defaults to
	/// RFC1918, unroutable, loopback, multicast, and testnet addresses for
	/// security.
	///
	/// To disable, set this to be an empty vector (`[]`).
	/// Please be aware that this is *not* a guarantee. You should be using a
	/// firewall with zones as doing this on the application layer may have
	/// bypasses.
	///
	/// Currently this does not account for proxies in use like Synapse does.
	#[serde(default = "default_ip_range_denylist")]
	pub ip_range_denylist: Vec<String>,

	/// Vector list of domains allowed to send requests to for URL previews.
	/// Defaults to none. Note: this is a *contains* match, not an explicit
	/// match. Putting "google.com" will match "https://google.com" and
	/// "http://mymaliciousdomainexamplegoogle.com" Setting this to "*" will
	/// allow all URL previews. Please note that this opens up significant
	/// attack surface to your server, you are expected to be aware of the
	/// risks by doing so.
	#[serde(default = "Vec::new")]
	pub url_preview_domain_contains_allowlist: Vec<String>,

	/// Vector list of explicit domains allowed to send requests to for URL
	/// previews. Defaults to none. Note: This is an *explicit* match, not a
	/// contains match. Putting "google.com" will match "https://google.com",
	/// "http://google.com", but not
	/// "https://mymaliciousdomainexamplegoogle.com". Setting this to "*" will
	/// allow all URL previews. Please note that this opens up significant
	/// attack surface to your server, you are expected to be aware of the
	/// risks by doing so.
	#[serde(default = "Vec::new")]
	pub url_preview_domain_explicit_allowlist: Vec<String>,

	/// Vector list of explicit domains not allowed to send requests to for URL
	/// previews. Defaults to none. Note: This is an *explicit* match, not a
	/// contains match. Putting "google.com" will match "https://google.com",
	/// "http://google.com", but not
	/// "https://mymaliciousdomainexamplegoogle.com". The denylist is checked
	/// first before allowlist. Setting this to "*" will not do anything.
	#[serde(default = "Vec::new")]
	pub url_preview_domain_explicit_denylist: Vec<String>,

	/// Vector list of URLs allowed to send requests to for URL previews.
	/// Defaults to none. Note that this is a *contains* match, not an
	/// explicit match. Putting "google.com" will match
	/// "https://google.com/",
	/// "https://google.com/url?q=https://mymaliciousdomainexample.com", and
	/// "https://mymaliciousdomainexample.com/hi/google.com" Setting this to
	/// "*" will allow all URL previews. Please note that this opens up
	/// significant attack surface to your server, you are expected to be
	/// aware of the risks by doing so.
	#[serde(default = "Vec::new")]
	pub url_preview_url_contains_allowlist: Vec<String>,

	/// Maximum amount of bytes allowed in a URL preview body size when
	/// spidering. Defaults to 384KB (384_000 bytes)
	#[serde(default = "default_url_preview_max_spider_size")]
	pub url_preview_max_spider_size: usize,

	/// Option to decide whether you would like to run the domain allowlist
	/// checks (contains and explicit) on the root domain or not. Does not apply
	/// to URL contains allowlist. Defaults to false. Example: If this is
	/// enabled and you have "wikipedia.org" allowed in the explicit and/or
	/// contains domain allowlist, it will allow all subdomains under
	/// "wikipedia.org" such as "en.m.wikipedia.org" as the root domain is
	/// checked and matched. Useful if the domain contains allowlist is still
	/// too broad for you but you still want to allow all the subdomains under a
	/// root domain.
	#[serde(default)]
	pub url_preview_check_root_domain: bool,

	/// List of forbidden room aliases and room IDs as patterns/strings. Values
	/// in this list are matched as *contains*. This is checked upon room alias
	/// creation, custom room ID creation if used, and startup as warnings if
	/// any room aliases in your database have a forbidden room alias/ID.
	/// No default.
	#[serde(default = "RegexSet::empty")]
	#[serde(with = "serde_regex")]
	pub forbidden_alias_names: RegexSet,

	/// List of forbidden username patterns/strings. Values in this list are
	/// matched as *contains*. This is checked upon username availability
	/// check, registration, and startup as warnings if any local users in your
	/// database have a forbidden username.
	/// No default.
	#[serde(default = "RegexSet::empty")]
	#[serde(with = "serde_regex")]
	pub forbidden_usernames: RegexSet,

	/// Retry failed and incomplete messages to remote servers immediately upon
	/// startup. This is called bursting. If this is disabled, said messages
	/// may not be delivered until more messages are queued for that server. Do
	/// not change this option unless server resources are extremely limited or
	/// the scale of the server's deployment is huge. Do not disable this
	/// unless you know what you are doing.
	#[serde(default = "true_fn")]
	pub startup_netburst: bool,

	/// messages are dropped and not reattempted. The `startup_netburst` option
	/// must be enabled for this value to have any effect. Do not change this
	/// value unless you know what you are doing. Set this value to -1 to
	/// reattempt every message without trimming the queues; this may consume
	/// significant disk. Set this value to 0 to drop all messages without any
	/// attempt at redelivery.
	#[serde(default = "default_startup_netburst_keep")]
	pub startup_netburst_keep: i64,

	/// controls whether non-admin local users are forbidden from sending room
	/// invites (local and remote), and if non-admin users can receive remote
	/// room invites. admins are always allowed to send and receive all room
	/// invites. defaults to false
	#[serde(default)]
	pub block_non_admin_invites: bool,

	/// Allows admins to enter commands in rooms other than #admins by prefixing
	/// with \!admin. The reply will be publicly visible to the room,
	/// originating from the sender. defaults to true
	#[serde(default = "true_fn")]
	pub admin_escape_commands: bool,

	/// Controls whether the conduwuit admin room console / CLI will immediately
	/// activate on startup. This option can also be enabled with `--console`
	/// conduwuit argument
	///
	/// Defaults to false
	#[serde(default)]
	pub admin_console_automatic: bool,

	/// Controls what admin commands will be executed on startup. This is a
	/// vector list of strings of admin commands to run.
	///
	/// An example of this can be: `admin_execute = ["debug ping puppygock.gay",
	/// "debug echo hi"]`
	///
	/// This option can also be configured with the `--execute` conduwuit
	/// argument and can take standard shell commands and environment variables
	///
	/// Such example could be: `./conduwuit --execute "server admin-notice
	/// conduwuit has started up at $(date)"`
	///
	/// Defaults to nothing.
	#[serde(default)]
	pub admin_execute: Vec<String>,

	/// Controls whether conduwuit should error and fail to start if an admin
	/// execute command (`--execute` / `admin_execute`) fails
	///
	/// Defaults to false
	#[serde(default)]
	pub admin_execute_errors_ignore: bool,

	/// Controls the max log level for admin command log captures (logs
	/// generated from running admin commands)
	///
	/// Defaults to "info" on release builds, else "debug" on debug builds
	#[serde(default = "default_admin_log_capture")]
	pub admin_log_capture: String,

	#[serde(default = "default_admin_room_tag")]
	pub admin_room_tag: String,

	/// Sentry.io crash/panic reporting, performance monitoring/metrics, etc.
	/// This is NOT enabled by default. conduwuit's default Sentry reporting
	/// endpoint is o4506996327251968.ingest.us.sentry.io
	///
	/// Defaults to *false*
	#[serde(default)]
	pub sentry: bool,

	/// Sentry reporting URL if a custom one is desired
	///
	/// Defaults to conduwuit's default Sentry endpoint:
	/// "https://fe2eb4536aa04949e28eff3128d64757@o4506996327251968.ingest.us.sentry.io/4506996334657536"
	#[serde(default = "default_sentry_endpoint")]
	pub sentry_endpoint: Option<Url>,

	/// Report your Conduwuit server_name in Sentry.io crash reports and metrics
	///
	/// Defaults to false
	#[serde(default)]
	pub sentry_send_server_name: bool,

	/// Performance monitoring/tracing sample rate for Sentry.io
	///
	/// Note that too high values may impact performance, and can be disabled by
	/// setting it to 0.0 (0%) This value is read as a percentage to Sentry,
	/// represented as a decimal
	///
	/// Defaults to 15% of traces (0.15)
	#[serde(default = "default_sentry_traces_sample_rate")]
	pub sentry_traces_sample_rate: f32,

	/// Whether to attach a stacktrace to Sentry reports.
	#[serde(default)]
	pub sentry_attach_stacktrace: bool,

	/// Send panics to sentry. This is true by default, but sentry has to be
	/// enabled.
	#[serde(default = "true_fn")]
	pub sentry_send_panic: bool,

	/// Send errors to sentry. This is true by default, but sentry has to be
	/// enabled. This option is only effective in release-mode; forced to false
	/// in debug-mode.
	#[serde(default = "true_fn")]
	pub sentry_send_error: bool,

	/// Controls the tracing log level for Sentry to send things like
	/// breadcrumbs and transactions Defaults to "info"
	#[serde(default = "default_sentry_filter")]
	pub sentry_filter: String,

	/// Enable the tokio-console. This option is only relevant to developers.
	/// See: docs/development.md#debugging-with-tokio-console for more
	/// information.
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
	pub fn load(paths: Option<&[PathBuf]>) -> Result<Figment> {
		let paths_files = paths.into_iter().flatten().map(Toml::file);

		let envs = [Env::var("CONDUIT_CONFIG"), Env::var("CONDUWUIT_CONFIG")];
		let envs_files = envs.into_iter().flatten().map(Toml::file);

		let config = envs_files
			.chain(paths_files)
			.fold(Figment::new(), |config, file| config.merge(file.nested()))
			.merge(Env::prefixed("CONDUIT_").global().split("__"))
			.merge(Env::prefixed("CONDUWUIT_").global().split("__"));

		Ok(config)
	}

	/// Finalize config
	pub fn new(raw_config: &Figment) -> Result<Self> {
		let config = raw_config
			.extract::<Self>()
			.map_err(|e| err!("There was a problem with your configuration file: {e}"))?;

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
			if self.registration_token.is_none() && self.registration_token_file.is_none() && self.allow_registration {
				"not set (⚠️ open registration!)"
			} else if self.registration_token.is_none() && self.registration_token_file.is_none() {
				"not set"
			} else {
				"set"
			},
		);
		line(
			"Registration token file path",
			self.registration_token_file
				.as_ref()
				.map_or("", |path| path.to_str().unwrap_or_default()),
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

fn default_federation_timeout() -> u64 { 25 }

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
