use std::{
	borrow::Borrow,
	collections::{BTreeMap, BTreeSet},
};

use conduit::{debug, debug_warn, error, implement, result::FlatOk, warn};
use futures::{stream::FuturesUnordered, StreamExt};
use ruma::{
	api::federation::discovery::ServerSigningKeys, serde::Raw, CanonicalJsonObject, OwnedServerName,
	OwnedServerSigningKeyId, ServerName, ServerSigningKeyId,
};
use serde_json::value::RawValue as RawJsonValue;

use super::key_exists;

type Batch = BTreeMap<OwnedServerName, Vec<OwnedServerSigningKeyId>>;

#[implement(super::Service)]
pub async fn acquire_events_pubkeys<'a, I>(&self, events: I)
where
	I: Iterator<Item = &'a Box<RawJsonValue>> + Send,
{
	type Batch = BTreeMap<OwnedServerName, BTreeSet<OwnedServerSigningKeyId>>;
	type Signatures = BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>;

	let mut batch = Batch::new();
	events
		.cloned()
		.map(Raw::<CanonicalJsonObject>::from_json)
		.map(|event| event.get_field::<Signatures>("signatures"))
		.filter_map(FlatOk::flat_ok)
		.flat_map(IntoIterator::into_iter)
		.for_each(|(server, sigs)| {
			batch.entry(server).or_default().extend(sigs.into_keys());
		});

	let batch = batch
		.iter()
		.map(|(server, keys)| (server.borrow(), keys.iter().map(Borrow::borrow)));

	self.acquire_pubkeys(batch).await;
}

#[implement(super::Service)]
pub async fn acquire_pubkeys<'a, S, K>(&self, batch: S)
where
	S: Iterator<Item = (&'a ServerName, K)> + Send + Clone,
	K: Iterator<Item = &'a ServerSigningKeyId> + Send + Clone,
{
	let notary_only = self.services.server.config.only_query_trusted_key_servers;
	let notary_first_always = self.services.server.config.query_trusted_key_servers_first;
	let notary_first_on_join = self
		.services
		.server
		.config
		.query_trusted_key_servers_first_on_join;

	let requested_servers = batch.clone().count();
	let requested_keys = batch.clone().flat_map(|(_, key_ids)| key_ids).count();

	debug!("acquire {requested_keys} keys from {requested_servers}");

	let mut missing = self.acquire_locals(batch).await;
	let mut missing_keys = keys_count(&missing);
	let mut missing_servers = missing.len();
	if missing_servers == 0 {
		return;
	}

	debug!("missing {missing_keys} keys for {missing_servers} servers locally");

	if notary_first_always || notary_first_on_join {
		missing = self.acquire_notary(missing.into_iter()).await;
		missing_keys = keys_count(&missing);
		missing_servers = missing.len();
		if missing_keys == 0 {
			return;
		}

		debug_warn!("missing {missing_keys} keys for {missing_servers} servers from all notaries first");
	}

	if !notary_only {
		missing = self.acquire_origins(missing.into_iter()).await;
		missing_keys = keys_count(&missing);
		missing_servers = missing.len();
		if missing_keys == 0 {
			return;
		}

		debug_warn!("missing {missing_keys} keys for {missing_servers} servers unreachable");
	}

	if !notary_first_always && !notary_first_on_join {
		missing = self.acquire_notary(missing.into_iter()).await;
		missing_keys = keys_count(&missing);
		missing_servers = missing.len();
		if missing_keys == 0 {
			return;
		}

		debug_warn!("still missing {missing_keys} keys for {missing_servers} servers from all notaries.");
	}

	if missing_keys > 0 {
		warn!(
			"did not obtain {missing_keys} keys for {missing_servers} servers out of {requested_keys} total keys for \
			 {requested_servers} total servers; some events may not be verifiable"
		);
	}
}

#[implement(super::Service)]
async fn acquire_locals<'a, S, K>(&self, batch: S) -> Batch
where
	S: Iterator<Item = (&'a ServerName, K)> + Send,
	K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
	let mut missing = Batch::new();
	for (server, key_ids) in batch {
		for key_id in key_ids {
			if !self.verify_key_exists(server, key_id).await {
				missing
					.entry(server.into())
					.or_default()
					.push(key_id.into());
			}
		}
	}

	missing
}

#[implement(super::Service)]
async fn acquire_origins<I>(&self, batch: I) -> Batch
where
	I: Iterator<Item = (OwnedServerName, Vec<OwnedServerSigningKeyId>)> + Send,
{
	let mut requests: FuturesUnordered<_> = batch
		.map(|(origin, key_ids)| self.acquire_origin(origin, key_ids))
		.collect();

	let mut missing = Batch::new();
	while let Some((origin, key_ids)) = requests.next().await {
		if !key_ids.is_empty() {
			missing.insert(origin, key_ids);
		}
	}

	missing
}

#[implement(super::Service)]
async fn acquire_origin(
	&self, origin: OwnedServerName, mut key_ids: Vec<OwnedServerSigningKeyId>,
) -> (OwnedServerName, Vec<OwnedServerSigningKeyId>) {
	if let Ok(server_keys) = self.server_request(&origin).await {
		self.add_signing_keys(server_keys.clone()).await;
		key_ids.retain(|key_id| !key_exists(&server_keys, key_id));
	}

	(origin, key_ids)
}

#[implement(super::Service)]
async fn acquire_notary<I>(&self, batch: I) -> Batch
where
	I: Iterator<Item = (OwnedServerName, Vec<OwnedServerSigningKeyId>)> + Send,
{
	let mut missing: Batch = batch.collect();
	for notary in self.services.globals.trusted_servers() {
		let missing_keys = keys_count(&missing);
		let missing_servers = missing.len();
		debug!("Asking notary {notary} for {missing_keys} missing keys from {missing_servers} servers");

		let batch = missing
			.iter()
			.map(|(server, keys)| (server.borrow(), keys.iter().map(Borrow::borrow)));

		match self.batch_notary_request(notary, batch).await {
			Err(e) => error!("Failed to contact notary {notary:?}: {e}"),
			Ok(results) => {
				for server_keys in results {
					self.acquire_notary_result(&mut missing, server_keys).await;
				}
			},
		}
	}

	missing
}

#[implement(super::Service)]
async fn acquire_notary_result(&self, missing: &mut Batch, server_keys: ServerSigningKeys) {
	let server = &server_keys.server_name;
	self.add_signing_keys(server_keys.clone()).await;

	if let Some(key_ids) = missing.get_mut(server) {
		key_ids.retain(|key_id| key_exists(&server_keys, key_id));
		if key_ids.is_empty() {
			missing.remove(server);
		}
	}
}

fn keys_count(batch: &Batch) -> usize { batch.iter().flat_map(|(_, key_ids)| key_ids.iter()).count() }
