use std::{
	collections::{hash_map, HashSet},
	time::{Duration, Instant, SystemTime},
};

use futures_util::{stream::FuturesUnordered, StreamExt};
use ruma::{
	api::federation::{
		discovery::{
			get_remote_server_keys,
			get_remote_server_keys_batch::{self, v2::QueryCriteria},
			get_server_keys,
		},
		membership::create_join_event,
	},
	serde::Base64,
	CanonicalJsonObject, CanonicalJsonValue, EventId, MilliSecondsSinceUnixEpoch, OwnedServerName,
	OwnedServerSigningKeyId, RoomVersionId, ServerName,
};
use serde_json::value::RawValue as RawJsonValue;
use tokio::sync::{RwLock, RwLockWriteGuard, Semaphore};
use tracing::{debug, error, info, trace, warn};

use crate::{
	service::{Arc, BTreeMap, HashMap, Result},
	services, Error,
};

impl super::Service {
	pub(crate) async fn fetch_required_signing_keys<'a, E>(
		&'a self, events: E, pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<()>
	where
		E: IntoIterator<Item = &'a BTreeMap<String, CanonicalJsonValue>>,
	{
		let mut server_key_ids = HashMap::new();
		for event in events {
			for (signature_server, signature) in event
				.get("signatures")
				.ok_or(Error::BadServerResponse("No signatures in server response pdu."))?
				.as_object()
				.ok_or(Error::BadServerResponse("Invalid signatures object in server response pdu."))?
			{
				let signature_object = signature.as_object().ok_or(Error::BadServerResponse(
					"Invalid signatures content object in server response pdu.",
				))?;

				for signature_id in signature_object.keys() {
					server_key_ids
						.entry(signature_server.clone())
						.or_insert_with(HashSet::new)
						.insert(signature_id.clone());
				}
			}
		}

		if server_key_ids.is_empty() {
			// Nothing to do, can exit early
			trace!("server_key_ids is empty, not fetching any keys");
			return Ok(());
		}

		trace!(
			"Fetch keys for {}",
			server_key_ids
				.keys()
				.cloned()
				.collect::<Vec<_>>()
				.join(", ")
		);

		let mut server_keys: FuturesUnordered<_> = server_key_ids
			.into_iter()
			.map(|(signature_server, signature_ids)| async {
				let fetch_res = self
					.fetch_signing_keys_for_server(
						signature_server.as_str().try_into().map_err(|_| {
							(
								signature_server.clone(),
								Error::BadServerResponse("Invalid servername in signatures of server response pdu."),
							)
						})?,
						signature_ids.into_iter().collect(), // HashSet to Vec
					)
					.await;

				match fetch_res {
					Ok(keys) => Ok((signature_server, keys)),
					Err(e) => {
						warn!("Signature verification failed: Could not fetch signing key for {signature_server}: {e}",);
						Err((signature_server, e))
					},
				}
			})
			.collect();

		while let Some(fetch_res) = server_keys.next().await {
			match fetch_res {
				Ok((signature_server, keys)) => {
					pub_key_map
						.write()
						.await
						.insert(signature_server.clone(), keys);
				},
				Err((signature_server, e)) => {
					warn!("Failed to fetch keys for {}: {:?}", signature_server, e);
				},
			}
		}

		Ok(())
	}

	// Gets a list of servers for which we don't have the signing key yet. We go
	// over the PDUs and either cache the key or add it to the list that needs to be
	// retrieved.
	async fn get_server_keys_from_cache(
		&self, pdu: &RawJsonValue,
		servers: &mut BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, QueryCriteria>>,
		room_version: &RoomVersionId,
		pub_key_map: &mut RwLockWriteGuard<'_, BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<()> {
		let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
			error!("Invalid PDU in server response: {:?}: {:?}", pdu, e);
			Error::BadServerResponse("Invalid PDU in server response")
		})?;

		let event_id = format!(
			"${}",
			ruma::signatures::reference_hash(&value, room_version).expect("ruma can calculate reference hashes")
		);
		let event_id = <&EventId>::try_from(event_id.as_str()).expect("ruma's reference hashes are valid event ids");

		if let Some((time, tries)) = services()
			.globals
			.bad_event_ratelimiter
			.read()
			.await
			.get(event_id)
		{
			// Exponential backoff
			let mut min_elapsed_duration = Duration::from_secs(5 * 60) * (*tries) * (*tries);
			if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
				min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
			}

			if time.elapsed() < min_elapsed_duration {
				debug!("Backing off from {}", event_id);
				return Err(Error::BadServerResponse("bad event, still backing off"));
			}
		}

		let signatures = value
			.get("signatures")
			.ok_or(Error::BadServerResponse("No signatures in server response pdu."))?
			.as_object()
			.ok_or(Error::BadServerResponse("Invalid signatures object in server response pdu."))?;

		for (signature_server, signature) in signatures {
			let signature_object = signature.as_object().ok_or(Error::BadServerResponse(
				"Invalid signatures content object in server response pdu.",
			))?;

			let signature_ids = signature_object.keys().cloned().collect::<Vec<_>>();

			let contains_all_ids =
				|keys: &BTreeMap<String, Base64>| signature_ids.iter().all(|id| keys.contains_key(id));

			let origin = <&ServerName>::try_from(signature_server.as_str())
				.map_err(|_| Error::BadServerResponse("Invalid servername in signatures of server response pdu."))?;

			if servers.contains_key(origin) || pub_key_map.contains_key(origin.as_str()) {
				continue;
			}

			debug!("Loading signing keys for {}", origin);

			let result: BTreeMap<_, _> = services()
				.globals
				.signing_keys_for(origin)?
				.into_iter()
				.map(|(k, v)| (k.to_string(), v.key))
				.collect();

			if !contains_all_ids(&result) {
				debug!("Signing key not loaded for {}", origin);
				servers.insert(origin.to_owned(), BTreeMap::new());
			}

			pub_key_map.insert(origin.to_string(), result);
		}

		Ok(())
	}

	/// Batch requests homeserver signing keys from trusted notary key servers
	/// (`trusted_servers` config option)
	async fn batch_request_signing_keys(
		&self, mut servers: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, QueryCriteria>>,
		pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<()> {
		for server in services().globals.trusted_servers() {
			debug!("Asking batch signing keys from trusted server {}", server);
			match services()
				.sending
				.send_federation_request(
					server,
					get_remote_server_keys_batch::v2::Request {
						server_keys: servers.clone(),
					},
				)
				.await
			{
				Ok(keys) => {
					debug!("Got signing keys: {:?}", keys);
					let mut pkm = pub_key_map.write().await;
					for k in keys.server_keys {
						let k = match k.deserialize() {
							Ok(key) => key,
							Err(e) => {
								warn!("Received error {e} while fetching keys from trusted server {server}");
								warn!("{}", k.into_json());
								continue;
							},
						};

						// TODO: Check signature from trusted server?
						servers.remove(&k.server_name);

						let result = services()
							.globals
							.add_signing_key(&k.server_name, k.clone())?
							.into_iter()
							.map(|(k, v)| (k.to_string(), v.key))
							.collect::<BTreeMap<_, _>>();

						pkm.insert(k.server_name.to_string(), result);
					}
				},
				Err(e) => {
					warn!(
						"Failed sending batched key request to trusted key server {server} for the remote servers \
						 {:?}: {e}",
						servers
					);
				},
			}
		}

		Ok(())
	}

	/// Requests multiple homeserver signing keys from individual servers (not
	/// trused notary servers)
	async fn request_signing_keys(
		&self, servers: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, QueryCriteria>>,
		pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<()> {
		debug!("Asking individual servers for signing keys: {servers:?}");
		let mut futures: FuturesUnordered<_> = servers
			.into_keys()
			.map(|server| async move {
				(
					services()
						.sending
						.send_federation_request(&server, get_server_keys::v2::Request::new())
						.await,
					server,
				)
			})
			.collect();

		while let Some(result) = futures.next().await {
			debug!("Received new Future result");
			if let (Ok(get_keys_response), origin) = result {
				debug!("Result is from {origin}");
				if let Ok(key) = get_keys_response.server_key.deserialize() {
					let result: BTreeMap<_, _> = services()
						.globals
						.add_signing_key(&origin, key)?
						.into_iter()
						.map(|(k, v)| (k.to_string(), v.key))
						.collect();
					pub_key_map.write().await.insert(origin.to_string(), result);
				}
			}
			debug!("Done handling Future result");
		}

		Ok(())
	}

	pub(crate) async fn fetch_join_signing_keys(
		&self, event: &create_join_event::v2::Response, room_version: &RoomVersionId,
		pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<()> {
		let mut servers: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, QueryCriteria>> = BTreeMap::new();

		{
			let mut pkm = pub_key_map.write().await;

			// Try to fetch keys, failure is okay
			// Servers we couldn't find in the cache will be added to `servers`
			for pdu in &event.room_state.state {
				_ = self
					.get_server_keys_from_cache(pdu, &mut servers, room_version, &mut pkm)
					.await;
			}
			for pdu in &event.room_state.auth_chain {
				_ = self
					.get_server_keys_from_cache(pdu, &mut servers, room_version, &mut pkm)
					.await;
			}

			drop(pkm);
		};

		if servers.is_empty() {
			trace!("We had all keys cached locally, not fetching any keys from remote servers");
			return Ok(());
		}

		if services().globals.query_trusted_key_servers_first() {
			info!(
				"query_trusted_key_servers_first is set to true, querying notary trusted key servers first for \
				 homeserver signing keys."
			);

			self.batch_request_signing_keys(servers.clone(), pub_key_map)
				.await?;

			if servers.is_empty() {
				debug!("Trusted server supplied all signing keys, no more keys to fetch");
				return Ok(());
			}

			debug!("Remaining servers left that the notary/trusted servers did not provide: {servers:?}");

			self.request_signing_keys(servers.clone(), pub_key_map)
				.await?;
		} else {
			debug!("query_trusted_key_servers_first is set to false, querying individual homeservers first");

			self.request_signing_keys(servers.clone(), pub_key_map)
				.await?;

			if servers.is_empty() {
				debug!("Individual homeservers supplied all signing keys, no more keys to fetch");
				return Ok(());
			}

			debug!("Remaining servers left the individual homeservers did not provide: {servers:?}");

			self.batch_request_signing_keys(servers.clone(), pub_key_map)
				.await?;
		}

		debug!("Search for signing keys done");

		/*if servers.is_empty() {
			warn!("Failed to find homeserver signing keys for the remaining servers: {servers:?}");
		}*/

		Ok(())
	}

	/// Search the DB for the signing keys of the given server, if we don't have
	/// them fetch them from the server and save to our DB.
	#[tracing::instrument(skip_all)]
	pub async fn fetch_signing_keys_for_server(
		&self, origin: &ServerName, signature_ids: Vec<String>,
	) -> Result<BTreeMap<String, Base64>> {
		let contains_all_ids = |keys: &BTreeMap<String, Base64>| signature_ids.iter().all(|id| keys.contains_key(id));

		let permit = services()
			.globals
			.servername_ratelimiter
			.read()
			.await
			.get(origin)
			.map(|s| Arc::clone(s).acquire_owned());

		let permit = if let Some(p) = permit {
			p
		} else {
			let mut write = services().globals.servername_ratelimiter.write().await;
			let s = Arc::clone(
				write
					.entry(origin.to_owned())
					.or_insert_with(|| Arc::new(Semaphore::new(1))),
			);

			s.acquire_owned()
		}
		.await;

		let back_off = |id| async {
			match services()
				.globals
				.bad_signature_ratelimiter
				.write()
				.await
				.entry(id)
			{
				hash_map::Entry::Vacant(e) => {
					e.insert((Instant::now(), 1));
				},
				hash_map::Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1 + 1),
			}
		};

		if let Some((time, tries)) = services()
			.globals
			.bad_signature_ratelimiter
			.read()
			.await
			.get(&signature_ids)
		{
			// Exponential backoff
			let mut min_elapsed_duration = Duration::from_secs(5 * 60) * (*tries) * (*tries);
			if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
				min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
			}

			if time.elapsed() < min_elapsed_duration {
				debug!("Backing off from {:?}", signature_ids);
				return Err(Error::BadServerResponse("bad signature, still backing off"));
			}
		}

		let mut result: BTreeMap<_, _> = services()
			.globals
			.signing_keys_for(origin)?
			.into_iter()
			.map(|(k, v)| (k.to_string(), v.key))
			.collect();

		if contains_all_ids(&result) {
			trace!("We have all homeserver signing keys locally for {origin}, not fetching any remotely");
			return Ok(result);
		}

		// i didnt split this out into their own functions because it's relatively small
		if services().globals.query_trusted_key_servers_first() {
			info!(
				"query_trusted_key_servers_first is set to true, querying notary trusted servers first for {origin} \
				 keys"
			);

			for server in services().globals.trusted_servers() {
				debug!("Asking notary server {server} for {origin}'s signing key");
				if let Some(server_keys) = services()
					.sending
					.send_federation_request(
						server,
						get_remote_server_keys::v2::Request::new(
							origin.to_owned(),
							MilliSecondsSinceUnixEpoch::from_system_time(
								SystemTime::now()
									.checked_add(Duration::from_secs(3600))
									.expect("SystemTime too large"),
							)
							.expect("time is valid"),
						),
					)
					.await
					.ok()
					.map(|resp| {
						resp.server_keys
							.into_iter()
							.filter_map(|e| e.deserialize().ok())
							.collect::<Vec<_>>()
					}) {
					debug!("Got signing keys: {:?}", server_keys);
					for k in server_keys {
						services().globals.add_signing_key(origin, k.clone())?;
						result.extend(
							k.verify_keys
								.into_iter()
								.map(|(k, v)| (k.to_string(), v.key)),
						);
						result.extend(
							k.old_verify_keys
								.into_iter()
								.map(|(k, v)| (k.to_string(), v.key)),
						);
					}

					if contains_all_ids(&result) {
						return Ok(result);
					}
				}
			}

			debug!("Asking {origin} for their signing keys over federation");
			if let Some(server_key) = services()
				.sending
				.send_federation_request(origin, get_server_keys::v2::Request::new())
				.await
				.ok()
				.and_then(|resp| resp.server_key.deserialize().ok())
			{
				services()
					.globals
					.add_signing_key(origin, server_key.clone())?;

				result.extend(
					server_key
						.verify_keys
						.into_iter()
						.map(|(k, v)| (k.to_string(), v.key)),
				);
				result.extend(
					server_key
						.old_verify_keys
						.into_iter()
						.map(|(k, v)| (k.to_string(), v.key)),
				);

				if contains_all_ids(&result) {
					return Ok(result);
				}
			}
		} else {
			info!("query_trusted_key_servers_first is set to false, querying {origin} first");

			debug!("Asking {origin} for their signing keys over federation");
			if let Some(server_key) = services()
				.sending
				.send_federation_request(origin, get_server_keys::v2::Request::new())
				.await
				.ok()
				.and_then(|resp| resp.server_key.deserialize().ok())
			{
				services()
					.globals
					.add_signing_key(origin, server_key.clone())?;

				result.extend(
					server_key
						.verify_keys
						.into_iter()
						.map(|(k, v)| (k.to_string(), v.key)),
				);
				result.extend(
					server_key
						.old_verify_keys
						.into_iter()
						.map(|(k, v)| (k.to_string(), v.key)),
				);

				if contains_all_ids(&result) {
					return Ok(result);
				}
			}

			for server in services().globals.trusted_servers() {
				debug!("Asking notary server {server} for {origin}'s signing key");
				if let Some(server_keys) = services()
					.sending
					.send_federation_request(
						server,
						get_remote_server_keys::v2::Request::new(
							origin.to_owned(),
							MilliSecondsSinceUnixEpoch::from_system_time(
								SystemTime::now()
									.checked_add(Duration::from_secs(3600))
									.expect("SystemTime too large"),
							)
							.expect("time is valid"),
						),
					)
					.await
					.ok()
					.map(|resp| {
						resp.server_keys
							.into_iter()
							.filter_map(|e| e.deserialize().ok())
							.collect::<Vec<_>>()
					}) {
					debug!("Got signing keys: {:?}", server_keys);
					for k in server_keys {
						services().globals.add_signing_key(origin, k.clone())?;
						result.extend(
							k.verify_keys
								.into_iter()
								.map(|(k, v)| (k.to_string(), v.key)),
						);
						result.extend(
							k.old_verify_keys
								.into_iter()
								.map(|(k, v)| (k.to_string(), v.key)),
						);
					}

					if contains_all_ids(&result) {
						return Ok(result);
					}
				}
			}
		}

		drop(permit);

		back_off(signature_ids).await;

		warn!("Failed to find public key for server: {origin}");
		Err(Error::BadServerResponse("Failed to find public key for server"))
	}
}
