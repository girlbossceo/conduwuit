use serde::{Serialize, Deserialize};
use url::Host;


pub trait Data: Send + Sync {
    /// check if given host exists in Acls, if so return it
    fn check_acl(&self,host: &Host<String> ) -> crate::Result<Option<AclMode>>;

    /// add a given Acl entry to the database
    fn add_acl(&self, acl: AclDatabaseEntry) -> crate::Result<()>;
    /// remove a given Acl entry from the database
    fn remove_acl(&self,host: Host<String>) -> crate::Result<()>;
}

#[derive(Serialize,Deserialize, Debug, Clone, Copy)]
pub enum AclMode{
    Block,
    Allow
}
#[derive(Serialize,Deserialize, Debug, Clone)]

pub struct AclDatabaseEntry { 
    pub(crate) mode: AclMode,
    pub(crate) hostname: Host
}