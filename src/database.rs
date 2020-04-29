use crate::utils;
use directories::ProjectDirs;
use sled::IVec;
use std::fs::remove_dir_all;

pub struct MultiValue(sled::Tree);

impl MultiValue {
    /// Get an iterator over all values.
    pub fn iter_all(&self) -> sled::Iter {
        self.0.scan_prefix(b"d")
    }

    /// Get an iterator over all values of this id.
    pub fn get_iter(&self, id: &[u8]) -> sled::Iter {
        // Data keys start with d
        let mut key = vec![b'd'];
        key.extend_from_slice(id.as_ref());
        key.push(0xff); // Add delimiter so we don't find keys starting with the same id

        self.0.scan_prefix(key)
    }

    pub fn clear(&self, id: &[u8]) {
        for key in self.get_iter(id).keys() {
            self.0.remove(key.unwrap()).unwrap();
        }
    }

    pub fn remove_value(&self, id: &[u8], value: &[u8]) {
        if let Some(key) = self
            .get_iter(id)
            .find(|t| &t.as_ref().unwrap().1 == value)
            .map(|t| t.unwrap().0)
        {
            self.0.remove(key).unwrap();
        }
    }

    /// Add another value to the id.
    pub fn add(&self, id: &[u8], value: IVec) {
        // The new value will need a new index. We store the last used index in 'n' + id
        let mut count_key: Vec<u8> = vec![b'n'];
        count_key.extend_from_slice(id.as_ref());

        // Increment the last index and use that
        let index = self
            .0
            .update_and_fetch(&count_key, utils::increment)
            .unwrap()
            .unwrap();

        // Data keys start with d
        let mut key = vec![b'd'];
        key.extend_from_slice(id.as_ref());
        key.push(0xff);
        key.extend_from_slice(&index);

        self.0.insert(key, value).unwrap();
    }
}

pub struct Database {
    pub userid_password: sled::Tree,
    pub userid_displayname: sled::Tree,
    pub userid_avatarurl: sled::Tree,
    pub userid_deviceids: MultiValue,
    pub userdeviceid_token: sled::Tree,
    pub token_userid: sled::Tree,
    pub pduid_pdu: sled::Tree, // PduId = 'd' + RoomId + Since (global since counter is at 'n')
    pub eventid_pduid: sled::Tree,
    pub roomid_pduleaves: MultiValue,
    pub roomstateid_pdu: sled::Tree, // Room + StateType + StateKey
    pub roomid_joinuserids: MultiValue,
    pub roomid_inviteuserids: MultiValue,
    pub userid_joinroomids: MultiValue,
    pub userid_inviteroomids: MultiValue,
    pub userid_leftroomids: MultiValue,
    // EDUs:
    pub roomlatestid_roomlatest: sled::Tree, // Read Receipts, RoomLatestId = RoomId + Since + UserId TODO: Types
    pub roomactiveid_roomactive: sled::Tree, // Typing, RoomActiveId = TimeoutTime + Since
    pub globalallid_globalall: sled::Tree,   // ToDevice, GlobalAllId = UserId + Since
    pub globallatestid_globallatest: sled::Tree, // Presence, GlobalLatestId = Since + Type + UserId
    pub keypair: ruma_signatures::Ed25519KeyPair,
    _db: sled::Db,
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
            userid_password: db.open_tree("userid_password").unwrap(),
            userid_deviceids: MultiValue(db.open_tree("userid_deviceids").unwrap()),
            userid_displayname: db.open_tree("userid_displayname").unwrap(),
            userid_avatarurl: db.open_tree("userid_avatarurl").unwrap(),
            userdeviceid_token: db.open_tree("userdeviceid_token").unwrap(),
            token_userid: db.open_tree("token_userid").unwrap(),
            pduid_pdu: db.open_tree("pduid_pdu").unwrap(),
            eventid_pduid: db.open_tree("eventid_pduid").unwrap(),
            roomid_pduleaves: MultiValue(db.open_tree("roomid_pduleaves").unwrap()),
            roomstateid_pdu: db.open_tree("roomstateid_pdu").unwrap(),
            roomid_joinuserids: MultiValue(db.open_tree("roomid_joinuserids").unwrap()),
            roomid_inviteuserids: MultiValue(db.open_tree("roomid_inviteuserids").unwrap()),
            userid_joinroomids: MultiValue(db.open_tree("userid_joinroomids").unwrap()),
            userid_inviteroomids: MultiValue(db.open_tree("userid_inviteroomids").unwrap()),
            userid_leftroomids: MultiValue(db.open_tree("userid_leftroomids").unwrap()),
            roomlatestid_roomlatest: db.open_tree("roomlatestid_roomlatest").unwrap(),
            roomactiveid_roomactive: db.open_tree("roomactiveid_roomactive").unwrap(),
            globalallid_globalall: db.open_tree("globalallid_globalall").unwrap(),
            globallatestid_globallatest: db.open_tree("globallatestid_globallatest").unwrap(),
            keypair: ruma_signatures::Ed25519KeyPair::new(
                &*db.update_and_fetch("keypair", utils::generate_keypair)
                    .unwrap()
                    .unwrap(),
                "key1".to_owned(),
            )
            .unwrap(),
            _db: db,
        }
    }

    pub fn debug(&self) {
        println!("# UserId -> Password:");
        for (k, v) in self.userid_password.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# UserId -> DeviceIds:");
        for (k, v) in self.userid_deviceids.iter_all().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# UserId -> Displayname:");
        for (k, v) in self.userid_displayname.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# UserId -> AvatarURL:");
        for (k, v) in self.userid_avatarurl.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# UserId+DeviceId -> Token:");
        for (k, v) in self.userdeviceid_token.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# Token -> UserId:");
        for (k, v) in self.token_userid.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# RoomId -> PDU leaves:");
        for (k, v) in self.roomid_pduleaves.iter_all().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# RoomStateId -> PDU:");
        for (k, v) in self.roomstateid_pdu.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# RoomId -> UserIds:");
        for (k, v) in self.roomid_joinuserids.iter_all().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# UserId -> RoomIds:");
        for (k, v) in self.userid_joinroomids.iter_all().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# PDU Id -> PDU:");
        for (k, v) in self.pduid_pdu.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# EventId -> PDU Id:");
        for (k, v) in self.eventid_pduid.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# RoomLatestId -> RoomLatest:");
        for (k, v) in self.roomlatestid_roomlatest.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# RoomActiveId -> RoomActives:");
        for (k, v) in self.roomactiveid_roomactive.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# GlobalAllId -> GlobalAll:");
        for (k, v) in self.globalallid_globalall.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# GlobalLatestId -> GlobalLatest:");
        for (k, v) in self.globallatestid_globallatest.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
    }
}
