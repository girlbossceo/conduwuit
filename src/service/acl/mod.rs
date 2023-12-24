use std::sync::Arc;

use ruma::ServerName;
use tracing::{debug, error, warn};
use url::Host;

use crate::{api::server_server::FedDest, config::acl::AccessControlListConfig};

pub use self::data::*;
mod data;
pub struct Service {
    pub db: &'static dyn Data,
    pub acl_config: Arc<AccessControlListConfig>,
}

impl Service {
    pub fn list_acls(&self, filter: Option<AclMode>) -> Vec<AclDatabaseEntry> {
        let mut set = self.db.get_all_acls();
        self.acl_config
            .allow_list
            .clone()
            .unwrap_or_default()
            .iter()
            .for_each(|it| {
                set.insert(AclDatabaseEntry {
                    mode: AclMode::Allow,
                    hostname: it.to_owned(),
                });
            });
        self.acl_config.block_list.clone().iter().for_each(|it| {
            set.insert(AclDatabaseEntry {
                mode: AclMode::Block,
                hostname: it.to_owned(),
            });
        });
        match filter {
            Some(filter) => set.into_iter().filter(|it| it.mode == filter).collect(),
            None => set.into_iter().collect(),
        }
    }
    pub fn remove_acl(&self, host: Host) -> crate::Result<Option<()>> {
        self.db.remove_acl(host)
    }

    pub fn add_acl(&self, host: Host, mode: AclMode) -> crate::Result<()> {
        self.db.add_acl(AclDatabaseEntry {
            mode,
            hostname: host,
        })
    }
    /// same as federation_with_allowed however it can work with the fedi_dest type
    pub fn is_federation_with_allowed_fedi_dest(&self, fedi_dest: &FedDest) -> bool {
        let hostname = if let Ok(name) = Host::parse(&fedi_dest.hostname()) {
            name
        } else {
            warn!(
                "cannot deserialise hostname for server with name {:?}",
                fedi_dest
            );
            return false;
        };
        self.is_federation_with_allowed(hostname)
    }

    /// same as federation_with_allowed however it can work with the fedi_dest type
    pub fn is_federation_with_allowed_server_name(&self, srv: &ServerName) -> bool {
        let hostname = if let Ok(name) = Host::parse(srv.host()) {
            name
        } else {
            warn!("cannot deserialise hostname for server with name {:?}", srv);
            return false;
        };
        self.is_federation_with_allowed(hostname)
    }
    /// is federation allowed with this particular server?
    pub fn is_federation_with_allowed(&self, server_host_name: Host<String>) -> bool {
        debug!("checking federation allowance for {}", server_host_name);
        // check blocklist first
        if self.acl_config.block_list.contains(&server_host_name) {
            return false;
        }
        let mut allow_list_enabled = false;
        // check allowlist
        if let Some(list) = &self.acl_config.allow_list {
            if list.contains(&server_host_name) {
                return true;
            }
            allow_list_enabled = true;
        }

        //check database
        match self.db.check_acl(&server_host_name) {
            Err(error) => {
                error!("database failed with {}", error);
                false
            }
            Ok(None) if allow_list_enabled => false,
            Ok(None) => true,
            Ok(Some(data::AclMode::Block)) => false,
            Ok(Some(data::AclMode::Allow)) if allow_list_enabled => true,
            Ok(Some(data::AclMode::Allow)) => {
                warn!("allowlist value found in database for {} but allow list is not enabled, allowed request", server_host_name);
                true
            }
        }
    }
}
