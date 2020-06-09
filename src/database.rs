pub(self) mod account_data;
pub(self) mod global_edus;
pub(self) mod globals;
pub(self) mod media;
pub(self) mod rooms;
pub(self) mod uiaa;
pub(self) mod users;

use crate::{Error, Result};
use directories::ProjectDirs;
use log::info;
use std::fs::remove_dir_all;

use rocket::Config;

pub struct Database {
    pub globals: globals::Globals,
    pub users: users::Users,
    pub uiaa: uiaa::Uiaa,
    pub rooms: rooms::Rooms,
    pub account_data: account_data::AccountData,
    pub global_edus: global_edus::GlobalEdus,
    pub media: media::Media,
    pub _db: sled::Db,
}

impl Database {
    /// Tries to remove the old database but ignores all errors.
    pub fn try_remove(server_name: &str) -> Result<()> {
        let mut path = ProjectDirs::from("xyz", "koesters", "conduit")
            .ok_or(Error::BadConfig(
                "The OS didn't return a valid home directory path.",
            ))?
            .data_dir()
            .to_path_buf();
        path.push(server_name);
        let _ = remove_dir_all(path);

        Ok(())
    }

    /// Load an existing database or create a new one.
    pub fn load_or_create(config: &Config) -> Result<Self> {
        let server_name = config.get_str("server_name").unwrap_or("localhost");

        let path = config
            .get_str("database_path")
            .map(|x| Ok::<_, Error>(x.to_owned()))
            .unwrap_or_else(|_| {
                let path = ProjectDirs::from("xyz", "koesters", "conduit")
                    .ok_or(Error::BadConfig(
                        "The OS didn't return a valid home directory path.",
                    ))?
                    .data_dir()
                    .join(server_name);

                Ok(path
                    .to_str()
                    .ok_or(Error::BadConfig("Database path contains invalid unicode."))?
                    .to_owned())
            })?;

        let db = sled::open(&path)?;
        info!("Opened sled database at {}", path);

        Ok(Self {
            globals: globals::Globals::load(db.open_tree("global")?, config)?,
            users: users::Users {
                userid_password: db.open_tree("userid_password")?,
                userid_displayname: db.open_tree("userid_displayname")?,
                userid_avatarurl: db.open_tree("userid_avatarurl")?,
                userdeviceid_token: db.open_tree("userdeviceid_token")?,
                userdeviceid_metadata: db.open_tree("userdeviceid_metadata")?,
                token_userdeviceid: db.open_tree("token_userdeviceid")?,
                onetimekeyid_onetimekeys: db.open_tree("onetimekeyid_onetimekeys")?,
                userdeviceid_devicekeys: db.open_tree("userdeviceid_devicekeys")?,
                devicekeychangeid_userid: db.open_tree("devicekeychangeid_userid")?,
                todeviceid_events: db.open_tree("todeviceid_events")?,
            },
            uiaa: uiaa::Uiaa {
                userdeviceid_uiaainfo: db.open_tree("userdeviceid_uiaainfo")?,
            },
            rooms: rooms::Rooms {
                edus: rooms::RoomEdus {
                    roomuserid_lastread: db.open_tree("roomuserid_lastread")?, // "Private" read receipt
                    roomlatestid_roomlatest: db.open_tree("roomlatestid_roomlatest")?, // Read receipts
                    roomactiveid_userid: db.open_tree("roomactiveid_userid")?, // Typing notifs
                    roomid_lastroomactiveupdate: db.open_tree("roomid_lastroomactiveupdate")?,
                },
                pduid_pdu: db.open_tree("pduid_pdu")?,
                eventid_pduid: db.open_tree("eventid_pduid")?,
                roomid_pduleaves: db.open_tree("roomid_pduleaves")?,
                roomstateid_pdu: db.open_tree("roomstateid_pdu")?,

                alias_roomid: db.open_tree("alias_roomid")?,
                aliasid_alias: db.open_tree("alias_roomid")?,
                publicroomids: db.open_tree("publicroomids")?,

                userroomid_joined: db.open_tree("userroomid_joined")?,
                roomuserid_joined: db.open_tree("roomuserid_joined")?,
                userroomid_invited: db.open_tree("userroomid_invited")?,
                roomuserid_invited: db.open_tree("roomuserid_invited")?,
                userroomid_left: db.open_tree("userroomid_left")?,
            },
            account_data: account_data::AccountData {
                roomuserdataid_accountdata: db.open_tree("roomuserdataid_accountdata")?,
            },
            global_edus: global_edus::GlobalEdus {
                presenceid_presence: db.open_tree("presenceid_presence")?, // Presence
            },
            media: media::Media {
                mediaid_file: db.open_tree("mediaid_file")?,
            },
            _db: db,
        })
    }
}
