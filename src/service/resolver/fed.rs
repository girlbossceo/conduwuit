use std::{
	borrow::Cow,
	fmt,
	net::{IpAddr, SocketAddr},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FedDest {
	Literal(SocketAddr),
	Named(String, String),
}

pub(crate) fn get_ip_with_port(dest_str: &str) -> Option<FedDest> {
	if let Ok(dest) = dest_str.parse::<SocketAddr>() {
		Some(FedDest::Literal(dest))
	} else if let Ok(ip_addr) = dest_str.parse::<IpAddr>() {
		Some(FedDest::Literal(SocketAddr::new(ip_addr, 8448)))
	} else {
		None
	}
}

pub(crate) fn add_port_to_hostname(dest_str: &str) -> FedDest {
	let (host, port) = match dest_str.find(':') {
		None => (dest_str, ":8448"),
		Some(pos) => dest_str.split_at(pos),
	};

	FedDest::Named(host.to_owned(), port.to_owned())
}

impl FedDest {
	pub(crate) fn https_string(&self) -> String {
		match self {
			Self::Literal(addr) => format!("https://{addr}"),
			Self::Named(host, port) => format!("https://{host}{port}"),
		}
	}

	pub(crate) fn uri_string(&self) -> String {
		match self {
			Self::Literal(addr) => addr.to_string(),
			Self::Named(host, port) => format!("{host}{port}"),
		}
	}

	#[inline]
	pub(crate) fn hostname(&self) -> Cow<'_, str> {
		match &self {
			Self::Literal(addr) => addr.ip().to_string().into(),
			Self::Named(host, _) => host.into(),
		}
	}

	#[inline]
	#[allow(clippy::string_slice)]
	pub(crate) fn port(&self) -> Option<u16> {
		match &self {
			Self::Literal(addr) => Some(addr.port()),
			Self::Named(_, port) => port[1..].parse().ok(),
		}
	}
}

impl fmt::Display for FedDest {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.uri_string().as_str()) }
}
