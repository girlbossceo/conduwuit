use std::fmt::Write;

use conduit::{utils::time, Result};
use ruma::{events::room::message::RoomMessageEventContent, OwnedServerName};

use super::Resolver;
use crate::services;

/// All the getters and iterators in key_value/users.rs
pub(super) async fn resolver(subcommand: Resolver) -> Result<RoomMessageEventContent> {
	match subcommand {
		Resolver::DestinationsCache {
			server_name,
		} => destinations_cache(server_name).await,
		Resolver::OverridesCache {
			name,
		} => overrides_cache(name).await,
	}
}

async fn destinations_cache(server_name: Option<OwnedServerName>) -> Result<RoomMessageEventContent> {
	use service::resolver::CachedDest;

	let mut out = String::new();
	writeln!(out, "| Server Name | Destination | Hostname | Expires |")?;
	writeln!(out, "| ----------- | ----------- | -------- | ------- |")?;
	let row = |(
		name,
		&CachedDest {
			ref dest,
			ref host,
			expire,
		},
	)| {
		let expire = time::format(expire, "%+");
		writeln!(out, "| {name} | {dest} | {host} | {expire} |").expect("wrote line");
	};

	let map = services().resolver.destinations.read().expect("locked");

	if let Some(server_name) = server_name.as_ref() {
		map.get_key_value(server_name).map(row);
	} else {
		map.iter().for_each(row);
	}

	Ok(RoomMessageEventContent::notice_markdown(out))
}

async fn overrides_cache(server_name: Option<String>) -> Result<RoomMessageEventContent> {
	use service::resolver::CachedOverride;

	let mut out = String::new();
	writeln!(out, "| Server Name | IP  | Port | Expires |")?;
	writeln!(out, "| ----------- | --- | ----:| ------- |")?;
	let row = |(
		name,
		&CachedOverride {
			ref ips,
			port,
			expire,
		},
	)| {
		let expire = time::format(expire, "%+");
		writeln!(out, "| {name} | {ips:?} | {port} | {expire} |").expect("wrote line");
	};

	let map = services().resolver.overrides.read().expect("locked");

	if let Some(server_name) = server_name.as_ref() {
		map.get_key_value(server_name).map(row);
	} else {
		map.iter().for_each(row);
	}

	Ok(RoomMessageEventContent::notice_markdown(out))
}
