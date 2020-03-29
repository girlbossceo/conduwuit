use directories::ProjectDirs;
use ruma_events::collections::all::RoomEvent;
use ruma_identifiers::UserId;

pub struct Data(sled::Db);

impl Data {
    pub fn set_hostname(&self, hostname: &str) {
        self.0.insert("hostname", hostname).unwrap();
    }
    pub fn hostname(&self) -> String {
        String::from_utf8(self.0.get("hostname").unwrap().unwrap().to_vec()).unwrap()
    }
    pub fn load_or_create() -> Self {
        Data(
            sled::open(
                ProjectDirs::from("xyz", "koesters", "matrixserver")
                    .unwrap()
                    .data_dir(),
            )
            .unwrap(),
        )
    }

    pub fn user_exists(&self, user_id: &UserId) -> bool {
        self.0
            .open_tree("username_password")
            .unwrap()
            .contains_key(user_id.to_string())
            .unwrap()
    }

    pub fn user_add(&self, user_id: UserId, password: Option<String>) {
        self.0
            .open_tree("username_password")
            .unwrap()
            .insert(user_id.to_string(), &*password.unwrap_or_default())
            .unwrap();
    }

    pub fn room_event_add(&self, room_event: &RoomEvent) {}
}
