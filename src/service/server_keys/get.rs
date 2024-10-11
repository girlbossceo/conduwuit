use std::borrow::Borrow;

use conduit::{implement, Err, Result};
use ruma::{api::federation::discovery::VerifyKey, CanonicalJsonObject, RoomVersionId, ServerName, ServerSigningKeyId};

use super::{extract_key, PubKeyMap, PubKeys};

#[implement(super::Service)]
pub async fn get_event_keys(&self, object: &CanonicalJsonObject, version: &RoomVersionId) -> Result<PubKeyMap> {
	use ruma::signatures::required_keys;

	let required = match required_keys(object, version) {
		Ok(required) => required,
		Err(e) => return Err!(BadServerResponse("Failed to determine keys required to verify: {e}")),
	};

	let batch = required
		.iter()
		.map(|(s, ids)| (s.borrow(), ids.iter().map(Borrow::borrow)));

	Ok(self.get_pubkeys(batch).await)
}

#[implement(super::Service)]
pub async fn get_pubkeys<'a, S, K>(&self, batch: S) -> PubKeyMap
where
	S: Iterator<Item = (&'a ServerName, K)> + Send,
	K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
	let mut keys = PubKeyMap::new();
	for (server, key_ids) in batch {
		let pubkeys = self.get_pubkeys_for(server, key_ids).await;
		keys.insert(server.into(), pubkeys);
	}

	keys
}

#[implement(super::Service)]
pub async fn get_pubkeys_for<'a, I>(&self, origin: &ServerName, key_ids: I) -> PubKeys
where
	I: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
	let mut keys = PubKeys::new();
	for key_id in key_ids {
		if let Ok(verify_key) = self.get_verify_key(origin, key_id).await {
			keys.insert(key_id.into(), verify_key.key);
		}
	}

	keys
}

#[implement(super::Service)]
pub async fn get_verify_key(&self, origin: &ServerName, key_id: &ServerSigningKeyId) -> Result<VerifyKey> {
	if let Some(result) = self.verify_keys_for(origin).await.remove(key_id) {
		return Ok(result);
	}

	if let Ok(server_key) = self.server_request(origin).await {
		self.add_signing_keys(server_key.clone()).await;
		if let Some(result) = extract_key(server_key, key_id) {
			return Ok(result);
		}
	}

	for notary in self.services.globals.trusted_servers() {
		if let Ok(server_keys) = self.notary_request(notary, origin).await {
			for server_key in &server_keys {
				self.add_signing_keys(server_key.clone()).await;
			}

			for server_key in server_keys {
				if let Some(result) = extract_key(server_key, key_id) {
					return Ok(result);
				}
			}
		}
	}

	Err!(BadServerResponse(debug_error!(
		?key_id,
		?origin,
		"Failed to fetch federation signing-key"
	)))
}
