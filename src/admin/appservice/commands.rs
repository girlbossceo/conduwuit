use ruma::{api::appservice::Registration, events::room::message::RoomMessageEventContent};

use crate::{Result, admin_command};

#[admin_command]
pub(super) async fn register(&self) -> Result<RoomMessageEventContent> {
	if self.body.len() < 2
		|| !self.body[0].trim().starts_with("```")
		|| self.body.last().unwrap_or(&"").trim() != "```"
	{
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

	let appservice_config_body = self.body[1..self.body.len().checked_sub(1).unwrap()].join("\n");
	let parsed_config = serde_yaml::from_str::<Registration>(&appservice_config_body);
	match parsed_config {
		| Ok(registration) => match self
			.services
			.appservice
			.register_appservice(&registration, &appservice_config_body)
			.await
		{
			| Ok(()) => Ok(RoomMessageEventContent::text_plain(format!(
				"Appservice registered with ID: {}",
				registration.id
			))),
			| Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
				"Failed to register appservice: {e}"
			))),
		},
		| Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
			"Could not parse appservice config as YAML: {e}"
		))),
	}
}

#[admin_command]
pub(super) async fn unregister(
	&self,
	appservice_identifier: String,
) -> Result<RoomMessageEventContent> {
	match self
		.services
		.appservice
		.unregister_appservice(&appservice_identifier)
		.await
	{
		| Ok(()) => Ok(RoomMessageEventContent::text_plain("Appservice unregistered.")),
		| Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
			"Failed to unregister appservice: {e}"
		))),
	}
}

#[admin_command]
pub(super) async fn show_appservice_config(
	&self,
	appservice_identifier: String,
) -> Result<RoomMessageEventContent> {
	match self
		.services
		.appservice
		.get_registration(&appservice_identifier)
		.await
	{
		| Some(config) => {
			let config_str = serde_yaml::to_string(&config)
				.expect("config should've been validated on register");
			let output =
				format!("Config for {appservice_identifier}:\n\n```yaml\n{config_str}\n```",);
			Ok(RoomMessageEventContent::notice_markdown(output))
		},
		| None => Ok(RoomMessageEventContent::text_plain("Appservice does not exist.")),
	}
}

#[admin_command]
pub(super) async fn list_registered(&self) -> Result<RoomMessageEventContent> {
	let appservices = self.services.appservice.iter_ids().await;
	let output = format!("Appservices ({}): {}", appservices.len(), appservices.join(", "));
	Ok(RoomMessageEventContent::text_plain(output))
}
