use std::sync::Arc;

use async_trait::async_trait;
use conduit::{error, warn, Result};
use ruma::{
	events::{push_rules::PushRulesEventContent, GlobalAccountDataEvent, GlobalAccountDataEventType},
	push::Ruleset,
};

use crate::{account_data, globals, users, Dep};

pub struct Service {
	services: Services,
}

struct Services {
	account_data: Dep<account_data::Service>,
	globals: Dep<globals::Service>,
	users: Dep<users::Service>,
}

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				account_data: args.depend::<account_data::Service>("account_data"),
				globals: args.depend::<globals::Service>("globals"),
				users: args.depend::<users::Service>("users"),
			},
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		self.set_emergency_access()
			.await
			.inspect_err(|e| error!("Could not set the configured emergency password for the conduit user: {e}"))?;

		Ok(())
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Sets the emergency password and push rules for the @conduit account in
	/// case emergency password is set
	async fn set_emergency_access(&self) -> Result<bool> {
		let conduit_user = &self.services.globals.server_user;

		self.services
			.users
			.set_password(conduit_user, self.services.globals.emergency_password().as_deref())?;

		let (ruleset, pwd_set) = match self.services.globals.emergency_password() {
			Some(_) => (Ruleset::server_default(conduit_user), true),
			None => (Ruleset::new(), false),
		};

		self.services
			.account_data
			.update(
				None,
				conduit_user,
				GlobalAccountDataEventType::PushRules.to_string().into(),
				&serde_json::to_value(&GlobalAccountDataEvent {
					content: PushRulesEventContent {
						global: ruleset,
					},
				})
				.expect("to json value always works"),
			)
			.await?;

		if pwd_set {
			warn!(
				"The server account emergency password is set! Please unset it as soon as you finish admin account \
				 recovery! You will be logged out of the server service account when you finish."
			);
		} else {
			// logs out any users still in the server service account and removes sessions
			self.services.users.deactivate_account(conduit_user).await?;
		}

		Ok(pwd_set)
	}
}
