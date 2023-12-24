

use std::sync::Arc;

use ruma::ServerName;
use tracing::{warn, debug, error};
use url::Host;

use crate::{config::acl::AccessControlListConfig, api::server_server::FedDest};

pub use self::data::*;
mod data;
pub struct Service {
    pub db: &'static dyn Data,
    pub acl_config: Arc<AccessControlListConfig>
}

impl Service {
        /// same as federation_with_allowed however it can work with the fedi_dest type
        pub fn is_federation_with_allowed_fedi_dest(&self,fedi_dest: &FedDest) -> bool {
            let hostname = if let Ok(name) = Host::parse(&fedi_dest.hostname()) {
                name
            } else {
                warn!("cannot deserialise hostname for server with name {:?}",fedi_dest);
                return  false;
            };
            return self.is_federation_with_allowed(hostname);
        }

                /// same as federation_with_allowed however it can work with the fedi_dest type
        pub fn is_federation_with_allowed_server_name(&self,srv: &ServerName) -> bool {
            let hostname = if let Ok(name) = Host::parse(srv.host()) {
                name
            } else {
                warn!("cannot deserialise hostname for server with name {:?}",srv);
                return  false;
            };
            return self.is_federation_with_allowed(hostname);
        }
        /// is federation allowed with this particular server?
        pub fn is_federation_with_allowed(&self,server_host_name: Host<String>) -> bool {
            debug!("checking federation allowance for {}", server_host_name);
            // check blocklist first
            if self.acl_config.block_list.contains(&server_host_name) {
                return  false;
            }
            let mut allow_list_enabled = false;
            // check allowlist
            if let Some(list) = &self.acl_config.allow_list {
                if list.contains(&server_host_name) {
                    return  true;
                }
                allow_list_enabled = true;
            }

            //check database
            match self.db.check_acl(&server_host_name) {
                Err(error) => {
                    error!("database failed with {}",error);
                     false
                }
                Ok(None) if allow_list_enabled => false,
                Ok(None) => true,
                Ok(Some(data::AclMode::Block)) =>  false, 
                Ok(Some(data::AclMode::Allow))  if allow_list_enabled => true,
                Ok(Some(data::AclMode::Allow)) => {
                    warn!("allowlist value found in database for {} but allow list is not enabled, denied request", server_host_name);
                 false
                }
                
            }
        }
}