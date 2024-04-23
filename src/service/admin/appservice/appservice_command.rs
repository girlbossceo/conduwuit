use ruma::{api::appservice::Registration, events::room::message::RoomMessageEventContent};

use crate::{service::admin::escape_html, services, Result};

pub(crate) async fn register(body: Vec<&str>) -> Result<RoomMessageEventContent> {
	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let appservice_config = body[1..body.len() - 1].join("\n");
		let parsed_config = serde_yaml::from_str::<Registration>(&appservice_config);
		match parsed_config {
			Ok(yaml) => match services().appservice.register_appservice(yaml).await {
				Ok(id) => Ok(RoomMessageEventContent::text_plain(format!(
					"Appservice registered with ID: {id}."
				))),
				Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
					"Failed to register appservice: {e}"
				))),
			},
			Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
				"Could not parse appservice config: {e}"
			))),
		}
	} else {
		Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		))
	}
}

pub(crate) async fn unregister(_body: Vec<&str>, appservice_identifier: String) -> Result<RoomMessageEventContent> {
	match services()
		.appservice
		.unregister_appservice(&appservice_identifier)
		.await
	{
		Ok(()) => Ok(RoomMessageEventContent::text_plain("Appservice unregistered.")),
		Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
			"Failed to unregister appservice: {e}"
		))),
	}
}

pub(crate) async fn show(_body: Vec<&str>, appservice_identifier: String) -> Result<RoomMessageEventContent> {
	match services()
		.appservice
		.get_registration(&appservice_identifier)
		.await
	{
		Some(config) => {
			let config_str = serde_yaml::to_string(&config).expect("config should've been validated on register");
			let output = format!("Config for {}:\n\n```yaml\n{}\n```", appservice_identifier, config_str,);
			let output_html = format!(
				"Config for {}:\n\n<pre><code class=\"language-yaml\">{}</code></pre>",
				escape_html(&appservice_identifier),
				escape_html(&config_str),
			);
			Ok(RoomMessageEventContent::text_html(output, output_html))
		},
		None => Ok(RoomMessageEventContent::text_plain("Appservice does not exist.")),
	}
}

pub(crate) async fn list(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let appservices = services().appservice.iter_ids().await;
	let output = format!("Appservices ({}): {}", appservices.len(), appservices.join(", "));
	Ok(RoomMessageEventContent::text_plain(output))
}
