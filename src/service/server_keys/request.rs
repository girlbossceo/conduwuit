use std::collections::BTreeMap;

use conduit::{implement, Err, Result};
use ruma::{
	api::federation::discovery::{
		get_remote_server_keys,
		get_remote_server_keys_batch::{self, v2::QueryCriteria},
		get_server_keys, ServerSigningKeys,
	},
	OwnedServerName, OwnedServerSigningKeyId, ServerName, ServerSigningKeyId,
};

#[implement(super::Service)]
pub(super) async fn batch_notary_request<'a, S, K>(
	&self, notary: &ServerName, batch: S,
) -> Result<Vec<ServerSigningKeys>>
where
	S: Iterator<Item = (&'a ServerName, K)> + Send,
	K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
	use get_remote_server_keys_batch::v2::Request;
	type RumaBatch = BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, QueryCriteria>>;

	let criteria = QueryCriteria {
		minimum_valid_until_ts: Some(self.minimum_valid_ts()),
	};

	let mut server_keys = RumaBatch::new();
	for (server, key_ids) in batch {
		let entry = server_keys.entry(server.into()).or_default();
		for key_id in key_ids {
			entry.insert(key_id.into(), criteria.clone());
		}
	}

	debug_assert!(!server_keys.is_empty(), "empty batch request to notary");
	let request = Request {
		server_keys,
	};

	self.services
		.sending
		.send_federation_request(notary, request)
		.await
		.map(|response| response.server_keys)
		.map(|keys| {
			keys.into_iter()
				.map(|key| key.deserialize())
				.filter_map(Result::ok)
				.collect()
		})
}

#[implement(super::Service)]
pub async fn notary_request(&self, notary: &ServerName, target: &ServerName) -> Result<Vec<ServerSigningKeys>> {
	use get_remote_server_keys::v2::Request;

	let request = Request {
		server_name: target.into(),
		minimum_valid_until_ts: self.minimum_valid_ts(),
	};

	self.services
		.sending
		.send_federation_request(notary, request)
		.await
		.map(|response| response.server_keys)
		.map(|keys| {
			keys.into_iter()
				.map(|key| key.deserialize())
				.filter_map(Result::ok)
				.collect()
		})
}

#[implement(super::Service)]
pub async fn server_request(&self, target: &ServerName) -> Result<ServerSigningKeys> {
	use get_server_keys::v2::Request;

	let server_signing_key = self
		.services
		.sending
		.send_federation_request(target, Request::new())
		.await
		.map(|response| response.server_key)
		.and_then(|key| key.deserialize().map_err(Into::into))?;

	if server_signing_key.server_name != target {
		return Err!(BadServerResponse(debug_warn!(
			requested = ?target,
			response = ?server_signing_key.server_name,
			"Server responded with bogus server_name"
		)));
	}

	Ok(server_signing_key)
}
