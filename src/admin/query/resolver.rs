use std::fmt::Write;

use clap::Subcommand;
use conduwuit::{utils::time, Result};
use ruma::{events::room::message::RoomMessageEventContent, OwnedServerName};

use crate::{admin_command, admin_command_dispatch};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
/// Resolver service and caches
pub(crate) enum ResolverCommand {
	/// Query the destinations cache
	DestinationsCache {
		server_name: Option<OwnedServerName>,
	},

	/// Query the overrides cache
	OverridesCache {
		name: Option<String>,
	},
}

#[admin_command]
async fn destinations_cache(&self, server_name: Option<OwnedServerName>) -> Result<RoomMessageEventContent> {
	use service::resolver::cache::CachedDest;

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

	let map = self
		.services
		.resolver
		.cache
		.destinations
		.read()
		.expect("locked");

	if let Some(server_name) = server_name.as_ref() {
		map.get_key_value(server_name).map(row);
	} else {
		map.iter().for_each(row);
	}

	Ok(RoomMessageEventContent::notice_markdown(out))
}

#[admin_command]
async fn overrides_cache(&self, server_name: Option<String>) -> Result<RoomMessageEventContent> {
	use service::resolver::cache::CachedOverride;

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

	let map = self
		.services
		.resolver
		.cache
		.overrides
		.read()
		.expect("locked");

	if let Some(server_name) = server_name.as_ref() {
		map.get_key_value(server_name).map(row);
	} else {
		map.iter().for_each(row);
	}

	Ok(RoomMessageEventContent::notice_markdown(out))
}
