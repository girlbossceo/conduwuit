use std::{
	collections::HashMap,
	net::IpAddr,
	sync::{Arc, RwLock},
	time::SystemTime,
};

use arrayvec::ArrayVec;
use conduwuit::{
	trace,
	utils::{math::Expected, rand},
};
use ruma::{OwnedServerName, ServerName};

use super::fed::FedDest;

pub struct Cache {
	pub destinations: RwLock<WellKnownMap>, // actual_destination, host
	pub overrides: RwLock<TlsNameMap>,
}

#[derive(Clone, Debug)]
pub struct CachedDest {
	pub dest: FedDest,
	pub host: String,
	pub expire: SystemTime,
}

#[derive(Clone, Debug)]
pub struct CachedOverride {
	pub ips: IpAddrs,
	pub port: u16,
	pub expire: SystemTime,
}

pub type WellKnownMap = HashMap<OwnedServerName, CachedDest>;
pub type TlsNameMap = HashMap<String, CachedOverride>;

pub type IpAddrs = ArrayVec<IpAddr, MAX_IPS>;
pub(crate) const MAX_IPS: usize = 3;

impl Cache {
	pub(super) fn new() -> Arc<Self> {
		Arc::new(Self {
			destinations: RwLock::new(WellKnownMap::new()),
			overrides: RwLock::new(TlsNameMap::new()),
		})
	}
}

impl super::Service {
	pub fn set_cached_destination(
		&self,
		name: OwnedServerName,
		dest: CachedDest,
	) -> Option<CachedDest> {
		trace!(?name, ?dest, "set cached destination");
		self.cache
			.destinations
			.write()
			.expect("locked for writing")
			.insert(name, dest)
	}

	#[must_use]
	pub fn get_cached_destination(&self, name: &ServerName) -> Option<CachedDest> {
		self.cache
			.destinations
			.read()
			.expect("locked for reading")
			.get(name)
			.cloned()
	}

	pub fn set_cached_override(
		&self,
		name: &str,
		over: CachedOverride,
	) -> Option<CachedOverride> {
		trace!(?name, ?over, "set cached override");
		self.cache
			.overrides
			.write()
			.expect("locked for writing")
			.insert(name.into(), over)
	}

	#[must_use]
	pub fn get_cached_override(&self, name: &str) -> Option<CachedOverride> {
		self.cache
			.overrides
			.read()
			.expect("locked for reading")
			.get(name)
			.cloned()
	}

	#[must_use]
	pub fn has_cached_override(&self, name: &str) -> bool {
		self.cache
			.overrides
			.read()
			.expect("locked for reading")
			.contains_key(name)
	}
}

impl CachedDest {
	#[inline]
	#[must_use]
	pub fn valid(&self) -> bool { true }

	//pub fn valid(&self) -> bool { self.expire > SystemTime::now() }

	#[must_use]
	pub(crate) fn default_expire() -> SystemTime {
		rand::timepoint_secs(60 * 60 * 18..60 * 60 * 36)
	}

	#[inline]
	#[must_use]
	pub fn size(&self) -> usize {
		self.dest
			.size()
			.expected_add(self.host.len())
			.expected_add(size_of_val(&self.expire))
	}
}

impl CachedOverride {
	#[inline]
	#[must_use]
	pub fn valid(&self) -> bool { true }

	//pub fn valid(&self) -> bool { self.expire > SystemTime::now() }

	#[must_use]
	pub(crate) fn default_expire() -> SystemTime {
		rand::timepoint_secs(60 * 60 * 6..60 * 60 * 12)
	}

	#[inline]
	#[must_use]
	pub fn size(&self) -> usize { size_of_val(self) }
}
