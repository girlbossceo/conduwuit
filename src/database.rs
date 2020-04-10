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
    pub userid_deviceids: MultiValue,
    pub userid_displayname: sled::Tree,
    pub userid_avatarurl: sled::Tree,
    pub deviceid_token: sled::Tree,
    pub token_userid: sled::Tree,
    pub pduid_pdus: sled::Tree,
    pub eventid_pduid: sled::Tree,
    pub roomid_pduleaves: MultiValue,
    pub roomid_userids: MultiValue,
    pub userid_roomids: MultiValue,
    _db: sled::Db,
}

impl Database {
    /// Tries to remove the old database but ignores all errors.
    pub fn try_remove(hostname: &str) {
        let mut path = ProjectDirs::from("xyz", "koesters", "matrixserver")
            .unwrap()
            .data_dir()
            .to_path_buf();
        path.push(hostname);
        let _ = remove_dir_all(path);
    }

    /// Load an existing database or create a new one.
    pub fn load_or_create(hostname: &str) -> Self {
        let mut path = ProjectDirs::from("xyz", "koesters", "matrixserver")
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
            deviceid_token: db.open_tree("deviceid_token").unwrap(),
            token_userid: db.open_tree("token_userid").unwrap(),
            pduid_pdus: db.open_tree("pduid_pdus").unwrap(),
            eventid_pduid: db.open_tree("eventid_pduid").unwrap(),
            roomid_pduleaves: MultiValue(db.open_tree("roomid_pduleaves").unwrap()),
            roomid_userids: MultiValue(db.open_tree("roomid_userids").unwrap()),
            userid_roomids: MultiValue(db.open_tree("userid_roomids").unwrap()),
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
        println!("# UserId -> Displayname:");
        for (k, v) in self.userid_displayname.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("# UserId -> AvatarURL:");
        for (k, v) in self.userid_avatarurl.iter().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# DeviceId -> Token:");
        for (k, v) in self.deviceid_token.iter().map(|r| r.unwrap()) {
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
        println!("\n# RoomId -> UserIds:");
        for (k, v) in self.roomid_userids.iter_all().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# UserId -> RoomIds:");
        for (k, v) in self.userid_roomids.iter_all().map(|r| r.unwrap()) {
            println!(
                "{:?} -> {:?}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("\n# PDU Id -> PDUs:");
        for (k, v) in self.pduid_pdus.iter().map(|r| r.unwrap()) {
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
    }
}
