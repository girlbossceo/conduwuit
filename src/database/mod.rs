mod cork;
mod key_value;
mod kvengine;
mod kvtree;
mod migrations;

#[cfg(feature = "rocksdb")]
mod rocksdb;

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(any(feature = "sqlite", feature = "rocksdb"))]
pub(crate) mod watchers;

use std::{
	collections::{BTreeMap, HashMap, HashSet},
	fs::{self},
	path::Path,
	sync::{Arc, Mutex, RwLock},
	time::Duration,
};

pub(crate) use cork::Cork;
pub(crate) use kvengine::KeyValueDatabaseEngine;
pub(crate) use kvtree::KvTree;
use lru_cache::LruCache;
use ruma::{
	events::{
		push_rules::PushRulesEventContent, room::message::RoomMessageEventContent, GlobalAccountDataEvent,
		GlobalAccountDataEventType,
	},
	push::Ruleset,
	CanonicalJsonValue, OwnedDeviceId, OwnedRoomId, OwnedUserId, UserId,
};
use serde::Deserialize;
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{interval, Instant};
use tracing::{debug, error, warn};

use crate::{
	database::migrations::migrations, service::rooms::timeline::PduCount, services, Config, Error,
	LogLevelReloadHandles, Result, Services, SERVICES,
};

pub(crate) struct KeyValueDatabase {
	db: Arc<dyn KeyValueDatabaseEngine>,

	//pub(crate) globals: globals::Globals,
	pub(crate) global: Arc<dyn KvTree>,
	pub(crate) server_signingkeys: Arc<dyn KvTree>,

	pub(crate) roomid_inviteviaservers: Arc<dyn KvTree>,

	//pub(crate) users: users::Users,
	pub(crate) userid_password: Arc<dyn KvTree>,
	pub(crate) userid_displayname: Arc<dyn KvTree>,
	pub(crate) userid_avatarurl: Arc<dyn KvTree>,
	pub(crate) userid_blurhash: Arc<dyn KvTree>,
	pub(crate) userdeviceid_token: Arc<dyn KvTree>,
	pub(crate) userdeviceid_metadata: Arc<dyn KvTree>, // This is also used to check if a device exists
	pub(crate) userid_devicelistversion: Arc<dyn KvTree>, // DevicelistVersion = u64
	pub(crate) token_userdeviceid: Arc<dyn KvTree>,

	pub(crate) onetimekeyid_onetimekeys: Arc<dyn KvTree>, // OneTimeKeyId = UserId + DeviceKeyId
	pub(crate) userid_lastonetimekeyupdate: Arc<dyn KvTree>, // LastOneTimeKeyUpdate = Count
	pub(crate) keychangeid_userid: Arc<dyn KvTree>,       // KeyChangeId = UserId/RoomId + Count
	pub(crate) keyid_key: Arc<dyn KvTree>,                // KeyId = UserId + KeyId (depends on key type)
	pub(crate) userid_masterkeyid: Arc<dyn KvTree>,
	pub(crate) userid_selfsigningkeyid: Arc<dyn KvTree>,
	pub(crate) userid_usersigningkeyid: Arc<dyn KvTree>,

	pub(crate) userfilterid_filter: Arc<dyn KvTree>, // UserFilterId = UserId + FilterId
	pub(crate) todeviceid_events: Arc<dyn KvTree>,   // ToDeviceId = UserId + DeviceId + Count
	pub(crate) userid_presenceid: Arc<dyn KvTree>,   // UserId => Count
	pub(crate) presenceid_presence: Arc<dyn KvTree>, // Count + UserId => Presence

	//pub(crate) uiaa: uiaa::Uiaa,
	pub(crate) userdevicesessionid_uiaainfo: Arc<dyn KvTree>, // User-interactive authentication
	pub(crate) userdevicesessionid_uiaarequest:
		RwLock<BTreeMap<(OwnedUserId, OwnedDeviceId, String), CanonicalJsonValue>>,

	//pub(crate) edus: RoomEdus,
	pub(crate) readreceiptid_readreceipt: Arc<dyn KvTree>, // ReadReceiptId = RoomId + Count + UserId
	pub(crate) roomuserid_privateread: Arc<dyn KvTree>,    // RoomUserId = Room + User, PrivateRead = Count
	pub(crate) roomuserid_lastprivatereadupdate: Arc<dyn KvTree>, // LastPrivateReadUpdate = Count

	//pub(crate) rooms: rooms::Rooms,
	pub(crate) pduid_pdu: Arc<dyn KvTree>, // PduId = ShortRoomId + Count
	pub(crate) eventid_pduid: Arc<dyn KvTree>,
	pub(crate) roomid_pduleaves: Arc<dyn KvTree>,
	pub(crate) alias_roomid: Arc<dyn KvTree>,
	pub(crate) aliasid_alias: Arc<dyn KvTree>, // AliasId = RoomId + Count
	pub(crate) publicroomids: Arc<dyn KvTree>,

	pub(crate) threadid_userids: Arc<dyn KvTree>, // ThreadId = RoomId + Count

	pub(crate) tokenids: Arc<dyn KvTree>, // TokenId = ShortRoomId + Token + PduIdCount

	/// Participating servers in a room.
	pub(crate) roomserverids: Arc<dyn KvTree>, // RoomServerId = RoomId + ServerName
	pub(crate) serverroomids: Arc<dyn KvTree>, // ServerRoomId = ServerName + RoomId

	pub(crate) userroomid_joined: Arc<dyn KvTree>,
	pub(crate) roomuserid_joined: Arc<dyn KvTree>,
	pub(crate) roomid_joinedcount: Arc<dyn KvTree>,
	pub(crate) roomid_invitedcount: Arc<dyn KvTree>,
	pub(crate) roomuseroncejoinedids: Arc<dyn KvTree>,
	pub(crate) userroomid_invitestate: Arc<dyn KvTree>, // InviteState = Vec<Raw<Pdu>>
	pub(crate) roomuserid_invitecount: Arc<dyn KvTree>, // InviteCount = Count
	pub(crate) userroomid_leftstate: Arc<dyn KvTree>,
	pub(crate) roomuserid_leftcount: Arc<dyn KvTree>,

	pub(crate) disabledroomids: Arc<dyn KvTree>, // Rooms where incoming federation handling is disabled

	pub(crate) bannedroomids: Arc<dyn KvTree>, // Rooms where local users are not allowed to join

	pub(crate) lazyloadedids: Arc<dyn KvTree>, // LazyLoadedIds = UserId + DeviceId + RoomId + LazyLoadedUserId

	pub(crate) userroomid_notificationcount: Arc<dyn KvTree>, // NotifyCount = u64
	pub(crate) userroomid_highlightcount: Arc<dyn KvTree>,    // HightlightCount = u64
	pub(crate) roomuserid_lastnotificationread: Arc<dyn KvTree>, // LastNotificationRead = u64

	/// Remember the current state hash of a room.
	pub(crate) roomid_shortstatehash: Arc<dyn KvTree>,
	pub(crate) roomsynctoken_shortstatehash: Arc<dyn KvTree>,
	/// Remember the state hash at events in the past.
	pub(crate) shorteventid_shortstatehash: Arc<dyn KvTree>,
	pub(crate) statekey_shortstatekey: Arc<dyn KvTree>, /* StateKey = EventType + StateKey, ShortStateKey =
	                                                     * Count */
	pub(crate) shortstatekey_statekey: Arc<dyn KvTree>,

	pub(crate) roomid_shortroomid: Arc<dyn KvTree>,

	pub(crate) shorteventid_eventid: Arc<dyn KvTree>,
	pub(crate) eventid_shorteventid: Arc<dyn KvTree>,

	pub(crate) statehash_shortstatehash: Arc<dyn KvTree>,
	pub(crate) shortstatehash_statediff: Arc<dyn KvTree>, /* StateDiff = parent (or 0) +
	                                                       * (shortstatekey+shorteventid++) + 0_u64 +
	                                                       * (shortstatekey+shorteventid--) */

	pub(crate) shorteventid_authchain: Arc<dyn KvTree>,

	/// RoomId + EventId -> outlier PDU.
	/// Any pdu that has passed the steps 1-8 in the incoming event
	/// /federation/send/txn.
	pub(crate) eventid_outlierpdu: Arc<dyn KvTree>,
	pub(crate) softfailedeventids: Arc<dyn KvTree>,

	/// ShortEventId + ShortEventId -> ().
	pub(crate) tofrom_relation: Arc<dyn KvTree>,
	/// RoomId + EventId -> Parent PDU EventId.
	pub(crate) referencedevents: Arc<dyn KvTree>,

	//pub(crate) account_data: account_data::AccountData,
	pub(crate) roomuserdataid_accountdata: Arc<dyn KvTree>, // RoomUserDataId = Room + User + Count + Type
	pub(crate) roomusertype_roomuserdataid: Arc<dyn KvTree>, // RoomUserType = Room + User + Type

	//pub(crate) media: media::Media,
	pub(crate) mediaid_file: Arc<dyn KvTree>, // MediaId = MXC + WidthHeight + ContentDisposition + ContentType
	pub(crate) url_previews: Arc<dyn KvTree>,
	pub(crate) mediaid_user: Arc<dyn KvTree>,
	//pub(crate) key_backups: key_backups::KeyBackups,
	pub(crate) backupid_algorithm: Arc<dyn KvTree>, // BackupId = UserId + Version(Count)
	pub(crate) backupid_etag: Arc<dyn KvTree>,      // BackupId = UserId + Version(Count)
	pub(crate) backupkeyid_backup: Arc<dyn KvTree>, // BackupKeyId = UserId + Version + RoomId + SessionId

	//pub(crate) transaction_ids: transaction_ids::TransactionIds,
	pub(crate) userdevicetxnid_response: Arc<dyn KvTree>, /* Response can be empty (/sendToDevice) or the event id
	                                                       * (/send) */
	//pub(crate) sending: sending::Sending,
	pub(crate) servername_educount: Arc<dyn KvTree>, // EduCount: Count of last EDU sync
	pub(crate) servernameevent_data: Arc<dyn KvTree>, /* ServernameEvent = (+ / $)SenderKey / ServerName / UserId +
	                                                  * PduId / Id (for edus), Data = EDU content */
	pub(crate) servercurrentevent_data: Arc<dyn KvTree>, /* ServerCurrentEvents = (+ / $)ServerName / UserId + PduId
	                                                      * / Id (for edus), Data = EDU content */

	//pub(crate) appservice: appservice::Appservice,
	pub(crate) id_appserviceregistrations: Arc<dyn KvTree>,

	//pub(crate) pusher: pusher::PushData,
	pub(crate) senderkey_pusher: Arc<dyn KvTree>,

	pub(crate) auth_chain_cache: Mutex<LruCache<Vec<u64>, Arc<[u64]>>>,
	pub(crate) our_real_users_cache: RwLock<HashMap<OwnedRoomId, Arc<HashSet<OwnedUserId>>>>,
	pub(crate) appservice_in_room_cache: RwLock<HashMap<OwnedRoomId, HashMap<String, bool>>>,
	pub(crate) lasttimelinecount_cache: Mutex<HashMap<OwnedRoomId, PduCount>>,
}

#[derive(Deserialize)]
struct CheckForUpdatesResponseEntry {
	id: u64,
	date: String,
	message: String,
}
#[derive(Deserialize)]
struct CheckForUpdatesResponse {
	updates: Vec<CheckForUpdatesResponseEntry>,
}

impl KeyValueDatabase {
	/// Load an existing database or create a new one.
	#[allow(clippy::too_many_lines)]
	pub(crate) async fn load_or_create(config: Config, tracing_reload_handler: LogLevelReloadHandles) -> Result<()> {
		Self::check_db_setup(&config)?;

		if !Path::new(&config.database_path).exists() {
			debug!("Database path does not exist, assuming this is a new setup and creating it");
			fs::create_dir_all(&config.database_path).map_err(|e| {
				error!("Failed to create database path: {e}");
				Error::bad_config(
					"Database folder doesn't exists and couldn't be created (e.g. due to missing permissions). Please \
					 create the database folder yourself or allow conduwuit the permissions to create directories and \
					 files.",
				)
			})?;
		}

		let builder: Arc<dyn KeyValueDatabaseEngine> = match &*config.database_backend {
			"sqlite" => {
				debug!("Got sqlite database backend");
				#[cfg(not(feature = "sqlite"))]
				return Err(Error::bad_config("Database backend not found."));
				#[cfg(feature = "sqlite")]
				Arc::new(Arc::<sqlite::Engine>::open(&config)?)
			},
			"rocksdb" => {
				debug!("Got rocksdb database backend");
				#[cfg(not(feature = "rocksdb"))]
				return Err(Error::bad_config("Database backend not found."));
				#[cfg(feature = "rocksdb")]
				Arc::new(Arc::<rocksdb::Engine>::open(&config)?)
			},
			_ => {
				return Err(Error::bad_config(
					"Database backend not found. sqlite (not recommended) and rocksdb are the only supported backends.",
				));
			},
		};

		let db_raw = Box::new(Self {
			db: builder.clone(),
			userid_password: builder.open_tree("userid_password")?,
			userid_displayname: builder.open_tree("userid_displayname")?,
			userid_avatarurl: builder.open_tree("userid_avatarurl")?,
			userid_blurhash: builder.open_tree("userid_blurhash")?,
			userdeviceid_token: builder.open_tree("userdeviceid_token")?,
			userdeviceid_metadata: builder.open_tree("userdeviceid_metadata")?,
			userid_devicelistversion: builder.open_tree("userid_devicelistversion")?,
			token_userdeviceid: builder.open_tree("token_userdeviceid")?,
			onetimekeyid_onetimekeys: builder.open_tree("onetimekeyid_onetimekeys")?,
			userid_lastonetimekeyupdate: builder.open_tree("userid_lastonetimekeyupdate")?,
			keychangeid_userid: builder.open_tree("keychangeid_userid")?,
			keyid_key: builder.open_tree("keyid_key")?,
			userid_masterkeyid: builder.open_tree("userid_masterkeyid")?,
			userid_selfsigningkeyid: builder.open_tree("userid_selfsigningkeyid")?,
			userid_usersigningkeyid: builder.open_tree("userid_usersigningkeyid")?,
			userfilterid_filter: builder.open_tree("userfilterid_filter")?,
			todeviceid_events: builder.open_tree("todeviceid_events")?,
			userid_presenceid: builder.open_tree("userid_presenceid")?,
			presenceid_presence: builder.open_tree("presenceid_presence")?,

			userdevicesessionid_uiaainfo: builder.open_tree("userdevicesessionid_uiaainfo")?,
			userdevicesessionid_uiaarequest: RwLock::new(BTreeMap::new()),
			readreceiptid_readreceipt: builder.open_tree("readreceiptid_readreceipt")?,
			roomuserid_privateread: builder.open_tree("roomuserid_privateread")?, // "Private" read receipt
			roomuserid_lastprivatereadupdate: builder.open_tree("roomuserid_lastprivatereadupdate")?,
			pduid_pdu: builder.open_tree("pduid_pdu")?,
			eventid_pduid: builder.open_tree("eventid_pduid")?,
			roomid_pduleaves: builder.open_tree("roomid_pduleaves")?,

			alias_roomid: builder.open_tree("alias_roomid")?,
			aliasid_alias: builder.open_tree("aliasid_alias")?,
			publicroomids: builder.open_tree("publicroomids")?,

			threadid_userids: builder.open_tree("threadid_userids")?,

			tokenids: builder.open_tree("tokenids")?,

			roomserverids: builder.open_tree("roomserverids")?,
			serverroomids: builder.open_tree("serverroomids")?,
			userroomid_joined: builder.open_tree("userroomid_joined")?,
			roomuserid_joined: builder.open_tree("roomuserid_joined")?,
			roomid_joinedcount: builder.open_tree("roomid_joinedcount")?,
			roomid_invitedcount: builder.open_tree("roomid_invitedcount")?,
			roomuseroncejoinedids: builder.open_tree("roomuseroncejoinedids")?,
			userroomid_invitestate: builder.open_tree("userroomid_invitestate")?,
			roomuserid_invitecount: builder.open_tree("roomuserid_invitecount")?,
			userroomid_leftstate: builder.open_tree("userroomid_leftstate")?,
			roomuserid_leftcount: builder.open_tree("roomuserid_leftcount")?,

			disabledroomids: builder.open_tree("disabledroomids")?,

			bannedroomids: builder.open_tree("bannedroomids")?,

			lazyloadedids: builder.open_tree("lazyloadedids")?,

			userroomid_notificationcount: builder.open_tree("userroomid_notificationcount")?,
			userroomid_highlightcount: builder.open_tree("userroomid_highlightcount")?,
			roomuserid_lastnotificationread: builder.open_tree("userroomid_highlightcount")?,

			statekey_shortstatekey: builder.open_tree("statekey_shortstatekey")?,
			shortstatekey_statekey: builder.open_tree("shortstatekey_statekey")?,

			shorteventid_authchain: builder.open_tree("shorteventid_authchain")?,

			roomid_shortroomid: builder.open_tree("roomid_shortroomid")?,

			shortstatehash_statediff: builder.open_tree("shortstatehash_statediff")?,
			eventid_shorteventid: builder.open_tree("eventid_shorteventid")?,
			shorteventid_eventid: builder.open_tree("shorteventid_eventid")?,
			shorteventid_shortstatehash: builder.open_tree("shorteventid_shortstatehash")?,
			roomid_shortstatehash: builder.open_tree("roomid_shortstatehash")?,
			roomsynctoken_shortstatehash: builder.open_tree("roomsynctoken_shortstatehash")?,
			statehash_shortstatehash: builder.open_tree("statehash_shortstatehash")?,

			eventid_outlierpdu: builder.open_tree("eventid_outlierpdu")?,
			softfailedeventids: builder.open_tree("softfailedeventids")?,

			tofrom_relation: builder.open_tree("tofrom_relation")?,
			referencedevents: builder.open_tree("referencedevents")?,
			roomuserdataid_accountdata: builder.open_tree("roomuserdataid_accountdata")?,
			roomusertype_roomuserdataid: builder.open_tree("roomusertype_roomuserdataid")?,
			mediaid_file: builder.open_tree("mediaid_file")?,
			url_previews: builder.open_tree("url_previews")?,
			mediaid_user: builder.open_tree("mediaid_user")?,
			backupid_algorithm: builder.open_tree("backupid_algorithm")?,
			backupid_etag: builder.open_tree("backupid_etag")?,
			backupkeyid_backup: builder.open_tree("backupkeyid_backup")?,
			userdevicetxnid_response: builder.open_tree("userdevicetxnid_response")?,
			servername_educount: builder.open_tree("servername_educount")?,
			servernameevent_data: builder.open_tree("servernameevent_data")?,
			servercurrentevent_data: builder.open_tree("servercurrentevent_data")?,
			id_appserviceregistrations: builder.open_tree("id_appserviceregistrations")?,
			senderkey_pusher: builder.open_tree("senderkey_pusher")?,
			global: builder.open_tree("global")?,
			server_signingkeys: builder.open_tree("server_signingkeys")?,

			roomid_inviteviaservers: builder.open_tree("roomid_inviteviaservers")?,

			auth_chain_cache: Mutex::new(LruCache::new(
				(f64::from(config.auth_chain_cache_capacity) * config.conduit_cache_capacity_modifier) as usize,
			)),
			our_real_users_cache: RwLock::new(HashMap::new()),
			appservice_in_room_cache: RwLock::new(HashMap::new()),
			lasttimelinecount_cache: Mutex::new(HashMap::new()),
		});

		let db = Box::leak(db_raw);

		let services_raw = Box::new(Services::build(db, &config, tracing_reload_handler)?);

		// This is the first and only time we initialize the SERVICE static
		*SERVICES.write().unwrap() = Some(Box::leak(services_raw));

		migrations(db, &config).await?;

		services().admin.start_handler();

		// Set emergency access for the conduit user
		match set_emergency_access() {
			Ok(pwd_set) => {
				if pwd_set {
					warn!(
						"The Conduit account emergency password is set! Please unset it as soon as you finish admin \
						 account recovery!"
					);
					services()
						.admin
						.send_message(RoomMessageEventContent::text_plain(
							"The Conduit account emergency password is set! Please unset it as soon as you finish \
							 admin account recovery!",
						))
						.await;
				}
			},
			Err(e) => {
				error!("Could not set the configured emergency password for the conduit user: {}", e);
			},
		};

		services().sending.start_handler();

		if config.allow_local_presence {
			services().presence.start_handler();
		}

		Self::start_cleanup_task().await;
		if services().globals.allow_check_for_updates() {
			Self::start_check_for_updates_task().await;
		}

		Ok(())
	}

	fn check_db_setup(config: &Config) -> Result<()> {
		let path = Path::new(&config.database_path);

		let sqlite_exists = path.join("conduit.db").exists();
		let rocksdb_exists = path.join("IDENTITY").exists();

		let mut count = 0;

		if sqlite_exists {
			count += 1;
		}

		if rocksdb_exists {
			count += 1;
		}

		if count > 1 {
			warn!("Multiple databases at database_path detected");
			return Ok(());
		}

		if sqlite_exists && config.database_backend != "sqlite" {
			return Err(Error::bad_config(
				"Found sqlite at database_path, but is not specified in config.",
			));
		}

		if rocksdb_exists && config.database_backend != "rocksdb" {
			return Err(Error::bad_config(
				"Found rocksdb at database_path, but is not specified in config.",
			));
		}

		Ok(())
	}

	#[tracing::instrument]
	async fn start_check_for_updates_task() {
		let timer_interval = Duration::from_secs(7200); // 2 hours

		tokio::spawn(async move {
			let mut i = interval(timer_interval);

			loop {
				tokio::select! {
					_ = i.tick() => {
						debug!(target: "start_check_for_updates_task", "Timer ticked");
					},
				}

				_ = Self::try_handle_updates().await;
			}
		});
	}

	async fn try_handle_updates() -> Result<()> {
		let response = services()
			.globals
			.client
			.default
			.get("https://pupbrain.dev/check-for-updates/stable")
			.send()
			.await?;

		let response = serde_json::from_str::<CheckForUpdatesResponse>(&response.text().await?).map_err(|e| {
			error!("Bad check for updates response: {e}");
			Error::BadServerResponse("Bad version check response")
		})?;

		let mut last_update_id = services().globals.last_check_for_updates_id()?;
		for update in response.updates {
			last_update_id = last_update_id.max(update.id);
			if update.id > services().globals.last_check_for_updates_id()? {
				error!("{}", update.message);
				services()
					.admin
					.send_message(RoomMessageEventContent::text_plain(format!(
						"@room: the following is a message from the conduwuit puppy. it was sent on '{}':\n\n{}",
						update.date, update.message
					)))
					.await;
			}
		}
		services()
			.globals
			.update_check_for_updates_id(last_update_id)?;

		Ok(())
	}

	#[tracing::instrument]
	async fn start_cleanup_task() {
		let timer_interval = Duration::from_secs(u64::from(services().globals.config.cleanup_second_interval));

		tokio::spawn(async move {
			let mut i = interval(timer_interval);

			#[cfg(unix)]
			let mut hangup = signal(SignalKind::hangup()).expect("Failed to register SIGHUP signal receiver");
			#[cfg(unix)]
			let mut ctrl_c = signal(SignalKind::interrupt()).expect("Failed to register SIGINT signal receiver");
			#[cfg(unix)]
			let mut terminate = signal(SignalKind::terminate()).expect("Failed to register SIGTERM signal receiver");

			loop {
				#[cfg(unix)]
				tokio::select! {
					_ = i.tick() => {
						debug!(target: "database-cleanup", "Timer ticked");
					}
					_ = hangup.recv() => {
						debug!(target: "database-cleanup","Received SIGHUP");
					}
					_ = ctrl_c.recv() => {
						debug!(target: "database-cleanup", "Received Ctrl+C");
					}
					_ = terminate.recv() => {
						debug!(target: "database-cleanup","Received SIGTERM");
					}
				}

				#[cfg(not(unix))]
				{
					i.tick().await;
					debug!(target: "database-cleanup", "Timer ticked")
				}

				Self::perform_cleanup();
			}
		});
	}

	fn perform_cleanup() {
		if !services().globals.config.rocksdb_periodic_cleanup {
			return;
		}

		let start = Instant::now();
		if let Err(e) = services().globals.cleanup() {
			error!(target: "database-cleanup", "Ran into an error during cleanup: {}", e);
		} else {
			debug!(target: "database-cleanup", "Finished cleanup in {:#?}.", start.elapsed());
		}
	}

	#[allow(dead_code)]
	fn flush(&self) -> Result<()> {
		let start = std::time::Instant::now();

		let res = self.db.flush();

		debug!("flush: took {:?}", start.elapsed());

		res
	}
}

/// Sets the emergency password and push rules for the @conduit account in case
/// emergency password is set
fn set_emergency_access() -> Result<bool> {
	let conduit_user = UserId::parse_with_server_name("conduit", services().globals.server_name())
		.expect("@conduit:server_name is a valid UserId");

	services()
		.users
		.set_password(&conduit_user, services().globals.emergency_password().as_deref())?;

	let (ruleset, res) = match services().globals.emergency_password() {
		Some(_) => (Ruleset::server_default(&conduit_user), Ok(true)),
		None => (Ruleset::new(), Ok(false)),
	};

	services().account_data.update(
		None,
		&conduit_user,
		GlobalAccountDataEventType::PushRules.to_string().into(),
		&serde_json::to_value(&GlobalAccountDataEvent {
			content: PushRulesEventContent {
				global: ruleset,
			},
		})
		.expect("to json value always works"),
	)?;

	res
}
