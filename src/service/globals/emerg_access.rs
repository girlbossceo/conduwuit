use conduit::Result;
use ruma::{
	events::{push_rules::PushRulesEventContent, GlobalAccountDataEvent, GlobalAccountDataEventType},
	push::Ruleset,
	UserId,
};
use tracing::{error, warn};

use crate::services;

/// Set emergency access for the conduit user
pub(crate) fn init_emergency_access() {
	if let Err(e) = set_emergency_access() {
		error!("Could not set the configured emergency password for the conduit user: {e}");
	}
}

/// Sets the emergency password and push rules for the @conduit account in case
/// emergency password is set
fn set_emergency_access() -> Result<bool> {
	let conduit_user = UserId::parse_with_server_name("conduit", services().globals.server_name())
		.expect("@conduit:server_name is a valid UserId");

	services()
		.users
		.set_password(&conduit_user, services().globals.emergency_password().as_deref())?;

	let (ruleset, pwd_set) = match services().globals.emergency_password() {
		Some(_) => (Ruleset::server_default(&conduit_user), true),
		None => (Ruleset::new(), false),
	};

	services().account_data.update(
		None,
		&conduit_user,
		GlobalAccountDataEventType::PushRules.to_string().into(),
		&serde_json::to_value(&GlobalAccountDataEvent {
			content: PushRulesEventContent {
				global: ruleset,
			},
		})
		.expect("to json value always works"),
	)?;

	if pwd_set {
		warn!(
			"The Conduit account emergency password is set! Please unset it as soon as you finish admin account \
			 recovery!"
		);
	}

	Ok(pwd_set)
}
