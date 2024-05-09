#![cfg(feature = "mods")]
#[cfg(not(any(clippy, debug_assertions, doctest, test)))]
compile_error!("Feature 'mods' is only available in developer builds");

use std::{
	future::Future,
	pin::Pin,
	sync::{atomic::Ordering, Arc},
};

use conduit::{mods, Error, Result};
use tracing::{debug, error};

use crate::Server;

type RunFuncResult = Pin<Box<dyn Future<Output = Result<(), Error>>>>;
type RunFuncProto = fn(&Arc<conduit::Server>) -> RunFuncResult;

const RESTART_THRESH: &str = "conduit_service";
const MODULE_NAMES: &[&str] = &[
	//"conduit_core",
	"conduit_database",
	"conduit_service",
	"conduit_api",
	"conduit_admin",
	"conduit_router",
];

#[cfg(feature = "panic_trap")]
conduit::mod_init! {{
	conduit::debug::set_panic_trap();
}}

pub(crate) async fn run(server: &Arc<Server>, starts: bool) -> Result<(bool, bool), Error> {
	let main_lock = server.mods.read().await;
	let main_mod = (*main_lock).last().expect("main module loaded");
	if starts {
		let start = main_mod.get::<RunFuncProto>("start")?;
		if let Err(error) = start(&server.server).await {
			error!("Starting server: {error}");
			return Err(error);
		}
	}
	let run = main_mod.get::<RunFuncProto>("run")?;
	if let Err(error) = run(&server.server).await {
		error!("Running server: {error}");
		return Err(error);
	}
	let reloads = server.server.reload.swap(false, Ordering::AcqRel);
	let stops = !reloads || stale(server).await? <= restart_thresh();
	let starts = reloads && stops;
	if stops {
		let stop = main_mod.get::<RunFuncProto>("stop")?;
		if let Err(error) = stop(&server.server).await {
			error!("Stopping server: {error}");
			return Err(error);
		}
	}

	Ok((starts, reloads))
}

pub(crate) async fn open(server: &Arc<Server>) -> Result<usize, Error> {
	let mut mods_lock = server.mods.write().await;
	let mods: &mut Vec<mods::Module> = &mut mods_lock;
	debug!(
		available = %available(),
		loaded = %mods.len(),
		"Loading modules",
	);

	for (i, name) in MODULE_NAMES.iter().enumerate() {
		if mods.get(i).is_none() {
			mods.push(mods::Module::from_name(name)?);
		}
	}

	Ok(mods.len())
}

pub(crate) async fn close(server: &Arc<Server>, force: bool) -> Result<usize, Error> {
	let stale = stale_count(server).await;
	let mut mods_lock = server.mods.write().await;
	let mods: &mut Vec<mods::Module> = &mut mods_lock;
	debug!(
		available = %available(),
		loaded = %mods.len(),
		stale = %stale,
		force,
		"Unloading modules",
	);

	while mods.last().is_some() {
		let module = &mods.last().expect("module");
		if force || module.deleted()? {
			mods.pop();
		} else {
			break;
		}
	}

	Ok(mods.len())
}

async fn stale_count(server: &Arc<Server>) -> usize {
	let watermark = stale(server).await.unwrap_or(available());
	available() - watermark
}

async fn stale(server: &Arc<Server>) -> Result<usize, Error> {
	let mods_lock = server.mods.read().await;
	let mods: &Vec<mods::Module> = &mods_lock;
	for (i, module) in mods.iter().enumerate() {
		if module.deleted()? {
			return Ok(i);
		}
	}

	Ok(mods.len())
}

fn restart_thresh() -> usize {
	MODULE_NAMES
		.iter()
		.position(|&name| name.ends_with(RESTART_THRESH))
		.unwrap_or(MODULE_NAMES.len())
}

const fn available() -> usize { MODULE_NAMES.len() }
