use crate::utils;
use directories::ProjectDirs;
use sled::IVec;

pub struct MultiValue(sled::Tree);

impl MultiValue {
    /// Get an iterator over all values.
    pub fn iter_all(&self) -> sled::Iter {
        self.0.iter()
    }

    /// Get an iterator over all values of this id.
    pub fn get_iter(&self, id: &[u8]) -> sled::Iter {
        // Data keys start with d
        let mut key = vec![b'd'];
        key.extend_from_slice(id.as_ref());
        key.push(0xff); // Add delimiter so we don't find usernames starting with the same id

        self.0.scan_prefix(key)
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
    pub deviceid_token: sled::Tree,
    pub token_userid: sled::Tree,
    pub roomid_eventid_event: sled::Tree,
    _db: sled::Db,
}

impl Database {
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
            deviceid_token: db.open_tree("deviceid_token").unwrap(),
            token_userid: db.open_tree("token_userid").unwrap(),
            roomid_eventid_event: db.open_tree("roomid_eventid_event").unwrap(),
            _db: db,
        }
    }

    pub fn debug(&self) {
        println!("# UserId -> Password:");
        for (k, v) in self.userid_password.iter().map(|r| r.unwrap()) {
            println!(
                "{} -> {}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("# UserId -> DeviceIds:");
        for (k, v) in self.userid_deviceids.iter_all().map(|r| r.unwrap()) {
            println!(
                "{} -> {}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("# DeviceId -> Token:");
        for (k, v) in self.deviceid_token.iter().map(|r| r.unwrap()) {
            println!(
                "{} -> {}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("# Token -> UserId:");
        for (k, v) in self.token_userid.iter().map(|r| r.unwrap()) {
            println!(
                "{} -> {}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
        println!("# RoomId + EventId -> Event:");
        for (k, v) in self.roomid_eventid_event.iter().map(|r| r.unwrap()) {
            println!(
                "{} -> {}",
                String::from_utf8_lossy(&k),
                String::from_utf8_lossy(&v),
            );
        }
    }
}
