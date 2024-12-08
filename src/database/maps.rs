use std::{collections::BTreeMap, sync::Arc};

use conduit::Result;

use crate::{Engine, Map};

pub type Maps = BTreeMap<MapsKey, MapsVal>;
pub(crate) type MapsVal = Arc<Map>;
pub(crate) type MapsKey = String;

pub(crate) fn open(db: &Arc<Engine>) -> Result<Maps> { open_list(db, MAPS) }

#[tracing::instrument(name = "maps", level = "debug", skip_all)]
pub(crate) fn open_list(db: &Arc<Engine>, maps: &[&str]) -> Result<Maps> {
	Ok(maps
		.iter()
		.map(|&name| (name.to_owned(), Map::open(db, name).expect("valid column opened")))
		.collect::<Maps>())
}

pub const MAPS: &[&str] = &[
	"alias_roomid",
	"alias_userid",
	"aliasid_alias",
	"backupid_algorithm",
	"backupid_etag",
	"backupkeyid_backup",
	"bannedroomids",
	"disabledroomids",
	"eventid_outlierpdu",
	"eventid_pduid",
	"eventid_shorteventid",
	"global",
	"id_appserviceregistrations",
	"keychangeid_userid",
	"keyid_key",
	"lazyloadedids",
	"mediaid_file",
	"mediaid_user",
	"onetimekeyid_onetimekeys",
	"pduid_pdu",
	"presenceid_presence",
	"publicroomids",
	"readreceiptid_readreceipt",
	"referencedevents",
	"roomid_invitedcount",
	"roomid_inviteviaservers",
	"roomid_joinedcount",
	"roomid_pduleaves",
	"roomid_shortroomid",
	"roomid_shortstatehash",
	"roomserverids",
	"roomsynctoken_shortstatehash",
	"roomuserdataid_accountdata",
	"roomuserid_invitecount",
	"roomuserid_joined",
	"roomuserid_lastprivatereadupdate",
	"roomuserid_leftcount",
	"roomuserid_privateread",
	"roomuseroncejoinedids",
	"roomusertype_roomuserdataid",
	"senderkey_pusher",
	"server_signingkeys",
	"servercurrentevent_data",
	"servername_educount",
	"servernameevent_data",
	"serverroomids",
	"shorteventid_authchain",
	"shorteventid_eventid",
	"shorteventid_shortstatehash",
	"shortstatehash_statediff",
	"shortstatekey_statekey",
	"softfailedeventids",
	"statehash_shortstatehash",
	"statekey_shortstatekey",
	"threadid_userids",
	"todeviceid_events",
	"tofrom_relation",
	"token_userdeviceid",
	"tokenids",
	"url_previews",
	"userdeviceid_metadata",
	"userdeviceid_token",
	"userdevicesessionid_uiaainfo",
	"userdevicetxnid_response",
	"userfilterid_filter",
	"userid_avatarurl",
	"userid_blurhash",
	"userid_devicelistversion",
	"userid_displayname",
	"userid_lastonetimekeyupdate",
	"userid_masterkeyid",
	"userid_password",
	"userid_presenceid",
	"userid_selfsigningkeyid",
	"userid_usersigningkeyid",
	"useridprofilekey_value",
	"openidtoken_expiresatuserid",
	"userroomid_highlightcount",
	"userroomid_invitestate",
	"userroomid_joined",
	"userroomid_leftstate",
	"userroomid_notificationcount",
];
