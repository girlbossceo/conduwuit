use std::collections::HashSet;

use tracing::warn;
use url::Host;

use crate::{
    service::acl::{AclDatabaseEntry, AclMode, Data},
    KeyValueDatabase,
};

impl Data for KeyValueDatabase {
    fn check_acl(&self, host: &Host<String>) -> crate::Result<Option<AclMode>> {
        let thing = self.acl_list.get(host.to_string().as_bytes())?;
        if let Some(thing) = thing {
            match thing.first() {
                Some(0x1) => Ok(Some(AclMode::Allow)),
                Some(0x0) => Ok(Some(AclMode::Block)),
                Some(invalid) => {
                    warn!(
                        "found invalid value for mode byte in value {}, probably db corruption",
                        invalid
                    );
                    Ok(None)
                }
                None => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    fn add_acl(&self, acl: AclDatabaseEntry) -> crate::Result<()> {
        self.acl_list.insert(
            acl.hostname.to_string().as_bytes(),
            match acl.mode {
                AclMode::Block => &[0x0],
                AclMode::Allow => &[0x1],
            },
        )
    }

    fn remove_acl(&self, host: Host<String>) -> crate::Result<()> {
        self.acl_list.remove(host.to_string().as_bytes())
    }

    fn get_all_acls(&self) -> HashSet<AclDatabaseEntry> {
        let mut set = HashSet::new();

        self.acl_list.iter().for_each(|it| {
            let Ok(key) = String::from_utf8(it.0) else {
                return;
            };
            let Ok(parsed_host) = Host::parse(&key) else {
                warn!("failed to parse host {}", key);
                return;
            };
            let mode = match it.1.first() {
                Some(0x1) => AclMode::Allow,
                Some(0x0) => AclMode::Block,
                Some(invalid) => {
                    warn!(
                        "found invalid value for mode byte in value {}, probably db corruption",
                        invalid
                    );
                    return;
                }
                None => return,
            };
            set.insert(AclDatabaseEntry {
                mode,
                hostname: parsed_host,
            });
        });
        set
    }
}
