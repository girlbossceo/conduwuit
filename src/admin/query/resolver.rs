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
async fn destinations_cache(
	&self,
	server_name: Option<OwnedServerName>,
) -> Result<RoomMessageEventContent> {
	use service::resolver::cache::CachedDest;

	writeln!(self, "| Server Name | Destination | Hostname | Expires |").await?;
	writeln!(self, "| ----------- | ----------- | -------- | ------- |").await?;

	let mut out = String::new();
	{
		let map = self
			.services
			.resolver
			.cache
			.destinations
			.read()
			.expect("locked");

		for (name, &CachedDest { ref dest, ref host, expire }) in map.iter() {
			if let Some(server_name) = server_name.as_ref() {
				if name != server_name {
					continue;
				}
			}

			let expire = time::format(expire, "%+");
			writeln!(out, "| {name} | {dest} | {host} | {expire} |")?;
		}
	}

	self.write_str(out.as_str()).await?;

	Ok(RoomMessageEventContent::notice_plain(""))
}

#[admin_command]
async fn overrides_cache(&self, server_name: Option<String>) -> Result<RoomMessageEventContent> {
	use service::resolver::cache::CachedOverride;

	writeln!(self, "| Server Name | IP  | Port | Expires |").await?;
	writeln!(self, "| ----------- | --- | ----:| ------- |").await?;

	let mut out = String::new();
	{
		let map = self
			.services
			.resolver
			.cache
			.overrides
			.read()
			.expect("locked");

		for (name, &CachedOverride { ref ips, port, expire }) in map.iter() {
			if let Some(server_name) = server_name.as_ref() {
				if name != server_name {
					continue;
				}
			}

			let expire = time::format(expire, "%+");
			writeln!(out, "| {name} | {ips:?} | {port} | {expire} |")?;
		}
	}

	self.write_str(out.as_str()).await?;

	Ok(RoomMessageEventContent::notice_plain(""))
}
