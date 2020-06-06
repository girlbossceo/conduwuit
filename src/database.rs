pub(self) mod account_data;
pub(self) mod global_edus;
pub(self) mod globals;
pub(self) mod media;
pub(self) mod rooms;
pub(self) mod uiaa;
pub(self) mod users;

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
    pub fn try_remove(server_name: &str) {
        let mut path = ProjectDirs::from("xyz", "koesters", "conduit")
            .unwrap()
            .data_dir()
            .to_path_buf();
        path.push(server_name);
        let _ = remove_dir_all(path);
    }

    /// Load an existing database or create a new one.
    pub fn load_or_create(config: &Config) -> Self {
        let server_name = config.get_str("server_name").unwrap_or("localhost");

        let path = config
            .get_str("database_path")
            .map(|x| x.to_owned())
            .unwrap_or_else(|_| {
                let path = ProjectDirs::from("xyz", "koesters", "conduit")
                    .unwrap()
                    .data_dir()
                    .join(server_name);
                path.to_str().unwrap().to_owned()
            });

        let db = sled::open(&path).unwrap();
        info!("Opened sled database at {}", path);

        Self {
            globals: globals::Globals::load(db.open_tree("global").unwrap(), config),
            users: users::Users {
                userid_password: db.open_tree("userid_password").unwrap(),
                userid_displayname: db.open_tree("userid_displayname").unwrap(),
                userid_avatarurl: db.open_tree("userid_avatarurl").unwrap(),
                userdeviceid_token: db.open_tree("userdeviceid_token").unwrap(),
                userdeviceid_metadata: db.open_tree("userdeviceid_metadata").unwrap(),
                token_userdeviceid: db.open_tree("token_userdeviceid").unwrap(),
                onetimekeyid_onetimekeys: db.open_tree("onetimekeyid_onetimekeys").unwrap(),
                userdeviceid_devicekeys: db.open_tree("userdeviceid_devicekeys").unwrap(),
                devicekeychangeid_userid: db.open_tree("devicekeychangeid_userid").unwrap(),
                todeviceid_events: db.open_tree("todeviceid_events").unwrap(),
            },
            uiaa: uiaa::Uiaa {
                userdeviceid_uiaainfo: db.open_tree("userdeviceid_uiaainfo").unwrap(),
            },
            rooms: rooms::Rooms {
                edus: rooms::RoomEdus {
                    roomuserid_lastread: db.open_tree("roomuserid_lastread").unwrap(), // "Private" read receipt
                    roomlatestid_roomlatest: db.open_tree("roomlatestid_roomlatest").unwrap(), // Read receipts
                    roomactiveid_userid: db.open_tree("roomactiveid_userid").unwrap(), // Typing notifs
                    roomid_lastroomactiveupdate: db
                        .open_tree("roomid_lastroomactiveupdate")
                        .unwrap(),
                },
                pduid_pdu: db.open_tree("pduid_pdu").unwrap(),
                eventid_pduid: db.open_tree("eventid_pduid").unwrap(),
                roomid_pduleaves: db.open_tree("roomid_pduleaves").unwrap(),
                roomstateid_pdu: db.open_tree("roomstateid_pdu").unwrap(),

                alias_roomid: db.open_tree("alias_roomid").unwrap(),
                aliasid_alias: db.open_tree("alias_roomid").unwrap(),
                publicroomids: db.open_tree("publicroomids").unwrap(),

                userroomid_joined: db.open_tree("userroomid_joined").unwrap(),
                roomuserid_joined: db.open_tree("roomuserid_joined").unwrap(),
                userroomid_invited: db.open_tree("userroomid_invited").unwrap(),
                roomuserid_invited: db.open_tree("roomuserid_invited").unwrap(),
                userroomid_left: db.open_tree("userroomid_left").unwrap(),
            },
            account_data: account_data::AccountData {
                roomuserdataid_accountdata: db.open_tree("roomuserdataid_accountdata").unwrap(),
            },
            global_edus: global_edus::GlobalEdus {
                presenceid_presence: db.open_tree("presenceid_presence").unwrap(), // Presence
            },
            media: media::Media {
                mediaid_file: db.open_tree("mediaid_file").unwrap(),
            },
            _db: db,
        }
    }
}
