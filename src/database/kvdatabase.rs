use std::{
	collections::{BTreeMap, HashMap, HashSet},
	path::Path,
	sync::{Arc, Mutex, RwLock},
};

use conduit::{Config, Error, PduCount, Result, Server};
use lru_cache::LruCache;
use ruma::{CanonicalJsonValue, OwnedDeviceId, OwnedRoomId, OwnedUserId};
use tracing::debug;

use crate::{KeyValueDatabaseEngine, KvTree};

pub struct KeyValueDatabase {
	pub db: Arc<dyn KeyValueDatabaseEngine>,

	//pub globals: globals::Globals,
	pub global: Arc<dyn KvTree>,
	pub server_signingkeys: Arc<dyn KvTree>,

	pub roomid_inviteviaservers: Arc<dyn KvTree>,

	//pub users: users::Users,
	pub userid_password: Arc<dyn KvTree>,
	pub userid_displayname: Arc<dyn KvTree>,
	pub userid_avatarurl: Arc<dyn KvTree>,
	pub userid_blurhash: Arc<dyn KvTree>,
	pub userdeviceid_token: Arc<dyn KvTree>,
	pub userdeviceid_metadata: Arc<dyn KvTree>, // This is also used to check if a device exists
	pub userid_devicelistversion: Arc<dyn KvTree>, // DevicelistVersion = u64
	pub token_userdeviceid: Arc<dyn KvTree>,

	pub onetimekeyid_onetimekeys: Arc<dyn KvTree>, // OneTimeKeyId = UserId + DeviceKeyId
	pub userid_lastonetimekeyupdate: Arc<dyn KvTree>, // LastOneTimeKeyUpdate = Count
	pub keychangeid_userid: Arc<dyn KvTree>,       // KeyChangeId = UserId/RoomId + Count
	pub keyid_key: Arc<dyn KvTree>,                // KeyId = UserId + KeyId (depends on key type)
	pub userid_masterkeyid: Arc<dyn KvTree>,
	pub userid_selfsigningkeyid: Arc<dyn KvTree>,
	pub userid_usersigningkeyid: Arc<dyn KvTree>,

	pub userfilterid_filter: Arc<dyn KvTree>, // UserFilterId = UserId + FilterId
	pub todeviceid_events: Arc<dyn KvTree>,   // ToDeviceId = UserId + DeviceId + Count
	pub userid_presenceid: Arc<dyn KvTree>,   // UserId => Count
	pub presenceid_presence: Arc<dyn KvTree>, // Count + UserId => Presence

	//pub uiaa: uiaa::Uiaa,
	pub userdevicesessionid_uiaainfo: Arc<dyn KvTree>, // User-interactive authentication
	pub userdevicesessionid_uiaarequest: RwLock<BTreeMap<(OwnedUserId, OwnedDeviceId, String), CanonicalJsonValue>>,

	//pub edus: RoomEdus,
	pub readreceiptid_readreceipt: Arc<dyn KvTree>, // ReadReceiptId = RoomId + Count + UserId
	pub roomuserid_privateread: Arc<dyn KvTree>,    // RoomUserId = Room + User, PrivateRead = Count
	pub roomuserid_lastprivatereadupdate: Arc<dyn KvTree>, // LastPrivateReadUpdate = Count

	//pub rooms: rooms::Rooms,
	pub pduid_pdu: Arc<dyn KvTree>, // PduId = ShortRoomId + Count
	pub eventid_pduid: Arc<dyn KvTree>,
	pub roomid_pduleaves: Arc<dyn KvTree>,
	pub alias_roomid: Arc<dyn KvTree>,
	pub aliasid_alias: Arc<dyn KvTree>, // AliasId = RoomId + Count
	pub publicroomids: Arc<dyn KvTree>,

	pub threadid_userids: Arc<dyn KvTree>, // ThreadId = RoomId + Count

	pub tokenids: Arc<dyn KvTree>, // TokenId = ShortRoomId + Token + PduIdCount

	/// Participating servers in a room.
	pub roomserverids: Arc<dyn KvTree>, // RoomServerId = RoomId + ServerName
	pub serverroomids: Arc<dyn KvTree>, // ServerRoomId = ServerName + RoomId

	pub userroomid_joined: Arc<dyn KvTree>,
	pub roomuserid_joined: Arc<dyn KvTree>,
	pub roomid_joinedcount: Arc<dyn KvTree>,
	pub roomid_invitedcount: Arc<dyn KvTree>,
	pub roomuseroncejoinedids: Arc<dyn KvTree>,
	pub userroomid_invitestate: Arc<dyn KvTree>, // InviteState = Vec<Raw<Pdu>>
	pub roomuserid_invitecount: Arc<dyn KvTree>, // InviteCount = Count
	pub userroomid_leftstate: Arc<dyn KvTree>,
	pub roomuserid_leftcount: Arc<dyn KvTree>,

	pub disabledroomids: Arc<dyn KvTree>, // Rooms where incoming federation handling is disabled

	pub bannedroomids: Arc<dyn KvTree>, // Rooms where local users are not allowed to join

	pub lazyloadedids: Arc<dyn KvTree>, // LazyLoadedIds = UserId + DeviceId + RoomId + LazyLoadedUserId

	pub userroomid_notificationcount: Arc<dyn KvTree>, // NotifyCount = u64
	pub userroomid_highlightcount: Arc<dyn KvTree>,    // HightlightCount = u64
	pub roomuserid_lastnotificationread: Arc<dyn KvTree>, // LastNotificationRead = u64

	/// Remember the current state hash of a room.
	pub roomid_shortstatehash: Arc<dyn KvTree>,
	pub roomsynctoken_shortstatehash: Arc<dyn KvTree>,
	/// Remember the state hash at events in the past.
	pub shorteventid_shortstatehash: Arc<dyn KvTree>,
	pub statekey_shortstatekey: Arc<dyn KvTree>, /* StateKey = EventType + StateKey, ShortStateKey =
	                                              * Count */
	pub shortstatekey_statekey: Arc<dyn KvTree>,

	pub roomid_shortroomid: Arc<dyn KvTree>,

	pub shorteventid_eventid: Arc<dyn KvTree>,
	pub eventid_shorteventid: Arc<dyn KvTree>,

	pub statehash_shortstatehash: Arc<dyn KvTree>,
	pub shortstatehash_statediff: Arc<dyn KvTree>, /* StateDiff = parent (or 0) +
	                                                * (shortstatekey+shorteventid++) + 0_u64 +
	                                                * (shortstatekey+shorteventid--) */

	pub shorteventid_authchain: Arc<dyn KvTree>,

	/// RoomId + EventId -> outlier PDU.
	/// Any pdu that has passed the steps 1-8 in the incoming event
	/// /federation/send/txn.
	pub eventid_outlierpdu: Arc<dyn KvTree>,
	pub softfailedeventids: Arc<dyn KvTree>,

	/// ShortEventId + ShortEventId -> ().
	pub tofrom_relation: Arc<dyn KvTree>,
	/// RoomId + EventId -> Parent PDU EventId.
	pub referencedevents: Arc<dyn KvTree>,

	//pub account_data: account_data::AccountData,
	pub roomuserdataid_accountdata: Arc<dyn KvTree>, // RoomUserDataId = Room + User + Count + Type
	pub roomusertype_roomuserdataid: Arc<dyn KvTree>, // RoomUserType = Room + User + Type

	//pub media: media::Media,
	pub mediaid_file: Arc<dyn KvTree>, // MediaId = MXC + WidthHeight + ContentDisposition + ContentType
	pub url_previews: Arc<dyn KvTree>,
	pub mediaid_user: Arc<dyn KvTree>,
	//pub key_backups: key_backups::KeyBackups,
	pub backupid_algorithm: Arc<dyn KvTree>, // BackupId = UserId + Version(Count)
	pub backupid_etag: Arc<dyn KvTree>,      // BackupId = UserId + Version(Count)
	pub backupkeyid_backup: Arc<dyn KvTree>, // BackupKeyId = UserId + Version + RoomId + SessionId

	//pub transaction_ids: transaction_ids::TransactionIds,
	pub userdevicetxnid_response: Arc<dyn KvTree>, /* Response can be empty (/sendToDevice) or the event id
	                                                * (/send) */
	//pub sending: sending::Sending,
	pub servername_educount: Arc<dyn KvTree>, // EduCount: Count of last EDU sync
	pub servernameevent_data: Arc<dyn KvTree>, /* ServernameEvent = (+ / $)SenderKey / ServerName / UserId +
	                                           * PduId / Id (for edus), Data = EDU content */
	pub servercurrentevent_data: Arc<dyn KvTree>, /* ServerCurrentEvents = (+ / $)ServerName / UserId + PduId
	                                               * / Id (for edus), Data = EDU content */

	//pub appservice: appservice::Appservice,
	pub id_appserviceregistrations: Arc<dyn KvTree>,

	//pub pusher: pusher::PushData,
	pub senderkey_pusher: Arc<dyn KvTree>,

	pub auth_chain_cache: Mutex<LruCache<Vec<u64>, Arc<[u64]>>>,
	pub our_real_users_cache: RwLock<HashMap<OwnedRoomId, Arc<HashSet<OwnedUserId>>>>,
	pub appservice_in_room_cache: RwLock<HashMap<OwnedRoomId, HashMap<String, bool>>>,
	pub lasttimelinecount_cache: Mutex<HashMap<OwnedRoomId, PduCount>>,
}

impl KeyValueDatabase {
	/// Load an existing database or create a new one.
	#[allow(clippy::too_many_lines)]
	pub async fn load_or_create(server: &Arc<Server>) -> Result<KeyValueDatabase> {
		let config = &server.config;
		check_db_setup(config)?;
		let builder = build(config)?;
		Ok(Self {
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
		})
	}
}

fn build(config: &Config) -> Result<Arc<dyn KeyValueDatabaseEngine>> {
	match &*config.database_backend {
		"sqlite" => {
			debug!("Got sqlite database backend");
			#[cfg(not(feature = "sqlite"))]
			return Err(Error::bad_config("Database backend not found."));
			#[cfg(feature = "sqlite")]
			Ok(Arc::new(Arc::<crate::sqlite::Engine>::open(config)?))
		},
		"rocksdb" => {
			debug!("Got rocksdb database backend");
			#[cfg(not(feature = "rocksdb"))]
			return Err(Error::bad_config("Database backend not found."));
			#[cfg(feature = "rocksdb")]
			Ok(Arc::new(Arc::<crate::rocksdb::Engine>::open(config)?))
		},
		_ => Err(Error::bad_config(
			"Database backend not found. sqlite (not recommended) and rocksdb are the only supported backends.",
		)),
	}
}

fn check_db_setup(config: &Config) -> Result<()> {
	let path = Path::new(&config.database_path);

	let sqlite_exists = path.join("conduit.db").exists();
	let rocksdb_exists = path.join("IDENTITY").exists();

	if sqlite_exists && rocksdb_exists {
		return Err(Error::bad_config("Multiple databases at database_path detected."));
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
