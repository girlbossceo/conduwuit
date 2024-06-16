#![cfg(unix)]

use std::{env, os::unix::process::CommandExt, process::Command};

use conduit::{debug, info};

pub(super) fn restart() -> ! {
	let exe = env::current_exe().expect("program path must be identified and available");
	let envs = env::vars();
	let args = env::args().skip(1);
	debug!(?exe, ?args, ?envs, "Restart");

	info!("Restart");

	let error = Command::new(exe).args(args).envs(envs).exec();
	panic!("{error:?}");
}
