use std::{collections::BTreeMap, error::Error};

use async_trait::async_trait;
use ruma::{
	api::federation::discovery::{ServerSigningKeys, VerifyKey},
	signatures::Ed25519KeyPair,
	DeviceId, OwnedServerSigningKeyId, ServerName, UserId,
};

use crate::Result;

#[async_trait]
pub trait Data: Send + Sync {
	fn next_count(&self) -> Result<u64>;
	fn current_count(&self) -> Result<u64>;
	fn last_check_for_updates_id(&self) -> Result<u64>;
	fn update_check_for_updates_id(&self, id: u64) -> Result<()>;
	#[allow(unused_qualifications)] // async traits
	async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result<()>;
	fn cleanup(&self) -> Result<()>;
	fn flush(&self) -> Result<()>;
	fn memory_usage(&self) -> String;
	fn clear_caches(&self, amount: u32);
	fn load_keypair(&self) -> Result<Ed25519KeyPair>;
	fn remove_keypair(&self) -> Result<()>;
	fn add_signing_key(
		&self, origin: &ServerName, new_keys: ServerSigningKeys,
	) -> Result<BTreeMap<OwnedServerSigningKeyId, VerifyKey>>;

	/// This returns an empty `Ok(BTreeMap<..>)` when there are no keys found
	/// for the server.
	fn signing_keys_for(&self, origin: &ServerName) -> Result<BTreeMap<OwnedServerSigningKeyId, VerifyKey>>;
	fn database_version(&self) -> Result<u64>;
	fn bump_database_version(&self, new_version: u64) -> Result<()>;
	fn backup(&self) -> Result<(), Box<dyn Error>> { unimplemented!() }
	fn backup_list(&self) -> Result<String> { Ok(String::new()) }
}
