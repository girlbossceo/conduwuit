use tracing::warn;

use crate::{service::acl::{Data, AclDatabaseEntry, AclMode}, KeyValueDatabase};

impl Data for KeyValueDatabase {
    fn check_acl(&self,host: &url::Host<String> ) -> crate::Result<Option<AclMode>> {
        let thing = self.acl_list.get(host.to_string().as_bytes())?;
         if let Some(thing) = thing  {
               match thing.first() {
                Some(0x1) => Ok(Some(AclMode::Allow)),
                Some(0x0) => Ok(Some(AclMode::Block)),
                Some(invalid) => {
                    warn!("found invalid value for mode byte in value {}, probably db corruption", invalid);
                    Ok(None)
                }
                None => Ok(None),
            }
         }else {
            Ok(None)
         }
    }

    fn add_acl(&self, acl: AclDatabaseEntry) -> crate::Result<()> {
        self.acl_list.insert(acl.hostname.to_string().as_bytes(), match acl.mode  {
            AclMode::Block => &[0x0],
            AclMode::Allow => &[0x1],
        })
    }

    fn remove_acl(&self,host: url::Host<String>) -> crate::Result<()> {
        self.acl_list.remove(host.to_string().as_bytes())
    }
}