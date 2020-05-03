pub(self) mod account_data;
pub(self) mod globals;
pub(self) mod rooms;
pub(self) mod users;

use directories::ProjectDirs;
use std::fs::remove_dir_all;

pub struct Database {
    pub globals: globals::Globals,
    pub users: users::Users,
    pub rooms: rooms::Rooms,
    pub account_data: account_data::AccountData,
    //pub globalallid_globalall: sled::Tree, // ToDevice, GlobalAllId = UserId + Count
    //pub globallatestid_globallatest: sled::Tree, // Presence, GlobalLatestId = Count + Type + UserId
    pub _db: sled::Db,
}

impl Database {
    /// Tries to remove the old database but ignores all errors.
    pub fn try_remove(hostname: &str) {
        let mut path = ProjectDirs::from("xyz", "koesters", "conduit")
            .unwrap()
            .data_dir()
            .to_path_buf();
        path.push(hostname);
        let _ = remove_dir_all(path);
    }

    /// Load an existing database or create a new one.
    pub fn load_or_create(hostname: &str) -> Self {
        let mut path = ProjectDirs::from("xyz", "koesters", "conduit")
            .unwrap()
            .data_dir()
            .to_path_buf();
        path.push(hostname);
        let db = sled::open(&path).unwrap();

        Self {
            globals: globals::Globals::load(db.open_tree("global").unwrap(), hostname.to_owned()),
            users: users::Users {
                userid_password: db.open_tree("userid_password").unwrap(),
                userdeviceid: db.open_tree("userdeviceid").unwrap(),
                userid_displayname: db.open_tree("userid_displayname").unwrap(),
                userid_avatarurl: db.open_tree("userid_avatarurl").unwrap(),
                userdeviceid_token: db.open_tree("userdeviceid_token").unwrap(),
                token_userid: db.open_tree("token_userid").unwrap(),
            },
            rooms: rooms::Rooms {
                edus: rooms::RoomEdus {
                    roomuserid_lastread: db.open_tree("roomuserid_lastread").unwrap(),
                    roomlatestid_roomlatest: db.open_tree("roomlatestid_roomlatest").unwrap(),
                    roomactiveid_roomactive: db.open_tree("roomactiveid_roomactive").unwrap(),
                },
                pduid_pdu: db.open_tree("pduid_pdu").unwrap(),
                eventid_pduid: db.open_tree("eventid_pduid").unwrap(),
                roomid_pduleaves: db.open_tree("roomid_pduleaves").unwrap(),
                roomstateid_pdu: db.open_tree("roomstateid_pdu").unwrap(),

                userroomid_joined: db.open_tree("userroomid_joined").unwrap(),
                roomuserid_joined: db.open_tree("roomuserid_joined").unwrap(),
                userroomid_invited: db.open_tree("userroomid_invited").unwrap(),
                roomuserid_invited: db.open_tree("roomuserid_invited").unwrap(),
                userroomid_left: db.open_tree("userroomid_left").unwrap(),
            },
            account_data: account_data::AccountData {
                roomuserdataid_accountdata: db.open_tree("roomuserdataid_accountdata").unwrap(),
            },
            //globalallid_globalall: db.open_tree("globalallid_globalall").unwrap(),
            //globallatestid_globallatest: db.open_tree("globallatestid_globallatest").unwrap(),
            _db: db,
        }
    }
}
