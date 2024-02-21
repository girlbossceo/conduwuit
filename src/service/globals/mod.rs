mod data;
use argon2::Argon2;
pub use data::Data;
use regex::RegexSet;
use ruma::{
    serde::Base64, OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedServerName,
    OwnedServerSigningKeyId, OwnedUserId,
};

use sha2::Digest;

use crate::api::server_server::FedDest;

use crate::{services, Config, Error, Result};
use futures_util::FutureExt;
use hyper::{
    client::connect::dns::{GaiResolver, Name},
    service::Service as HyperService,
};
use reqwest::dns::{Addrs, Resolve, Resolving};
use ruma::{
    api::{
        client::sync::sync_events,
        federation::discovery::{ServerSigningKeys, VerifyKey},
    },
    DeviceId, RoomVersionId, ServerName, UserId,
};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error as StdError,
    fs,
    future::{self, Future},
    iter,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{self, AtomicBool},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, watch::Receiver, Mutex as TokioMutex, Semaphore};
use tracing::{error, info};
use trust_dns_resolver::TokioAsyncResolver;

use base64::{engine::general_purpose, Engine as _};

type WellKnownMap = HashMap<OwnedServerName, (FedDest, String)>;
type TlsNameMap = HashMap<String, (Vec<IpAddr>, u16)>;
type RateLimitState = (Instant, u32); // Time if last failed try, number of failed tries
type SyncHandle = (
    Option<String>,                                      // since
    Receiver<Option<Result<sync_events::v3::Response>>>, // rx
);

pub struct Service<'a> {
    pub db: &'static dyn Data,

    pub actual_destination_cache: Arc<RwLock<WellKnownMap>>, // actual_destination, host
    pub tls_name_override: Arc<RwLock<TlsNameMap>>,
    pub config: Config,
    keypair: Arc<ruma::signatures::Ed25519KeyPair>,
    dns_resolver: TokioAsyncResolver,
    jwt_decoding_key: Option<jsonwebtoken::DecodingKey>,
    url_preview_client: reqwest::Client,
    federation_client: reqwest::Client,
    default_client: reqwest::Client,
    pub stable_room_versions: Vec<RoomVersionId>,
    pub unstable_room_versions: Vec<RoomVersionId>,
    pub bad_event_ratelimiter: Arc<RwLock<HashMap<OwnedEventId, RateLimitState>>>,
    pub bad_signature_ratelimiter: Arc<RwLock<HashMap<Vec<String>, RateLimitState>>>,
    pub bad_query_ratelimiter: Arc<RwLock<HashMap<OwnedServerName, RateLimitState>>>,
    pub servername_ratelimiter: Arc<RwLock<HashMap<OwnedServerName, Arc<Semaphore>>>>,
    pub sync_receivers: RwLock<HashMap<(OwnedUserId, OwnedDeviceId), SyncHandle>>,
    pub roomid_mutex_insert: RwLock<HashMap<OwnedRoomId, Arc<TokioMutex<()>>>>,
    pub roomid_mutex_state: RwLock<HashMap<OwnedRoomId, Arc<TokioMutex<()>>>>,
    pub roomid_mutex_federation: RwLock<HashMap<OwnedRoomId, Arc<TokioMutex<()>>>>, // this lock will be held longer
    pub roomid_federationhandletime: RwLock<HashMap<OwnedRoomId, (OwnedEventId, Instant)>>,
    pub stateres_mutex: Arc<Mutex<()>>,
    pub(crate) rotate: RotationHandler,

    pub shutdown: AtomicBool,
    pub argon: Argon2<'a>,
}

/// Handles "rotation" of long-polling requests. "Rotation" in this context is similar to "rotation" of log files and the like.
///
/// This is utilized to have sync workers return early and release read locks on the database.
pub(crate) struct RotationHandler(broadcast::Sender<()>, ());

impl RotationHandler {
    pub fn new() -> Self {
        let (s, _r) = broadcast::channel(1);
        Self(s, ())
    }

    pub fn watch(&self) -> impl Future<Output = ()> {
        let mut r = self.0.subscribe();

        async move {
            let _ = r.recv().await;
        }
    }

    pub fn fire(&self) {
        let _ = self.0.send(());
    }
}

impl Default for RotationHandler {
    fn default() -> Self {
        Self::new()
    }
}

struct Resolver {
    inner: GaiResolver,
    overrides: Arc<RwLock<TlsNameMap>>,
}

impl Resolver {
    fn new(overrides: Arc<RwLock<TlsNameMap>>) -> Self {
        Resolver {
            inner: GaiResolver::new(),
            overrides,
        }
    }
}

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        self.overrides
            .read()
            .expect("lock should not be poisoned")
            .get(name.as_str())
            .and_then(|(override_name, port)| {
                override_name.first().map(|first_name| {
                    let x: Box<dyn Iterator<Item = SocketAddr> + Send> =
                        Box::new(iter::once(SocketAddr::new(*first_name, *port)));
                    let x: Resolving = Box::pin(future::ready(Ok(x)));
                    x
                })
            })
            .unwrap_or_else(|| {
                let this = &mut self.inner.clone();
                Box::pin(HyperService::<Name>::call(this, name).map(|result| {
                    result
                        .map(|addrs| -> Addrs { Box::new(addrs) })
                        .map_err(|err| -> Box<dyn StdError + Send + Sync> { Box::new(err) })
                }))
            })
    }
}

impl Service<'_> {
    pub fn load(db: &'static dyn Data, config: Config) -> Result<Self> {
        let keypair = db.load_keypair();

        let keypair = match keypair {
            Ok(k) => k,
            Err(e) => {
                error!("Keypair invalid. Deleting...");
                db.remove_keypair()?;
                return Err(e);
            }
        };

        let tls_name_override = Arc::new(RwLock::new(TlsNameMap::new()));

        let jwt_decoding_key = config
            .jwt_secret
            .as_ref()
            .map(|secret| jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()));

        let url_preview_client = url_preview_reqwest_client_builder(&config)?.build()?;
        let default_client = reqwest_client_builder(&config)?.build()?;
        let federation_client = reqwest_client_builder(&config)?
            .dns_resolver(Arc::new(Resolver::new(tls_name_override.clone())))
            .build()?;

        // Supported and stable room versions
        let stable_room_versions = vec![
            RoomVersionId::V6,
            RoomVersionId::V7,
            RoomVersionId::V8,
            RoomVersionId::V9,
            RoomVersionId::V10,
        ];
        // Experimental, partially supported room versions
        let unstable_room_versions = vec![
            RoomVersionId::V1,
            RoomVersionId::V2,
            RoomVersionId::V3,
            RoomVersionId::V4,
            RoomVersionId::V5,
            RoomVersionId::V11,
        ];
        // 19456 Kib blocks, iterations = 2, parallelism = 1 for more info https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#argon2id
        let argon = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::default(),
            argon2::Params::new(19456, 2, 1, None).expect("valid parameters"),
        );
        let mut s = Self {
            db,
            config,
            keypair: Arc::new(keypair),
            dns_resolver: TokioAsyncResolver::tokio_from_system_conf().map_err(|e| {
                error!(
                    "Failed to set up trust dns resolver with system config: {}",
                    e
                );
                Error::bad_config("Failed to set up trust dns resolver with system config.")
            })?,
            actual_destination_cache: Arc::new(RwLock::new(WellKnownMap::new())),
            tls_name_override,
            url_preview_client,
            federation_client,
            default_client,
            jwt_decoding_key,
            stable_room_versions,
            unstable_room_versions,
            bad_event_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
            bad_signature_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
            bad_query_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
            servername_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
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
    pub fn keypair(&self) -> &ruma::signatures::Ed25519KeyPair {
        &self.keypair
    }

    /// Returns a reqwest client which can be used to send requests for URL previews
    /// This is the same as `default_client()` except a redirect policy of max 2 is set
    pub fn url_preview_client(&self) -> reqwest::Client {
        // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
        self.url_preview_client.clone()
    }

    /// Returns a reqwest client which can be used to send requests
    pub fn default_client(&self) -> reqwest::Client {
        // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
        self.default_client.clone()
    }

    /// Returns a client used for resolving .well-knowns
    pub fn federation_client(&self) -> reqwest::Client {
        // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
        self.federation_client.clone()
    }

    #[tracing::instrument(skip(self))]
    pub fn next_count(&self) -> Result<u64> {
        self.db.next_count()
    }

    #[tracing::instrument(skip(self))]
    pub fn current_count(&self) -> Result<u64> {
        self.db.current_count()
    }

    #[tracing::instrument(skip(self))]
    pub fn last_check_for_updates_id(&self) -> Result<u64> {
        self.db.last_check_for_updates_id()
    }

    #[tracing::instrument(skip(self))]
    pub fn update_check_for_updates_id(&self, id: u64) -> Result<()> {
        self.db.update_check_for_updates_id(id)
    }

    pub async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result<()> {
        self.db.watch(user_id, device_id).await
    }

    pub fn cleanup(&self) -> Result<()> {
        self.db.cleanup()
    }

    pub fn server_name(&self) -> &ServerName {
        self.config.server_name.as_ref()
    }

    pub fn max_request_size(&self) -> u32 {
        self.config.max_request_size
    }

    pub fn max_fetch_prev_events(&self) -> u16 {
        self.config.max_fetch_prev_events
    }

    pub fn allow_registration(&self) -> bool {
        self.config.allow_registration
    }

    pub fn allow_guest_registration(&self) -> bool {
        self.config.allow_guest_registration
    }

    pub fn allow_encryption(&self) -> bool {
        self.config.allow_encryption
    }

    pub fn allow_federation(&self) -> bool {
        self.config.allow_federation
    }

    pub fn allow_public_room_directory_over_federation(&self) -> bool {
        self.config.allow_public_room_directory_over_federation
    }

    pub fn allow_public_room_directory_without_auth(&self) -> bool {
        self.config.allow_public_room_directory_without_auth
    }

    pub fn allow_device_name_federation(&self) -> bool {
        self.config.allow_device_name_federation
    }

    pub fn allow_room_creation(&self) -> bool {
        self.config.allow_room_creation
    }

    pub fn allow_unstable_room_versions(&self) -> bool {
        self.config.allow_unstable_room_versions
    }

    pub fn default_room_version(&self) -> RoomVersionId {
        self.config.default_room_version.clone()
    }

    pub fn new_user_displayname_suffix(&self) -> &String {
        &self.config.new_user_displayname_suffix
    }

    pub fn allow_check_for_updates(&self) -> bool {
        self.config.allow_check_for_updates
    }

    pub fn trusted_servers(&self) -> &[OwnedServerName] {
        &self.config.trusted_servers
    }

    pub fn dns_resolver(&self) -> &TokioAsyncResolver {
        &self.dns_resolver
    }

    pub fn jwt_decoding_key(&self) -> Option<&jsonwebtoken::DecodingKey> {
        self.jwt_decoding_key.as_ref()
    }

    pub fn turn_password(&self) -> &String {
        &self.config.turn_password
    }

    pub fn turn_ttl(&self) -> u64 {
        self.config.turn_ttl
    }

    pub fn turn_uris(&self) -> &[String] {
        &self.config.turn_uris
    }

    pub fn turn_username(&self) -> &String {
        &self.config.turn_username
    }

    pub fn turn_secret(&self) -> &String {
        &self.config.turn_secret
    }

    pub fn notification_push_path(&self) -> &String {
        &self.config.notification_push_path
    }

    pub fn emergency_password(&self) -> &Option<String> {
        &self.config.emergency_password
    }

    pub fn url_preview_domain_contains_allowlist(&self) -> &Vec<String> {
        &self.config.url_preview_domain_contains_allowlist
    }

    pub fn url_preview_domain_explicit_allowlist(&self) -> &Vec<String> {
        &self.config.url_preview_domain_explicit_allowlist
    }

    pub fn url_preview_url_contains_allowlist(&self) -> &Vec<String> {
        &self.config.url_preview_url_contains_allowlist
    }

    pub fn url_preview_max_spider_size(&self) -> usize {
        self.config.url_preview_max_spider_size
    }

    pub fn url_preview_check_root_domain(&self) -> bool {
        self.config.url_preview_check_root_domain
    }

    pub fn forbidden_room_names(&self) -> &RegexSet {
        &self.config.forbidden_room_names
    }

    pub fn forbidden_usernames(&self) -> &RegexSet {
        &self.config.forbidden_usernames
    }

    pub fn allow_local_presence(&self) -> bool {
        self.config.allow_local_presence
    }

    pub fn allow_incoming_presence(&self) -> bool {
        self.config.allow_incoming_presence
    }

    pub fn allow_outgoing_presence(&self) -> bool {
        self.config.allow_outgoing_presence
    }

    pub fn presence_idle_timeout_s(&self) -> u64 {
        self.config.presence_idle_timeout_s
    }

    pub fn presence_offline_timeout_s(&self) -> u64 {
        self.config.presence_offline_timeout_s
    }

    pub fn rocksdb_log_level(&self) -> &String {
        &self.config.rocksdb_log_level
    }

    pub fn rocksdb_max_log_file_size(&self) -> usize {
        self.config.rocksdb_max_log_file_size
    }

    pub fn rocksdb_log_time_to_roll(&self) -> usize {
        self.config.rocksdb_log_time_to_roll
    }

    pub fn rocksdb_optimize_for_spinning_disks(&self) -> bool {
        self.config.rocksdb_optimize_for_spinning_disks
    }

    pub fn prevent_media_downloads_from(&self) -> &[OwnedServerName] {
        &self.config.prevent_media_downloads_from
    }

    pub fn ip_range_denylist(&self) -> &[String] {
        &self.config.ip_range_denylist
    }

    pub fn supported_room_versions(&self) -> Vec<RoomVersionId> {
        let mut room_versions: Vec<RoomVersionId> = vec![];
        room_versions.extend(self.stable_room_versions.clone());
        if self.allow_unstable_room_versions() {
            room_versions.extend(self.unstable_room_versions.clone());
        };
        room_versions
    }

    /// TODO: the key valid until timestamp (`valid_until_ts`) is only honored in room version > 4
    ///
    /// Remove the outdated keys and insert the new ones.
    ///
    /// This doesn't actually check that the keys provided are newer than the old set.
    pub fn add_signing_key(
        &self,
        origin: &ServerName,
        new_keys: ServerSigningKeys,
    ) -> Result<BTreeMap<OwnedServerSigningKeyId, VerifyKey>> {
        self.db.add_signing_key(origin, new_keys)
    }

    /// This returns an empty `Ok(BTreeMap<..>)` when there are no keys found for the server.
    pub fn signing_keys_for(
        &self,
        origin: &ServerName,
    ) -> Result<BTreeMap<OwnedServerSigningKeyId, VerifyKey>> {
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

    pub fn database_version(&self) -> Result<u64> {
        self.db.database_version()
    }

    pub fn bump_database_version(&self, new_version: u64) -> Result<()> {
        self.db.bump_database_version(new_version)
    }

    pub fn get_media_folder(&self) -> PathBuf {
        let mut r = PathBuf::new();
        r.push(self.config.database_path.clone());
        r.push("media");
        r
    }

    /// new SHA256 file name media function, requires "sha256_media" feature flag enabled and database migrated
    /// uses SHA256 hash of the base64 key as the file name
    pub fn get_media_file_new(&self, key: &[u8]) -> PathBuf {
        let mut r = PathBuf::new();
        r.push(self.config.database_path.clone());
        r.push("media");
        // Using the hash of the base64 key as the filename
        // This is to prevent the total length of the path from exceeding the maximum length in most filesystems
        r.push(general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(key)));
        r
    }

    /// old base64 file name media function
    /// This is the old version of `get_media_file` that uses the full base64 key as the filename.
    ///
    /// This is deprecated and will be removed in a future release.
    /// Please use `get_media_file_new` instead.
    #[deprecated(note = "Use get_media_file_new instead")]
    pub fn get_media_file(&self, key: &[u8]) -> PathBuf {
        let mut r = PathBuf::new();
        r.push(self.config.database_path.clone());
        r.push("media");
        r.push(general_purpose::URL_SAFE_NO_PAD.encode(key));
        r
    }

    pub fn well_known_client(&self) -> &Option<String> {
        &self.config.well_known_client
    }

    pub fn well_known_server(&self) -> &Option<String> {
        &self.config.well_known_server
    }

    pub fn unix_socket_path(&self) -> &Option<PathBuf> {
        &self.config.unix_socket_path
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, atomic::Ordering::Relaxed);
        // On shutdown

        if self.unix_socket_path().is_some() {
            match &self.unix_socket_path() {
                Some(path) => {
                    std::fs::remove_file(path).unwrap();
                }
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

fn reqwest_client_builder(config: &Config) -> Result<reqwest::ClientBuilder> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > 6 {
            attempt.error("Too many redirects (max is 6)")
        } else {
            attempt.follow()
        }
    });

    let mut reqwest_client_builder = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(60 * 5))
        .redirect(redirect_policy)
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ));

    if let Some(proxy) = config.proxy.to_proxy()? {
        reqwest_client_builder = reqwest_client_builder.proxy(proxy);
    }

    Ok(reqwest_client_builder)
}

fn url_preview_reqwest_client_builder(config: &Config) -> Result<reqwest::ClientBuilder> {
    // for security reasons (e.g. malicious open redirect), we do not want to follow too many redirects when generating URL previews.
    // let's keep it at least 2 to account for HTTP -> HTTPS upgrades, if it becomes an issue we can consider raising it to 3.
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > 2 {
            attempt.error("Too many redirects (max is 2)")
        } else {
            attempt.follow()
        }
    });

    let mut reqwest_client_builder = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(60 * 5))
        .redirect(redirect_policy)
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ));

    if let Some(proxy) = config.proxy.to_proxy()? {
        reqwest_client_builder = reqwest_client_builder.proxy(proxy);
    }

    Ok(reqwest_client_builder)
}
