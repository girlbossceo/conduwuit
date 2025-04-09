pub mod check;
pub mod manager;
pub mod proxy;

use std::{
	collections::{BTreeMap, BTreeSet},
	net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
	path::{Path, PathBuf},
};

use conduwuit_macros::config_example_generator;
use either::{
	Either,
	Either::{Left, Right},
};
use figment::providers::{Env, Format, Toml};
pub use figment::{Figment, value::Value as FigmentValue};
use regex::RegexSet;
use ruma::{
	OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, RoomVersionId,
	api::client::discovery::discover_support::ContactRole,
};
use serde::{Deserialize, de::IgnoredAny};
use url::Url;

use self::proxy::ProxyConfig;
pub use self::{check::check, manager::Manager};
use crate::{Result, err, error::Error, utils::sys};

/// All the config options for conduwuit.
#[allow(clippy::struct_excessive_bools)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::bare_urls)]
#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(
	filename = "conduwuit-example.toml",
	section = "global",
	undocumented = "# This item is undocumented. Please contribute documentation for it.",
	header = r#"### conduwuit Configuration
###
### THIS FILE IS GENERATED. CHANGES/CONTRIBUTIONS IN THE REPO WILL BE
### OVERWRITTEN!
###
### You should rename this file before configuring your server. Changes to
### documentation and defaults can be contributed in source code at
### src/core/config/mod.rs. This file is generated when building.
###
### Any values pre-populated are the default values for said config option.
###
### At the minimum, you MUST edit all the config options to your environment
### that say "YOU NEED TO EDIT THIS".
###
### For more information, see:
### https://conduwuit.puppyirl.gay/configuration.html
"#,
	ignore = "catchall well_known tls blurhashing allow_invalid_tls_certificates_yes_i_know_what_the_fuck_i_am_doing_with_this_and_i_know_this_is_insecure"
)]
pub struct Config {
	/// The server_name is the pretty name of this server. It is used as a
	/// suffix for user and room IDs/aliases.
	///
	/// See the docs for reverse proxying and delegation:
	/// https://conduwuit.puppyirl.gay/deploying/generic.html#setting-up-the-reverse-proxy
	///
	/// Also see the `[global.well_known]` config section at the very bottom.
	///
	/// Examples of delegation:
	/// - https://puppygock.gay/.well-known/matrix/server
	/// - https://puppygock.gay/.well-known/matrix/client
	///
	/// YOU NEED TO EDIT THIS. THIS CANNOT BE CHANGED AFTER WITHOUT A DATABASE
	/// WIPE.
	///
	/// example: "conduwuit.woof"
	pub server_name: OwnedServerName,

	/// The default address (IPv4 or IPv6) conduwuit will listen on.
	///
	/// If you are using Docker or a container NAT networking setup, this must
	/// be "0.0.0.0".
	///
	/// To listen on multiple addresses, specify a vector e.g. ["127.0.0.1",
	/// "::1"]
	///
	/// default: ["127.0.0.1", "::1"]
	#[serde(default = "default_address")]
	address: ListeningAddr,

	/// The port(s) conduwuit will listen on.
	///
	/// For reverse proxying, see:
	/// https://conduwuit.puppyirl.gay/deploying/generic.html#setting-up-the-reverse-proxy
	///
	/// If you are using Docker, don't change this, you'll need to map an
	/// external port to this.
	///
	/// To listen on multiple ports, specify a vector e.g. [8080, 8448]
	///
	/// default: 8008
	#[serde(default = "default_port")]
	port: ListeningPort,

	// external structure; separate section
	#[serde(default)]
	pub tls: TlsConfig,

	/// The UNIX socket conduwuit will listen on.
	///
	/// conduwuit cannot listen on both an IP address and a UNIX socket. If
	/// listening on a UNIX socket, you MUST remove/comment the `address` key.
	///
	/// Remember to make sure that your reverse proxy has access to this socket
	/// file, either by adding your reverse proxy to the 'conduwuit' group or
	/// granting world R/W permissions with `unix_socket_perms` (666 minimum).
	///
	/// example: "/run/conduwuit/conduwuit.sock"
	pub unix_socket_path: Option<PathBuf>,

	/// The default permissions (in octal) to create the UNIX socket with.
	///
	/// default: 660
	#[serde(default = "default_unix_socket_perms")]
	pub unix_socket_perms: u32,

	/// This is the only directory where conduwuit will save its data, including
	/// media. Note: this was previously "/var/lib/matrix-conduit".
	///
	/// YOU NEED TO EDIT THIS.
	///
	/// example: "/var/lib/conduwuit"
	pub database_path: PathBuf,

	/// conduwuit supports online database backups using RocksDB's Backup engine
	/// API. To use this, set a database backup path that conduwuit can write
	/// to.
	///
	/// For more information, see:
	/// https://conduwuit.puppyirl.gay/maintenance.html#backups
	///
	/// example: "/opt/conduwuit-db-backups"
	pub database_backup_path: Option<PathBuf>,

	/// The amount of online RocksDB database backups to keep/retain, if using
	/// "database_backup_path", before deleting the oldest one.
	///
	/// default: 1
	#[serde(default = "default_database_backups_to_keep")]
	pub database_backups_to_keep: i16,

	/// Text which will be added to the end of the user's displayname upon
	/// registration with a space before the text. In Conduit, this was the
	/// lightning bolt emoji.
	///
	/// To disable, set this to "" (an empty string).
	///
	/// The default is the trans pride flag.
	///
	/// example: "🏳️‍⚧️"
	///
	/// default: "🏳️‍⚧️"
	#[serde(default = "default_new_user_displayname_suffix")]
	pub new_user_displayname_suffix: String,

	/// If enabled, conduwuit will send a simple GET request periodically to
	/// `https://pupbrain.dev/check-for-updates/stable` for any new
	/// announcements made. Despite the name, this is not an update check
	/// endpoint, it is simply an announcement check endpoint.
	///
	/// This is disabled by default as this is rarely used except for security
	/// updates or major updates.
	#[serde(default, alias = "allow_announcements_check")]
	pub allow_check_for_updates: bool,

	/// Set this to any float value to multiply conduwuit's in-memory LRU caches
	/// with such as "auth_chain_cache_capacity".
	///
	/// May be useful if you have significant memory to spare to increase
	/// performance.
	///
	/// If you have low memory, reducing this may be viable.
	///
	/// By default, the individual caches such as "auth_chain_cache_capacity"
	/// are scaled by your CPU core count.
	///
	/// default: 1.0
	#[serde(
		default = "default_cache_capacity_modifier",
		alias = "conduit_cache_capacity_modifier"
	)]
	pub cache_capacity_modifier: f64,

	/// Set this to any float value in megabytes for conduwuit to tell the
	/// database engine that this much memory is available for database read
	/// caches.
	///
	/// May be useful if you have significant memory to spare to increase
	/// performance.
	///
	/// Similar to the individual LRU caches, this is scaled up with your CPU
	/// core count.
	///
	/// This defaults to 128.0 + (64.0 * CPU core count).
	///
	/// default: varies by system
	#[serde(default = "default_db_cache_capacity_mb")]
	pub db_cache_capacity_mb: f64,

	/// Set this to any float value in megabytes for conduwuit to tell the
	/// database engine that this much memory is available for database write
	/// caches.
	///
	/// May be useful if you have significant memory to spare to increase
	/// performance.
	///
	/// Similar to the individual LRU caches, this is scaled up with your CPU
	/// core count.
	///
	/// This defaults to 48.0 + (4.0 * CPU core count).
	///
	/// default: varies by system
	#[serde(default = "default_db_write_buffer_capacity_mb")]
	pub db_write_buffer_capacity_mb: f64,

	/// default: varies by system
	#[serde(default = "default_pdu_cache_capacity")]
	pub pdu_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_auth_chain_cache_capacity")]
	pub auth_chain_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_shorteventid_cache_capacity")]
	pub shorteventid_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_eventidshort_cache_capacity")]
	pub eventidshort_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_eventid_pdu_cache_capacity")]
	pub eventid_pdu_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_shortstatekey_cache_capacity")]
	pub shortstatekey_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_statekeyshort_cache_capacity")]
	pub statekeyshort_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_servernameevent_data_cache_capacity")]
	pub servernameevent_data_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_stateinfo_cache_capacity")]
	pub stateinfo_cache_capacity: u32,

	/// default: varies by system
	#[serde(default = "default_roomid_spacehierarchy_cache_capacity")]
	pub roomid_spacehierarchy_cache_capacity: u32,

	/// Maximum entries stored in DNS memory-cache. The size of an entry may
	/// vary so please take care if raising this value excessively. Only
	/// decrease this when using an external DNS cache. Please note that
	/// systemd-resolved does *not* count as an external cache, even when
	/// configured to do so.
	///
	/// default: 32768
	#[serde(default = "default_dns_cache_entries")]
	pub dns_cache_entries: u32,

	/// Minimum time-to-live in seconds for entries in the DNS cache. The
	/// default may appear high to most administrators; this is by design as the
	/// majority of NXDOMAINs are correct for a long time (e.g. the server is no
	/// longer running Matrix). Only decrease this if you are using an external
	/// DNS cache.
	///
	/// default: 10800
	#[serde(default = "default_dns_min_ttl")]
	pub dns_min_ttl: u64,

	/// Minimum time-to-live in seconds for NXDOMAIN entries in the DNS cache.
	/// This value is critical for the server to federate efficiently.
	/// NXDOMAIN's are assumed to not be returning to the federation and
	/// aggressively cached rather than constantly rechecked.
	///
	/// Defaults to 3 days as these are *very rarely* false negatives.
	///
	/// default: 259200
	#[serde(default = "default_dns_min_ttl_nxdomain")]
	pub dns_min_ttl_nxdomain: u64,

	/// Number of DNS nameserver retries after a timeout or error.
	///
	/// default: 10
	#[serde(default = "default_dns_attempts")]
	pub dns_attempts: u16,

	/// The number of seconds to wait for a reply to a DNS query. Please note
	/// that recursive queries can take up to several seconds for some domains,
	/// so this value should not be too low, especially on slower hardware or
	/// resolvers.
	///
	/// default: 10
	#[serde(default = "default_dns_timeout")]
	pub dns_timeout: u64,

	/// Fallback to TCP on DNS errors. Set this to false if unsupported by
	/// nameserver.
	#[serde(default = "true_fn")]
	pub dns_tcp_fallback: bool,

	/// Enable to query all nameservers until the domain is found. Referred to
	/// as "trust_negative_responses" in hickory_resolver. This can avoid
	/// useless DNS queries if the first nameserver responds with NXDOMAIN or
	/// an empty NOERROR response.
	#[serde(default = "true_fn")]
	pub query_all_nameservers: bool,

	/// Enable using *only* TCP for querying your specified nameservers instead
	/// of UDP.
	///
	/// If you are running conduwuit in a container environment, this config
	/// option may need to be enabled. For more details, see:
	/// https://conduwuit.puppyirl.gay/troubleshooting.html#potential-dns-issues-when-using-docker
	#[serde(default)]
	pub query_over_tcp_only: bool,

	/// DNS A/AAAA record lookup strategy
	///
	/// Takes a number of one of the following options:
	/// 1 - Ipv4Only (Only query for A records, no AAAA/IPv6)
	///
	/// 2 - Ipv6Only (Only query for AAAA records, no A/IPv4)
	///
	/// 3 - Ipv4AndIpv6 (Query for A and AAAA records in parallel, uses whatever
	/// returns a successful response first)
	///
	/// 4 - Ipv6thenIpv4 (Query for AAAA record, if that fails then query the A
	/// record)
	///
	/// 5 - Ipv4thenIpv6 (Query for A record, if that fails then query the AAAA
	/// record)
	///
	/// If you don't have IPv6 networking, then for better DNS performance it
	/// may be suitable to set this to Ipv4Only (1) as you will never ever use
	/// the AAAA record contents even if the AAAA record is successful instead
	/// of the A record.
	///
	/// default: 5
	#[serde(default = "default_ip_lookup_strategy")]
	pub ip_lookup_strategy: u8,

	/// Max request size for file uploads in bytes. Defaults to 20MB.
	///
	/// default: 20971520
	#[serde(default = "default_max_request_size")]
	pub max_request_size: usize,

	/// default: 192
	#[serde(default = "default_max_fetch_prev_events")]
	pub max_fetch_prev_events: u16,

	/// Default/base connection timeout (seconds). This is used only by URL
	/// previews and update/news endpoint checks.
	///
	/// default: 10
	#[serde(default = "default_request_conn_timeout")]
	pub request_conn_timeout: u64,

	/// Default/base request timeout (seconds). The time waiting to receive more
	/// data from another server. This is used only by URL previews,
	/// update/news, and misc endpoint checks.
	///
	/// default: 35
	#[serde(default = "default_request_timeout")]
	pub request_timeout: u64,

	/// Default/base request total timeout (seconds). The time limit for a whole
	/// request. This is set very high to not cancel healthy requests while
	/// serving as a backstop. This is used only by URL previews and update/news
	/// endpoint checks.
	///
	/// default: 320
	#[serde(default = "default_request_total_timeout")]
	pub request_total_timeout: u64,

	/// Default/base idle connection pool timeout (seconds). This is used only
	/// by URL previews and update/news endpoint checks.
	///
	/// default: 5
	#[serde(default = "default_request_idle_timeout")]
	pub request_idle_timeout: u64,

	/// Default/base max idle connections per host. This is used only by URL
	/// previews and update/news endpoint checks. Defaults to 1 as generally the
	/// same open connection can be re-used.
	///
	/// default: 1
	#[serde(default = "default_request_idle_per_host")]
	pub request_idle_per_host: u16,

	/// Federation well-known resolution connection timeout (seconds).
	///
	/// default: 6
	#[serde(default = "default_well_known_conn_timeout")]
	pub well_known_conn_timeout: u64,

	/// Federation HTTP well-known resolution request timeout (seconds).
	///
	/// default: 10
	#[serde(default = "default_well_known_timeout")]
	pub well_known_timeout: u64,

	/// Federation client request timeout (seconds). You most definitely want
	/// this to be high to account for extremely large room joins, slow
	/// homeservers, your own resources etc.
	///
	/// default: 300
	#[serde(default = "default_federation_timeout")]
	pub federation_timeout: u64,

	/// Federation client idle connection pool timeout (seconds).
	///
	/// default: 25
	#[serde(default = "default_federation_idle_timeout")]
	pub federation_idle_timeout: u64,

	/// Federation client max idle connections per host. Defaults to 1 as
	/// generally the same open connection can be re-used.
	///
	/// default: 1
	#[serde(default = "default_federation_idle_per_host")]
	pub federation_idle_per_host: u16,

	/// Federation sender request timeout (seconds). The time it takes for the
	/// remote server to process sent transactions can take a while.
	///
	/// default: 180
	#[serde(default = "default_sender_timeout")]
	pub sender_timeout: u64,

	/// Federation sender idle connection pool timeout (seconds).
	///
	/// default: 180
	#[serde(default = "default_sender_idle_timeout")]
	pub sender_idle_timeout: u64,

	/// Federation sender transaction retry backoff limit (seconds).
	///
	/// default: 86400
	#[serde(default = "default_sender_retry_backoff_limit")]
	pub sender_retry_backoff_limit: u64,

	/// Appservice URL request connection timeout. Defaults to 35 seconds as
	/// generally appservices are hosted within the same network.
	///
	/// default: 35
	#[serde(default = "default_appservice_timeout")]
	pub appservice_timeout: u64,

	/// Appservice URL idle connection pool timeout (seconds).
	///
	/// default: 300
	#[serde(default = "default_appservice_idle_timeout")]
	pub appservice_idle_timeout: u64,

	/// Notification gateway pusher idle connection pool timeout.
	///
	/// default: 15
	#[serde(default = "default_pusher_idle_timeout")]
	pub pusher_idle_timeout: u64,

	/// Maximum time to receive a request from a client (seconds).
	///
	/// default: 75
	#[serde(default = "default_client_receive_timeout")]
	pub client_receive_timeout: u64,

	/// Maximum time to process a request received from a client (seconds).
	///
	/// default: 180
	#[serde(default = "default_client_request_timeout")]
	pub client_request_timeout: u64,

	/// Maximum time to transmit a response to a client (seconds)
	///
	/// default: 120
	#[serde(default = "default_client_response_timeout")]
	pub client_response_timeout: u64,

	/// Grace period for clean shutdown of client requests (seconds).
	///
	/// default: 10
	#[serde(default = "default_client_shutdown_timeout")]
	pub client_shutdown_timeout: u64,

	/// Grace period for clean shutdown of federation requests (seconds).
	///
	/// default: 5
	#[serde(default = "default_sender_shutdown_timeout")]
	pub sender_shutdown_timeout: u64,

	/// Enables registration. If set to false, no users can register on this
	/// server.
	///
	/// If set to true without a token configured, users can register with no
	/// form of 2nd-step only if you set the following option to true:
	/// `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`
	///
	/// If you would like registration only via token reg, please configure
	/// `registration_token` or `registration_token_file`.
	#[serde(default)]
	pub allow_registration: bool,

	/// Enabling this setting opens registration to anyone without restrictions.
	/// This makes your server vulnerable to abuse
	#[serde(default)]
	pub yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse: bool,

	/// A static registration token that new users will have to provide when
	/// creating an account. If unset and `allow_registration` is true,
	/// you must set
	/// `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`
	/// to true to allow open registration without any conditions.
	///
	/// YOU NEED TO EDIT THIS OR USE registration_token_file.
	///
	/// example: "o&^uCtes4HPf0Vu@F20jQeeWE7"
	///
	/// display: sensitive
	pub registration_token: Option<String>,

	/// Path to a file on the system that gets read for additional registration
	/// tokens. Multiple tokens can be added if you separate them with
	/// whitespace
	///
	/// conduwuit must be able to access the file, and it must not be empty
	///
	/// example: "/etc/conduwuit/.reg_token"
	pub registration_token_file: Option<PathBuf>,

	/// Controls whether encrypted rooms and events are allowed.
	#[serde(default = "true_fn")]
	pub allow_encryption: bool,

	/// Controls whether federation is allowed or not. It is not recommended to
	/// disable this after the fact due to potential federation breakage.
	#[serde(default = "true_fn")]
	pub allow_federation: bool,

	/// Allows federation requests to be made to itself
	///
	/// This isn't intended and is very likely a bug if federation requests are
	/// being sent to yourself. This currently mainly exists for development
	/// purposes.
	#[serde(default)]
	pub federation_loopback: bool,

	/// Always calls /forget on behalf of the user if leaving a room. This is a
	/// part of MSC4267 "Automatically forgetting rooms on leave"
	#[serde(default)]
	pub forget_forced_upon_leave: bool,

	/// Set this to true to require authentication on the normally
	/// unauthenticated profile retrieval endpoints (GET)
	/// "/_matrix/client/v3/profile/{userId}".
	///
	/// This can prevent profile scraping.
	#[serde(default)]
	pub require_auth_for_profile_requests: bool,

	/// Set this to true to allow your server's public room directory to be
	/// federated. Set this to false to protect against /publicRooms spiders,
	/// but will forbid external users from viewing your server's public room
	/// directory. If federation is disabled entirely (`allow_federation`), this
	/// is inherently false.
	#[serde(default)]
	pub allow_public_room_directory_over_federation: bool,

	/// Set this to true to allow your server's public room directory to be
	/// queried without client authentication (access token) through the Client
	/// APIs. Set this to false to protect against /publicRooms spiders.
	#[serde(default)]
	pub allow_public_room_directory_without_auth: bool,

	/// Allow guests/unauthenticated users to access TURN credentials.
	///
	/// This is the equivalent of Synapse's `turn_allow_guests` config option.
	/// This allows any unauthenticated user to call the endpoint
	/// `/_matrix/client/v3/voip/turnServer`.
	///
	/// It is unlikely you need to enable this as all major clients support
	/// authentication for this endpoint and prevents misuse of your TURN server
	/// from potential bots.
	#[serde(default)]
	pub turn_allow_guests: bool,

	/// Set this to true to lock down your server's public room directory and
	/// only allow admins to publish rooms to the room directory. Unpublishing
	/// is still allowed by all users with this enabled.
	#[serde(default)]
	pub lockdown_public_room_directory: bool,

	/// Set this to true to allow federating device display names / allow
	/// external users to see your device display name. If federation is
	/// disabled entirely (`allow_federation`), this is inherently false. For
	/// privacy reasons, this is best left disabled.
	#[serde(default)]
	pub allow_device_name_federation: bool,

	/// Config option to allow or disallow incoming federation requests that
	/// obtain the profiles of our local users from
	/// `/_matrix/federation/v1/query/profile`
	///
	/// Increases privacy of your local user's such as display names, but some
	/// remote users may get a false "this user does not exist" error when they
	/// try to invite you to a DM or room. Also can protect against profile
	/// spiders.
	///
	/// This is inherently false if `allow_federation` is disabled
	#[serde(default = "true_fn", alias = "allow_profile_lookup_federation_requests")]
	pub allow_inbound_profile_lookup_federation_requests: bool,

	/// Allow standard users to create rooms. Appservices and admins are always
	/// allowed to create rooms
	#[serde(default = "true_fn")]
	pub allow_room_creation: bool,

	/// Set to false to disable users from joining or creating room versions
	/// that aren't officially supported by conduwuit.
	///
	/// conduwuit officially supports room versions 6 - 11.
	///
	/// conduwuit has slightly experimental (though works fine in practice)
	/// support for versions 3 - 5.
	#[serde(default = "true_fn")]
	pub allow_unstable_room_versions: bool,

	/// Default room version conduwuit will create rooms with.
	///
	/// Per spec, room version 11 is the default.
	///
	/// default: 11
	#[serde(default = "default_default_room_version")]
	pub default_room_version: RoomVersionId,

	// external structure; separate section
	#[serde(default)]
	pub well_known: WellKnownConfig,

	#[serde(default)]
	pub allow_jaeger: bool,

	/// default: "info"
	#[serde(default = "default_jaeger_filter")]
	pub jaeger_filter: String,

	/// If the 'perf_measurements' compile-time feature is enabled, enables
	/// collecting folded stack trace profile of tracing spans using
	/// tracing_flame. The resulting profile can be visualized with inferno[1],
	/// speedscope[2], or a number of other tools.
	///
	/// [1]: https://github.com/jonhoo/inferno
	/// [2]: www.speedscope.app
	#[serde(default)]
	pub tracing_flame: bool,

	/// default: "info"
	#[serde(default = "default_tracing_flame_filter")]
	pub tracing_flame_filter: String,

	/// default: "./tracing.folded"
	#[serde(default = "default_tracing_flame_output_path")]
	pub tracing_flame_output_path: String,

	#[cfg(not(doctest))]
	/// Examples:
	///
	/// - No proxy (default):
	///
	///       proxy = "none"
	///
	/// - For global proxy, create the section at the bottom of this file:
	///
	///       [global.proxy]
	///       global = { url = "socks5h://localhost:9050" }
	///
	/// - To proxy some domains:
	///
	///       [global.proxy]
	///       [[global.proxy.by_domain]]
	///       url = "socks5h://localhost:9050"
	///       include = ["*.onion", "matrix.myspecial.onion"]
	///       exclude = ["*.myspecial.onion"]
	///
	/// Include vs. Exclude:
	///
	/// - If include is an empty list, it is assumed to be `["*"]`.
	///
	/// - If a domain matches both the exclude and include list, the proxy will
	///   only be used if it was included because of a more specific rule than
	///   it was excluded. In the above example, the proxy would be used for
	///   `ordinary.onion`, `matrix.myspecial.onion`, but not
	///   `hello.myspecial.onion`.
	///
	/// default: "none"
	#[serde(default)]
	pub proxy: ProxyConfig,

	/// Servers listed here will be used to gather public keys of other servers
	/// (notary trusted key servers).
	///
	/// Currently, conduwuit doesn't support inbound batched key requests, so
	/// this list should only contain other Synapse servers.
	///
	/// example: ["matrix.org", "tchncs.de"]
	///
	/// default: ["matrix.org"]
	#[serde(default = "default_trusted_servers")]
	pub trusted_servers: Vec<OwnedServerName>,

	/// Whether to query the servers listed in trusted_servers first or query
	/// the origin server first. For best security, querying the origin server
	/// first is advised to minimize the exposure to a compromised trusted
	/// server. For maximum federation/join performance this can be set to true,
	/// however other options exist to query trusted servers first under
	/// specific high-load circumstances and should be evaluated before setting
	/// this to true.
	#[serde(default)]
	pub query_trusted_key_servers_first: bool,

	/// Whether to query the servers listed in trusted_servers first
	/// specifically on room joins. This option limits the exposure to a
	/// compromised trusted server to room joins only. The join operation
	/// requires gathering keys from many origin servers which can cause
	/// significant delays. Therefor this defaults to true to mitigate
	/// unexpected delays out-of-the-box. The security-paranoid or those willing
	/// to tolerate delays are advised to set this to false. Note that setting
	/// query_trusted_key_servers_first to true causes this option to be
	/// ignored.
	#[serde(default = "true_fn")]
	pub query_trusted_key_servers_first_on_join: bool,

	/// Only query trusted servers for keys and never the origin server. This is
	/// intended for clusters or custom deployments using their trusted_servers
	/// as forwarding-agents to cache and deduplicate requests. Notary servers
	/// do not act as forwarding-agents by default, therefor do not enable this
	/// unless you know exactly what you are doing.
	#[serde(default)]
	pub only_query_trusted_key_servers: bool,

	/// Maximum number of keys to request in each trusted server batch query.
	///
	/// default: 1024
	#[serde(default = "default_trusted_server_batch_size")]
	pub trusted_server_batch_size: usize,

	/// Max log level for conduwuit. Allows debug, info, warn, or error.
	///
	/// See also:
	/// https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives
	///
	/// **Caveat**:
	/// For release builds, the tracing crate is configured to only implement
	/// levels higher than error to avoid unnecessary overhead in the compiled
	/// binary from trace macros. For debug builds, this restriction is not
	/// applied.
	///
	/// default: "info"
	#[serde(default = "default_log")]
	pub log: String,

	/// Output logs with ANSI colours.
	#[serde(default = "true_fn", alias = "log_colours")]
	pub log_colors: bool,

	/// Configures the span events which will be outputted with the log.
	///
	/// default: "none"
	#[serde(default = "default_log_span_events")]
	pub log_span_events: String,

	/// Configures whether CONDUWUIT_LOG EnvFilter matches values using regular
	/// expressions. See the tracing_subscriber documentation on Directives.
	///
	/// default: true
	#[serde(default = "true_fn")]
	pub log_filter_regex: bool,

	/// Toggles the display of ThreadId in tracing log output.
	///
	/// default: false
	#[serde(default)]
	pub log_thread_ids: bool,

	/// OpenID token expiration/TTL in seconds.
	///
	/// These are the OpenID tokens that are primarily used for Matrix account
	/// integrations (e.g. Vector Integrations in Element), *not* OIDC/OpenID
	/// Connect/etc.
	///
	/// default: 3600
	#[serde(default = "default_openid_token_ttl")]
	pub openid_token_ttl: u64,

	/// Allow an existing session to mint a login token for another client.
	/// This requires interactive authentication, but has security ramifications
	/// as a malicious client could use the mechanism to spawn more than one
	/// session.
	/// Enabled by default.
	#[serde(default = "true_fn")]
	pub login_via_existing_session: bool,

	/// Login token expiration/TTL in milliseconds.
	///
	/// These are short-lived tokens for the m.login.token endpoint.
	/// This is used to allow existing sessions to create new sessions.
	/// see login_via_existing_session.
	///
	/// default: 120000
	#[serde(default = "default_login_token_ttl")]
	pub login_token_ttl: u64,

	/// Static TURN username to provide the client if not using a shared secret
	/// ("turn_secret"), It is recommended to use a shared secret over static
	/// credentials.
	#[serde(default)]
	pub turn_username: String,

	/// Static TURN password to provide the client if not using a shared secret
	/// ("turn_secret"). It is recommended to use a shared secret over static
	/// credentials.
	///
	/// display: sensitive
	#[serde(default)]
	pub turn_password: String,

	/// Vector list of TURN URIs/servers to use.
	///
	/// Replace "example.turn.uri" with your TURN domain, such as the coturn
	/// "realm" config option. If using TURN over TLS, replace the URI prefix
	/// "turn:" with "turns:".
	///
	/// example: ["turn:example.turn.uri?transport=udp",
	/// "turn:example.turn.uri?transport=tcp"]
	///
	/// default: []
	#[serde(default)]
	pub turn_uris: Vec<String>,

	/// TURN secret to use for generating the HMAC-SHA1 hash apart of username
	/// and password generation.
	///
	/// This is more secure, but if needed you can use traditional static
	/// username/password credentials.
	///
	/// display: sensitive
	#[serde(default)]
	pub turn_secret: String,

	/// TURN secret to use that's read from the file path specified.
	///
	/// This takes priority over "turn_secret" first, and falls back to
	/// "turn_secret" if invalid or failed to open.
	///
	/// example: "/etc/conduwuit/.turn_secret"
	pub turn_secret_file: Option<PathBuf>,

	/// TURN TTL, in seconds.
	///
	/// default: 86400
	#[serde(default = "default_turn_ttl")]
	pub turn_ttl: u64,

	/// List/vector of room IDs or room aliases that conduwuit will make newly
	/// registered users join. The rooms specified must be rooms that you have
	/// joined at least once on the server, and must be public.
	///
	/// example: ["#conduwuit:puppygock.gay",
	/// "!eoIzvAvVwY23LPDay8:puppygock.gay"]
	///
	/// default: []
	#[serde(default = "Vec::new")]
	pub auto_join_rooms: Vec<OwnedRoomOrAliasId>,

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
	/// reasons and this performs a full user deactivation.
	#[serde(default)]
	pub auto_deactivate_banned_room_attempts: bool,

	/// RocksDB log level. This is not the same as conduwuit's log level. This
	/// is the log level for the RocksDB engine/library which show up in your
	/// database folder/path as `LOG` files. conduwuit will log RocksDB errors
	/// as normal through tracing or panics if severe for safety.
	///
	/// default: "error"
	#[serde(default = "default_rocksdb_log_level")]
	pub rocksdb_log_level: String,

	#[serde(default)]
	pub rocksdb_log_stderr: bool,

	/// Max RocksDB `LOG` file size before rotating in bytes. Defaults to 4MB in
	/// bytes.
	///
	/// default: 4194304
	#[serde(default = "default_rocksdb_max_log_file_size")]
	pub rocksdb_max_log_file_size: usize,

	/// Time in seconds before RocksDB will forcibly rotate logs.
	///
	/// default: 0
	#[serde(default = "default_rocksdb_log_time_to_roll")]
	pub rocksdb_log_time_to_roll: usize,

	/// Set this to true to use RocksDB config options that are tailored to HDDs
	/// (slower device storage).
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
	/// your DB issues.
	///
	/// For more information, see:
	/// https://github.com/facebook/rocksdb/wiki/Direct-IO
	#[serde(default)]
	pub rocksdb_optimize_for_spinning_disks: bool,

	/// Enables direct-io to increase database performance via unbuffered I/O.
	///
	/// For more details about direct I/O and RockDB, see:
	/// https://github.com/facebook/rocksdb/wiki/Direct-IO
	///
	/// Set this option to false if the database resides on a filesystem which
	/// does not support direct-io like FUSE, or any form of complex filesystem
	/// setup such as possibly ZFS.
	#[serde(default = "true_fn")]
	pub rocksdb_direct_io: bool,

	/// Amount of threads that RocksDB will use for parallelism on database
	/// operations such as cleanup, sync, flush, compaction, etc. Set to 0 to
	/// use all your logical threads. Defaults to your CPU logical thread count.
	///
	/// default: varies by system
	#[serde(default = "default_rocksdb_parallelism_threads")]
	pub rocksdb_parallelism_threads: usize,

	/// Maximum number of LOG files RocksDB will keep. This must *not* be set to
	/// 0. It must be at least 1. Defaults to 3 as these are not very useful
	/// unless troubleshooting/debugging a RocksDB bug.
	///
	/// default: 3
	#[serde(default = "default_rocksdb_max_log_files")]
	pub rocksdb_max_log_files: usize,

	/// Type of RocksDB database compression to use.
	///
	/// Available options are "zstd", "bz2", "lz4", or "none".
	///
	/// It is best to use ZSTD as an overall good balance between
	/// speed/performance, storage, IO amplification, and CPU usage. For more
	/// performance but less compression (more storage used) and less CPU usage,
	/// use LZ4.
	///
	/// For more details, see:
	/// https://github.com/facebook/rocksdb/wiki/Compression
	///
	/// "none" will disable compression.
	///
	/// default: "zstd"
	#[serde(default = "default_rocksdb_compression_algo")]
	pub rocksdb_compression_algo: String,

	/// Level of compression the specified compression algorithm for RocksDB to
	/// use.
	///
	/// Default is 32767, which is internally read by RocksDB as the default
	/// magic number and translated to the library's default compression level
	/// as they all differ. See their `kDefaultCompressionLevel`.
	///
	/// Note when using the default value we may override it with a setting
	/// tailored specifically conduwuit.
	///
	/// default: 32767
	#[serde(default = "default_rocksdb_compression_level")]
	pub rocksdb_compression_level: i32,

	/// Level of compression the specified compression algorithm for the
	/// bottommost level/data for RocksDB to use. Default is 32767, which is
	/// internally read by RocksDB as the default magic number and translated to
	/// the library's default compression level as they all differ. See their
	/// `kDefaultCompressionLevel`.
	///
	/// Since this is the bottommost level (generally old and least used data),
	/// it may be desirable to have a very high compression level here as it's
	/// less likely for this data to be used. Research your chosen compression
	/// algorithm.
	///
	/// Note when using the default value we may override it with a setting
	/// tailored specifically conduwuit.
	///
	/// default: 32767
	#[serde(default = "default_rocksdb_bottommost_compression_level")]
	pub rocksdb_bottommost_compression_level: i32,

	/// Whether to enable RocksDB's "bottommost_compression".
	///
	/// At the expense of more CPU usage, this will further compress the
	/// database to reduce more storage. It is recommended to use ZSTD
	/// compression with this for best compression results. This may be useful
	/// if you're trying to reduce storage usage from the database.
	///
	/// See https://github.com/facebook/rocksdb/wiki/Compression for more details.
	#[serde(default = "true_fn")]
	pub rocksdb_bottommost_compression: bool,

	/// Database recovery mode (for RocksDB WAL corruption).
	///
	/// Use this option when the server reports corruption and refuses to start.
	/// Set mode 2 (PointInTime) to cleanly recover from this corruption. The
	/// server will continue from the last good state, several seconds or
	/// minutes prior to the crash. Clients may have to run "clear-cache &
	/// reload" to account for the rollback. Upon success, you may reset the
	/// mode back to default and restart again. Please note in some cases the
	/// corruption error may not be cleared for at least 30 minutes of operation
	/// in PointInTime mode.
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
	/// For more information on these modes, see:
	/// https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes
	///
	/// For more details on recovering a corrupt database, see:
	/// https://conduwuit.puppyirl.gay/troubleshooting.html#database-corruption
	///
	/// default: 1
	#[serde(default = "default_rocksdb_recovery_mode")]
	pub rocksdb_recovery_mode: u8,

	/// Enables or disables paranoid SST file checks. This can improve RocksDB
	/// database consistency at a potential performance impact due to further
	/// safety checks ran.
	///
	/// For more information, see:
	/// https://github.com/facebook/rocksdb/wiki/Online-Verification#columnfamilyoptionsparanoid_file_checks
	#[serde(default)]
	pub rocksdb_paranoid_file_checks: bool,

	/// Enables or disables checksum verification in rocksdb at runtime.
	/// Checksums are usually hardware accelerated with low overhead; they are
	/// enabled in rocksdb by default. Older or slower platforms may see gains
	/// from disabling.
	///
	/// default: true
	#[serde(default = "true_fn")]
	pub rocksdb_checksums: bool,

	/// Enables the "atomic flush" mode in rocksdb. This option is not intended
	/// for users. It may be removed or ignored in future versions. Atomic flush
	/// may be enabled by the paranoid to possibly improve database integrity at
	/// the cost of performance.
	#[serde(default)]
	pub rocksdb_atomic_flush: bool,

	/// Database repair mode (for RocksDB SST corruption).
	///
	/// Use this option when the server reports corruption while running or
	/// panics. If the server refuses to start use the recovery mode options
	/// first. Corruption errors containing the acronym 'SST' which occur after
	/// startup will likely require this option.
	///
	/// - Backing up your database directory is recommended prior to running the
	///   repair.
	///
	/// - Disabling repair mode and restarting the server is recommended after
	///   running the repair.
	///
	/// See https://conduwuit.puppyirl.gay/troubleshooting.html#database-corruption for more details on recovering a corrupt database.
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

	/// Enables RocksDB compaction. You should never ever have to set this
	/// option to false. If you for some reason find yourself needing to use this
	/// option as part of troubleshooting or a bug, please reach out to us in
	/// the conduwuit Matrix room with information and details.
	///
	/// Disabling compaction will lead to a significantly bloated and
	/// explosively large database, gradually poor performance, unnecessarily
	/// excessive disk read/writes, and slower shutdowns and startups.
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
	/// default: 1
	#[serde(default = "default_rocksdb_stats_level")]
	pub rocksdb_stats_level: u8,

	/// This is a password that can be configured that will let you login to the
	/// server bot account (currently `@conduit`) for emergency troubleshooting
	/// purposes such as recovering/recreating your admin room, or inviting
	/// yourself back.
	///
	/// See https://conduwuit.puppyirl.gay/troubleshooting.html#lost-access-to-admin-room for other ways to get back into your admin room.
	///
	/// Once this password is unset, all sessions will be logged out for
	/// security purposes.
	///
	/// example: "F670$2CP@Hw8mG7RY1$%!#Ic7YA"
	///
	/// display: sensitive
	pub emergency_password: Option<String>,

	/// default: "/_matrix/push/v1/notify"
	#[serde(default = "default_notification_push_path")]
	pub notification_push_path: String,

	/// Allow local (your server only) presence updates/requests.
	///
	/// Note that presence on conduwuit is very fast unlike Synapse's. If using
	/// outgoing presence, this MUST be enabled.
	#[serde(default = "true_fn")]
	pub allow_local_presence: bool,

	/// Allow incoming federated presence updates/requests.
	///
	/// This option receives presence updates from other servers, but does not
	/// send any unless `allow_outgoing_presence` is true. Note that presence on
	/// conduwuit is very fast unlike Synapse's.
	#[serde(default = "true_fn")]
	pub allow_incoming_presence: bool,

	/// Allow outgoing presence updates/requests.
	///
	/// This option sends presence updates to other servers, but does not
	/// receive any unless `allow_incoming_presence` is true. Note that presence
	/// on conduwuit is very fast unlike Synapse's. If using outgoing presence,
	/// you MUST enable `allow_local_presence` as well.
	#[serde(default = "true_fn")]
	pub allow_outgoing_presence: bool,

	/// How many seconds without presence updates before you become idle.
	/// Defaults to 5 minutes.
	///
	/// default: 300
	#[serde(default = "default_presence_idle_timeout_s")]
	pub presence_idle_timeout_s: u64,

	/// How many seconds without presence updates before you become offline.
	/// Defaults to 30 minutes.
	///
	/// default: 1800
	#[serde(default = "default_presence_offline_timeout_s")]
	pub presence_offline_timeout_s: u64,

	/// Enable the presence idle timer for remote users.
	///
	/// Disabling is offered as an optimization for servers participating in
	/// many large rooms or when resources are limited. Disabling it may cause
	/// incorrect presence states (i.e. stuck online) to be seen for some remote
	/// users.
	#[serde(default = "true_fn")]
	pub presence_timeout_remote_users: bool,

	/// Allow receiving incoming read receipts from remote servers.
	#[serde(default = "true_fn")]
	pub allow_incoming_read_receipts: bool,

	/// Allow sending read receipts to remote servers.
	#[serde(default = "true_fn")]
	pub allow_outgoing_read_receipts: bool,

	/// Allow outgoing typing updates to federation.
	#[serde(default = "true_fn")]
	pub allow_outgoing_typing: bool,

	/// Allow incoming typing updates from federation.
	#[serde(default = "true_fn")]
	pub allow_incoming_typing: bool,

	/// Maximum time federation user can indicate typing.
	///
	/// default: 30
	#[serde(default = "default_typing_federation_timeout_s")]
	pub typing_federation_timeout_s: u64,

	/// Minimum time local client can indicate typing. This does not override a
	/// client's request to stop typing. It only enforces a minimum value in
	/// case of no stop request.
	///
	/// default: 15
	#[serde(default = "default_typing_client_timeout_min_s")]
	pub typing_client_timeout_min_s: u64,

	/// Maximum time local client can indicate typing.
	///
	/// default: 45
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
	///
	/// If you are in a large amount of rooms, you may find that enabling this
	/// is necessary to reduce the significantly large response bodies.
	#[serde(default)]
	pub gzip_compression: bool,

	/// Set this to true for conduwuit to compress HTTP response bodies using
	/// brotli. This option does nothing if conduwuit was not built with
	/// `brotli_compression` feature. Please be aware that enabling HTTP
	/// compression may weaken TLS. Most users should not need to enable this.
	/// See https://breachattack.com/ and https://wikipedia.org/wiki/BREACH
	/// before deciding to enable this.
	#[serde(default)]
	pub brotli_compression: bool,

	/// Set to true to allow user type "guest" registrations. Some clients like
	/// Element attempt to register guest users automatically.
	#[serde(default)]
	pub allow_guest_registration: bool,

	/// Set to true to log guest registrations in the admin room. Note that
	/// these may be noisy or unnecessary if you're a public homeserver.
	#[serde(default)]
	pub log_guest_registrations: bool,

	/// Set to true to allow guest registrations/users to auto join any rooms
	/// specified in `auto_join_rooms`.
	#[serde(default)]
	pub allow_guests_auto_join_rooms: bool,

	/// Enable the legacy unauthenticated Matrix media repository endpoints.
	/// These endpoints consist of:
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

	/// Check consistency of the media directory at startup:
	/// 1. When `media_compat_file_link` is enabled, this check will upgrade
	///    media when switching back and forth between Conduit and conduwuit.
	///    Both options must be enabled to handle this.
	/// 2. When media is deleted from the directory, this check will also delete
	///    its database entry.
	///
	/// If none of these checks apply to your use cases, and your media
	/// directory is significantly large setting this to false may reduce
	/// startup time.
	#[serde(default = "true_fn")]
	pub media_startup_check: bool,

	/// Enable backward-compatibility with Conduit's media directory by creating
	/// symlinks of media.
	///
	/// This option is only necessary if you plan on using Conduit again.
	/// Otherwise setting this to false reduces filesystem clutter and overhead
	/// for managing these symlinks in the directory. This is now disabled by
	/// default. You may still return to upstream Conduit but you have to run
	/// conduwuit at least once with this set to true and allow the
	/// media_startup_check to take place before shutting down to return to
	/// Conduit.
	#[serde(default)]
	pub media_compat_file_link: bool,

	/// Prune missing media from the database as part of the media startup
	/// checks.
	///
	/// This means if you delete files from the media directory the
	/// corresponding entries will be removed from the database. This is
	/// disabled by default because if the media directory is accidentally moved
	/// or inaccessible, the metadata entries in the database will be lost with
	/// sadness.
	#[serde(default)]
	pub prune_missing_media: bool,

	/// Vector list of regex patterns of server names that conduwuit will refuse
	/// to download remote media from.
	///
	/// example: ["badserver\.tld$", "badphrase", "19dollarfortnitecards"]
	///
	/// default: []
	#[serde(default, with = "serde_regex")]
	pub prevent_media_downloads_from: RegexSet,

	/// List of forbidden server names via regex patterns that we will block
	/// incoming AND outgoing federation with, and block client room joins /
	/// remote user invites.
	///
	/// This check is applied on the room ID, room alias, sender server name,
	/// sender user's server name, inbound federation X-Matrix origin, and
	/// outbound federation handler.
	///
	/// Basically "global" ACLs.
	///
	/// example: ["badserver\.tld$", "badphrase", "19dollarfortnitecards"]
	///
	/// default: []
	#[serde(default, with = "serde_regex")]
	pub forbidden_remote_server_names: RegexSet,

	/// List of forbidden server names via regex patterns that we will block all
	/// outgoing federated room directory requests for. Useful for preventing
	/// our users from wandering into bad servers or spaces.
	///
	/// example: ["badserver\.tld$", "badphrase", "19dollarfortnitecards"]
	///
	/// default: []
	#[serde(default, with = "serde_regex")]
	pub forbidden_remote_room_directory_server_names: RegexSet,

	/// Vector list of IPv4 and IPv6 CIDR ranges / subnets *in quotes* that you
	/// do not want conduwuit to send outbound requests to. Defaults to
	/// RFC1918, unroutable, loopback, multicast, and testnet addresses for
	/// security.
	///
	/// Please be aware that this is *not* a guarantee. You should be using a
	/// firewall with zones as doing this on the application layer may have
	/// bypasses.
	///
	/// Currently this does not account for proxies in use like Synapse does.
	///
	/// To disable, set this to be an empty vector (`[]`).
	///
	/// Defaults to:
	/// ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12",
	/// "192.168.0.0/16", "100.64.0.0/10", "192.0.0.0/24", "169.254.0.0/16",
	/// "192.88.99.0/24", "198.18.0.0/15", "192.0.2.0/24", "198.51.100.0/24",
	/// "203.0.113.0/24", "224.0.0.0/4", "::1/128", "fe80::/10", "fc00::/7",
	/// "2001:db8::/32", "ff00::/8", "fec0::/10"]
	#[serde(default = "default_ip_range_denylist")]
	pub ip_range_denylist: Vec<String>,

	/// Optional IP address or network interface-name to bind as the source of
	/// URL preview requests. If not set, it will not bind to a specific
	/// address or interface.
	///
	/// Interface names only supported on Linux, Android, and Fuchsia platforms;
	/// all other platforms can specify the IP address. To list the interfaces
	/// on your system, use the command `ip link show`.
	///
	/// example: `"eth0"` or `"1.2.3.4"`
	///
	/// default:
	#[serde(default, with = "either::serde_untagged_optional")]
	pub url_preview_bound_interface: Option<Either<IpAddr, String>>,

	/// Vector list of domains allowed to send requests to for URL previews.
	///
	/// This is a *contains* match, not an explicit match. Putting "google.com"
	/// will match "https://google.com" and
	/// "http://mymaliciousdomainexamplegoogle.com" Setting this to "*" will
	/// allow all URL previews. Please note that this opens up significant
	/// attack surface to your server, you are expected to be aware of the risks
	/// by doing so.
	///
	/// default: []
	#[serde(default)]
	pub url_preview_domain_contains_allowlist: Vec<String>,

	/// Vector list of explicit domains allowed to send requests to for URL
	/// previews.
	///
	/// This is an *explicit* match, not a contains match. Putting "google.com"
	/// will match "https://google.com", "http://google.com", but not
	/// "https://mymaliciousdomainexamplegoogle.com". Setting this to "*" will
	/// allow all URL previews. Please note that this opens up significant
	/// attack surface to your server, you are expected to be aware of the risks
	/// by doing so.
	///
	/// default: []
	#[serde(default)]
	pub url_preview_domain_explicit_allowlist: Vec<String>,

	/// Vector list of explicit domains not allowed to send requests to for URL
	/// previews.
	///
	/// This is an *explicit* match, not a contains match. Putting "google.com"
	/// will match "https://google.com", "http://google.com", but not
	/// "https://mymaliciousdomainexamplegoogle.com". The denylist is checked
	/// first before allowlist. Setting this to "*" will not do anything.
	///
	/// default: []
	#[serde(default)]
	pub url_preview_domain_explicit_denylist: Vec<String>,

	/// Vector list of URLs allowed to send requests to for URL previews.
	///
	/// Note that this is a *contains* match, not an explicit match. Putting
	/// "google.com" will match "https://google.com/",
	/// "https://google.com/url?q=https://mymaliciousdomainexample.com", and
	/// "https://mymaliciousdomainexample.com/hi/google.com" Setting this to "*"
	/// will allow all URL previews. Please note that this opens up significant
	/// attack surface to your server, you are expected to be aware of the risks
	/// by doing so.
	///
	/// default: []
	#[serde(default)]
	pub url_preview_url_contains_allowlist: Vec<String>,

	/// Maximum amount of bytes allowed in a URL preview body size when
	/// spidering. Defaults to 256KB in bytes.
	///
	/// default: 256000
	#[serde(default = "default_url_preview_max_spider_size")]
	pub url_preview_max_spider_size: usize,

	/// Option to decide whether you would like to run the domain allowlist
	/// checks (contains and explicit) on the root domain or not. Does not apply
	/// to URL contains allowlist. Defaults to false.
	///
	/// Example usecase: If this is enabled and you have "wikipedia.org" allowed
	/// in the explicit and/or contains domain allowlist, it will allow all
	/// subdomains under "wikipedia.org" such as "en.m.wikipedia.org" as the
	/// root domain is checked and matched. Useful if the domain contains
	/// allowlist is still too broad for you but you still want to allow all the
	/// subdomains under a root domain.
	#[serde(default)]
	pub url_preview_check_root_domain: bool,

	/// List of forbidden room aliases and room IDs as strings of regex
	/// patterns.
	///
	/// Regex can be used or explicit contains matches can be done by just
	/// specifying the words (see example).
	///
	/// This is checked upon room alias creation, custom room ID creation if
	/// used, and startup as warnings if any room aliases in your database have
	/// a forbidden room alias/ID.
	///
	/// example: ["19dollarfortnitecards", "b[4a]droom", "badphrase"]
	///
	/// default: []
	#[serde(default, with = "serde_regex")]
	pub forbidden_alias_names: RegexSet,

	/// List of forbidden username patterns/strings.
	///
	/// Regex can be used or explicit contains matches can be done by just
	/// specifying the words (see example).
	///
	/// This is checked upon username availability check, registration, and
	/// startup as warnings if any local users in your database have a forbidden
	/// username.
	///
	/// example: ["administrator", "b[a4]dusernam[3e]", "badphrase"]
	///
	/// default: []
	#[serde(default, with = "serde_regex")]
	pub forbidden_usernames: RegexSet,

	/// Retry failed and incomplete messages to remote servers immediately upon
	/// startup. This is called bursting. If this is disabled, said messages may
	/// not be delivered until more messages are queued for that server. Do not
	/// change this option unless server resources are extremely limited or the
	/// scale of the server's deployment is huge. Do not disable this unless you
	/// know what you are doing.
	#[serde(default = "true_fn")]
	pub startup_netburst: bool,

	/// Messages are dropped and not reattempted. The `startup_netburst` option
	/// must be enabled for this value to have any effect. Do not change this
	/// value unless you know what you are doing. Set this value to -1 to
	/// reattempt every message without trimming the queues; this may consume
	/// significant disk. Set this value to 0 to drop all messages without any
	/// attempt at redelivery.
	///
	/// default: 50
	#[serde(default = "default_startup_netburst_keep")]
	pub startup_netburst_keep: i64,

	/// Block non-admin local users from sending room invites (local and
	/// remote), and block non-admin users from receiving remote room invites.
	///
	/// Admins are always allowed to send and receive all room invites.
	#[serde(default)]
	pub block_non_admin_invites: bool,

	/// Allow admins to enter commands in rooms other than "#admins" (admin
	/// room) by prefixing your message with "\!admin" or "\\!admin" followed up
	/// a normal conduwuit admin command. The reply will be publicly visible to
	/// the room, originating from the sender.
	///
	/// example: \\!admin debug ping puppygock.gay
	#[serde(default = "true_fn")]
	pub admin_escape_commands: bool,

	/// Automatically activate the conduwuit admin room console / CLI on
	/// startup. This option can also be enabled with `--console` conduwuit
	/// argument.
	#[serde(default)]
	pub admin_console_automatic: bool,

	/// List of admin commands to execute on startup.
	///
	/// This option can also be configured with the `--execute` conduwuit
	/// argument and can take standard shell commands and environment variables
	///
	/// For example: `./conduwuit --execute "server admin-notice conduwuit has
	/// started up at $(date)"`
	///
	/// example: admin_execute = ["debug ping puppygock.gay", "debug echo hi"]`
	///
	/// default: []
	#[serde(default)]
	pub admin_execute: Vec<String>,

	/// Ignore errors in startup commands.
	///
	/// If false, conduwuit will error and fail to start if an admin execute
	/// command (`--execute` / `admin_execute`) fails.
	#[serde(default)]
	pub admin_execute_errors_ignore: bool,

	/// List of admin commands to execute on SIGUSR2.
	///
	/// Similar to admin_execute, but these commands are executed when the
	/// server receives SIGUSR2 on supporting platforms.
	///
	/// default: []
	#[serde(default)]
	pub admin_signal_execute: Vec<String>,

	/// Controls the max log level for admin command log captures (logs
	/// generated from running admin commands). Defaults to "info" on release
	/// builds, else "debug" on debug builds.
	///
	/// default: "info"
	#[serde(default = "default_admin_log_capture")]
	pub admin_log_capture: String,

	/// The default room tag to apply on the admin room.
	///
	/// On some clients like Element, the room tag "m.server_notice" is a
	/// special pinned room at the very bottom of your room list. The conduwuit
	/// admin room can be pinned here so you always have an easy-to-access
	/// shortcut dedicated to your admin room.
	///
	/// default: "m.server_notice"
	#[serde(default = "default_admin_room_tag")]
	pub admin_room_tag: String,

	/// Sentry.io crash/panic reporting, performance monitoring/metrics, etc.
	/// This is NOT enabled by default. conduwuit's default Sentry reporting
	/// endpoint domain is `o4506996327251968.ingest.us.sentry.io`.
	#[serde(default)]
	pub sentry: bool,

	/// Sentry reporting URL, if a custom one is desired.
	///
	/// display: sensitive
	/// default: "https://fe2eb4536aa04949e28eff3128d64757@o4506996327251968.ingest.us.sentry.io/4506996334657536"
	#[serde(default = "default_sentry_endpoint")]
	pub sentry_endpoint: Option<Url>,

	/// Report your conduwuit server_name in Sentry.io crash reports and
	/// metrics.
	#[serde(default)]
	pub sentry_send_server_name: bool,

	/// Performance monitoring/tracing sample rate for Sentry.io.
	///
	/// Note that too high values may impact performance, and can be disabled by
	/// setting it to 0.0 (0%) This value is read as a percentage to Sentry,
	/// represented as a decimal. Defaults to 15% of traces (0.15)
	///
	/// default: 0.15
	#[serde(default = "default_sentry_traces_sample_rate")]
	pub sentry_traces_sample_rate: f32,

	/// Whether to attach a stacktrace to Sentry reports.
	#[serde(default)]
	pub sentry_attach_stacktrace: bool,

	/// Send panics to Sentry. This is true by default, but Sentry has to be
	/// enabled. The global `sentry` config option must be enabled to send any
	/// data.
	#[serde(default = "true_fn")]
	pub sentry_send_panic: bool,

	/// Send errors to sentry. This is true by default, but sentry has to be
	/// enabled. This option is only effective in release-mode; forced to false
	/// in debug-mode.
	#[serde(default = "true_fn")]
	pub sentry_send_error: bool,

	/// Controls the tracing log level for Sentry to send things like
	/// breadcrumbs and transactions
	///
	/// default: "info"
	#[serde(default = "default_sentry_filter")]
	pub sentry_filter: String,

	/// Enable the tokio-console. This option is only relevant to developers.
	///
	///	For more information, see:
	/// https://conduwuit.puppyirl.gay/development.html#debugging-with-tokio-console
	#[serde(default)]
	pub tokio_console: bool,

	#[serde(default)]
	pub test: BTreeSet<String>,

	/// Controls whether admin room notices like account registrations, password
	/// changes, account deactivations, room directory publications, etc will be
	/// sent to the admin room. Update notices and normal admin command
	/// responses will still be sent.
	#[serde(default = "true_fn")]
	pub admin_room_notices: bool,

	/// Enable database pool affinity support. On supporting systems, block
	/// device queue topologies are detected and the request pool is optimized
	/// for the hardware; db_pool_workers is determined automatically.
	///
	/// default: true
	#[serde(default = "true_fn")]
	pub db_pool_affinity: bool,

	/// Sets the number of worker threads in the frontend-pool of the database.
	/// This number should reflect the I/O capabilities of the system,
	/// such as the queue-depth or the number of simultaneous requests in
	/// flight. Defaults to 32 or four times the number of CPU cores, whichever
	/// is greater.
	///
	/// Note: This value is only used if db_pool_affinity is disabled or not
	/// detected on the system, otherwise it is determined automatically.
	///
	/// default: 32
	#[serde(default = "default_db_pool_workers")]
	pub db_pool_workers: usize,

	/// When db_pool_affinity is enabled and detected, the size of any worker
	/// group will not exceed the determined value. This is necessary when
	/// thread-pooling approach does not scale to the full capabilities of
	/// high-end hardware; using detected values without limitation could
	/// degrade performance.
	///
	/// The value is multiplied by the number of cores which share a device
	/// queue, since group workers can be scheduled on any of those cores.
	///
	/// default: 64
	#[serde(default = "default_db_pool_workers_limit")]
	pub db_pool_workers_limit: usize,

	/// Determines the size of the queues feeding the database's frontend-pool.
	/// The size of the queue is determined by multiplying this value with the
	/// number of pool workers. When this queue is full, tokio tasks conducting
	/// requests will yield until space is available; this is good for
	/// flow-control by avoiding buffer-bloat, but can inhibit throughput if
	/// too low.
	///
	/// default: 4
	#[serde(default = "default_db_pool_queue_mult")]
	pub db_pool_queue_mult: usize,

	/// Sets the initial value for the concurrency of streams. This value simply
	/// allows overriding the default in the code. The default is 32, which is
	/// the same as the default in the code. Note this value is itself
	/// overridden by the computed stream_width_scale, unless that is disabled;
	/// this value can serve as a fixed-width instead.
	///
	/// default: 32
	#[serde(default = "default_stream_width_default")]
	pub stream_width_default: usize,

	/// Scales the stream width starting from a base value detected for the
	/// specific system. The base value is the database pool worker count
	/// determined from the hardware queue size (e.g. 32 for SSD or 64 or 128+
	/// for NVMe). This float allows scaling the width up or down by multiplying
	/// it (e.g. 1.5, 2.0, etc). The maximum result can be the size of the pool
	/// queue (see: db_pool_queue_mult) as any larger value will stall the tokio
	/// task. The value can also be scaled down (e.g. 0.5)  to improve
	/// responsiveness for many users at the cost of throughput for each.
	///
	/// Setting this value to 0.0 causes the stream width to be fixed at the
	/// value of stream_width_default. The default scale is 1.0 to match the
	/// capabilities detected for the system.
	///
	/// default: 1.0
	#[serde(default = "default_stream_width_scale")]
	pub stream_width_scale: f32,

	/// Sets the initial amplification factor. This controls batch sizes of
	/// requests made by each pool worker, multiplying the throughput of each
	/// stream. This value is somewhat abstract from specific hardware
	/// characteristics and can be significantly larger than any thread count or
	/// queue size. This is because each database query may require several
	/// index lookups, thus many database queries in a batch may make progress
	/// independently while also sharing index and data blocks which may or may
	/// not be cached. It is worthwhile to submit huge batches to reduce
	/// complexity. The maximum value is 32768, though sufficient hardware is
	/// still advised for that.
	///
	/// default: 1024
	#[serde(default = "default_stream_amplification")]
	pub stream_amplification: usize,

	/// Number of sender task workers; determines sender parallelism. Default is
	/// '0' which means the value is determined internally, likely matching the
	/// number of tokio worker-threads or number of cores, etc. Override by
	/// setting a non-zero value.
	///
	/// default: 0
	#[serde(default)]
	pub sender_workers: usize,

	/// Enables listener sockets; can be set to false to disable listening. This
	/// option is intended for developer/diagnostic purposes only.
	#[serde(default = "true_fn")]
	pub listening: bool,

	/// Enables configuration reload when the server receives SIGUSR1 on
	/// supporting platforms.
	///
	/// default: true
	#[serde(default = "true_fn")]
	pub config_reload_signal: bool,

	/// Toggles ignore checking/validating TLS certificates
	///
	/// This applies to everything, including URL previews, federation requests,
	/// etc. This is a hidden argument that should NOT be used in production as
	/// it is highly insecure and I will personally yell at you if I catch you
	/// using this.
	#[serde(default)]
	pub allow_invalid_tls_certificates_yes_i_know_what_the_fuck_i_am_doing_with_this_and_i_know_this_is_insecure:
		bool,

	// external structure; separate section
	#[serde(default)]
	pub blurhashing: BlurhashConfig,
	#[serde(flatten)]
	#[allow(clippy::zero_sized_map_values)]
	// this is a catchall, the map shouldn't be zero at runtime
	catchall: BTreeMap<String, IgnoredAny>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[config_example_generator(filename = "conduwuit-example.toml", section = "global.tls")]
pub struct TlsConfig {
	/// Path to a valid TLS certificate file.
	///
	/// example: "/path/to/my/certificate.crt"
	pub certs: Option<String>,

	/// Path to a valid TLS certificate private key.
	///
	/// example: "/path/to/my/certificate.key"
	pub key: Option<String>,

	/// Whether to listen and allow for HTTP and HTTPS connections (insecure!)
	#[serde(default)]
	pub dual_protocol: bool,
}

#[allow(rustdoc::broken_intra_doc_links, rustdoc::bare_urls)]
#[derive(Clone, Debug, Deserialize, Default)]
#[config_example_generator(filename = "conduwuit-example.toml", section = "global.well_known")]
pub struct WellKnownConfig {
	/// The server URL that the client well-known file will serve. This should
	/// not contain a port, and should just be a valid HTTPS URL.
	///
	/// example: "https://matrix.example.com"
	pub client: Option<Url>,

	/// The server base domain of the URL with a specific port that the server
	/// well-known file will serve. This should contain a port at the end, and
	/// should not be a URL.
	///
	/// example: "matrix.example.com:443"
	pub server: Option<OwnedServerName>,

	pub support_page: Option<Url>,

	pub support_role: Option<ContactRole>,

	pub support_email: Option<String>,

	pub support_mxid: Option<OwnedUserId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Default)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::bare_urls)]
#[config_example_generator(filename = "conduwuit-example.toml", section = "global.blurhashing")]
pub struct BlurhashConfig {
	/// blurhashing x component, 4 is recommended by https://blurha.sh/
	///
	/// default: 4
	#[serde(default = "default_blurhash_x_component")]
	pub components_x: u32,
	/// blurhashing y component, 3 is recommended by https://blurha.sh/
	///
	/// default: 3
	#[serde(default = "default_blurhash_y_component")]
	pub components_y: u32,
	/// Max raw size that the server will blurhash, this is the size of the
	/// image after converting it to raw data, it should be higher than the
	/// upload limit but not too high. The higher it is the higher the
	/// potential load will be for clients requesting blurhashes. The default
	/// is 33.55MB. Setting it to 0 disables blurhashing.
	///
	/// default: 33554432
	#[serde(default = "default_blurhash_max_raw_size")]
	pub blurhash_max_raw_size: u64,
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
	pub fn load<'a, I>(paths: I) -> Result<Figment>
	where
		I: Iterator<Item = &'a Path>,
	{
		let envs = [Env::var("CONDUIT_CONFIG"), Env::var("CONDUWUIT_CONFIG")];

		let config = envs
			.into_iter()
			.flatten()
			.map(Toml::file)
			.chain(paths.map(Toml::file))
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
			| Left(addr) => vec![*addr],
			| Right(addrs) => addrs.clone(),
		}
	}

	fn get_bind_ports(&self) -> Vec<u16> {
		match &self.port.ports {
			| Left(port) => vec![*port],
			| Right(ports) => ports.clone(),
		}
	}

	pub fn check(&self) -> Result<(), Error> { check(self) }
}

fn true_fn() -> bool { true }

fn default_address() -> ListeningAddr {
	ListeningAddr {
		addrs: Right(vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]),
	}
}

fn default_port() -> ListeningPort { ListeningPort { ports: Left(8008) } }

fn default_unix_socket_perms() -> u32 { 660 }

fn default_database_backups_to_keep() -> i16 { 1 }

fn default_db_write_buffer_capacity_mb() -> f64 { 48.0 + parallelism_scaled_f64(4.0) }

fn default_db_cache_capacity_mb() -> f64 { 128.0 + parallelism_scaled_f64(64.0) }

fn default_pdu_cache_capacity() -> u32 { parallelism_scaled_u32(10_000).saturating_add(100_000) }

fn default_cache_capacity_modifier() -> f64 { 1.0 }

fn default_auth_chain_cache_capacity() -> u32 {
	parallelism_scaled_u32(10_000).saturating_add(100_000)
}

fn default_shorteventid_cache_capacity() -> u32 {
	parallelism_scaled_u32(50_000).saturating_add(100_000)
}

fn default_eventidshort_cache_capacity() -> u32 {
	parallelism_scaled_u32(25_000).saturating_add(100_000)
}

fn default_eventid_pdu_cache_capacity() -> u32 {
	parallelism_scaled_u32(25_000).saturating_add(100_000)
}

fn default_shortstatekey_cache_capacity() -> u32 {
	parallelism_scaled_u32(10_000).saturating_add(100_000)
}

fn default_statekeyshort_cache_capacity() -> u32 {
	parallelism_scaled_u32(10_000).saturating_add(100_000)
}

fn default_servernameevent_data_cache_capacity() -> u32 {
	parallelism_scaled_u32(100_000).saturating_add(500_000)
}

fn default_stateinfo_cache_capacity() -> u32 { parallelism_scaled_u32(100) }

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

fn default_max_fetch_prev_events() -> u16 { 192_u16 }

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

fn default_trusted_servers() -> Vec<OwnedServerName> {
	vec![OwnedServerName::try_from("matrix.org").unwrap()]
}

/// do debug logging by default for debug builds
#[must_use]
pub fn default_log() -> String {
	cfg!(debug_assertions)
		.then_some("debug")
		.unwrap_or("info")
		.to_owned()
}

#[must_use]
pub fn default_log_span_events() -> String { "none".into() }

fn default_notification_push_path() -> String { "/_matrix/push/v1/notify".to_owned() }

fn default_openid_token_ttl() -> u64 { 60 * 60 }

fn default_login_token_ttl() -> u64 { 2 * 60 * 1000 }

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

fn default_rocksdb_compression_algo() -> String {
	cfg!(feature = "zstd_compression")
		.then_some("zstd")
		.unwrap_or("none")
		.to_owned()
}

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
#[inline]
pub fn default_default_room_version() -> RoomVersionId { RoomVersionId::V11 }

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
	256_000 // 256KB
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

fn default_trusted_server_batch_size() -> usize { 256 }

fn default_db_pool_workers() -> usize {
	sys::available_parallelism()
		.saturating_mul(4)
		.clamp(32, 1024)
}

fn default_db_pool_workers_limit() -> usize { 64 }

fn default_db_pool_queue_mult() -> usize { 4 }

fn default_stream_width_default() -> usize { 32 }

fn default_stream_width_scale() -> f32 { 1.0 }

fn default_stream_amplification() -> usize { 1024 }

fn default_client_receive_timeout() -> u64 { 75 }

fn default_client_request_timeout() -> u64 { 180 }

fn default_client_response_timeout() -> u64 { 120 }

fn default_client_shutdown_timeout() -> u64 { 15 }

fn default_sender_shutdown_timeout() -> u64 { 5 }

// blurhashing defaults recommended by https://blurha.sh/
// 2^25
pub(super) fn default_blurhash_max_raw_size() -> u64 { 33_554_432 }

pub(super) fn default_blurhash_x_component() -> u32 { 4 }

pub(super) fn default_blurhash_y_component() -> u32 { 3 }

// end recommended & blurhashing defaults
