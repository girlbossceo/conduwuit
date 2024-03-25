use std::{
	collections::HashMap,
	error::Error as StdError,
	future::{self},
	iter,
	net::{IpAddr, SocketAddr},
	sync::{Arc, RwLock as StdRwLock},
};

use futures_util::FutureExt;
use hickory_resolver::TokioAsyncResolver;
use hyper::{
	client::connect::dns::{GaiResolver, Name},
	service::Service as HyperService,
};
use reqwest::dns::{Addrs, Resolve, Resolving};
use ruma::OwnedServerName;
use tokio::sync::RwLock;
use tracing::error;

use crate::{api::server_server::FedDest, Config, Error};

pub type WellKnownMap = HashMap<OwnedServerName, (FedDest, String)>;
pub type TlsNameMap = HashMap<String, (Vec<IpAddr>, u16)>;

pub struct Resolver {
	inner: GaiResolver,
	pub overrides: Arc<StdRwLock<TlsNameMap>>,
	pub destinations: Arc<RwLock<WellKnownMap>>, // actual_destination, host
	pub resolver: TokioAsyncResolver,
}

impl Resolver {
	pub(crate) fn new(_config: &Config) -> Self {
		Resolver {
			inner: GaiResolver::new(),
			overrides: Arc::new(StdRwLock::new(TlsNameMap::new())),
			destinations: Arc::new(RwLock::new(WellKnownMap::new())),
			resolver: TokioAsyncResolver::tokio_from_system_conf()
				.map_err(|e| {
					error!("Failed to set up trust dns resolver with system config: {}", e);
					Error::bad_config("Failed to set up trust dns resolver with system config.")
				})
				.unwrap(),
		}
	}
}

impl Resolve for Resolver {
	fn resolve(&self, name: Name) -> Resolving {
		self.overrides
			.read()
			.unwrap()
			.get(name.as_str())
			.and_then(|(override_name, port)| {
				override_name.first().map(|first_name| {
					let x: Box<dyn Iterator<Item = SocketAddr> + Send> =
						Box::new(iter::once(SocketAddr::new(*first_name, *port)));
					let x: Resolving = Box::pin(future::ready(Ok(x)));
					x
				})
			})
			.unwrap_or_else(|| {
				let this = &mut self.inner.clone();
				Box::pin(HyperService::<Name>::call(this, name).map(|result| {
					result
						.map(|addrs| -> Addrs { Box::new(addrs) })
						.map_err(|err| -> Box<dyn StdError + Send + Sync> { Box::new(err) })
				}))
			})
	}
}
