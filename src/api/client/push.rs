use axum::extract::State;
use conduit::err;
use ruma::{
	api::client::{
		error::ErrorKind,
		push::{
			delete_pushrule, get_pushers, get_pushrule, get_pushrule_actions, get_pushrule_enabled, get_pushrules_all,
			set_pusher, set_pushrule, set_pushrule_actions, set_pushrule_enabled, RuleScope,
		},
	},
	events::{
		push_rules::{PushRulesEvent, PushRulesEventContent},
		GlobalAccountDataEventType,
	},
	push::{InsertPushRuleError, RemovePushRuleError, Ruleset},
	CanonicalJsonObject,
};
use service::Services;

use crate::{Error, Result, Ruma};

/// # `GET /_matrix/client/r0/pushrules/`
///
/// Retrieves the push rules event for this user.
pub(crate) async fn get_pushrules_all_route(
	State(services): State<crate::State>, body: Ruma<get_pushrules_all::v3::Request>,
) -> Result<get_pushrules_all::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let global_ruleset: Ruleset;

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await;

	let Ok(event) = event else {
		// user somehow has non-existent push rule event. recreate it and return server
		// default silently
		return recreate_push_rules_and_return(&services, sender_user).await;
	};

	let value = serde_json::from_str::<CanonicalJsonObject>(event.get())
		.map_err(|e| err!(Database(warn!("Invalid push rules account data event in database: {e}"))))?;

	let Some(content_value) = value.get("content") else {
		// user somehow has a push rule event with no content key, recreate it and
		// return server default silently
		return recreate_push_rules_and_return(&services, sender_user).await;
	};

	if content_value.to_string().is_empty() {
		// user somehow has a push rule event with empty content, recreate it and return
		// server default silently
		return recreate_push_rules_and_return(&services, sender_user).await;
	}

	let account_data_content = serde_json::from_value::<PushRulesEventContent>(content_value.clone().into())
		.map_err(|e| err!(Database(warn!("Invalid push rules account data event in database: {e}"))))?;

	global_ruleset = account_data_content.global;

	Ok(get_pushrules_all::v3::Response {
		global: global_ruleset,
	})
}

/// # `GET /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}`
///
/// Retrieves a single specified push rule for this user.
pub(crate) async fn get_pushrule_route(
	State(services): State<crate::State>, body: Ruma<get_pushrule::v3::Request>,
) -> Result<get_pushrule::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?
		.content;

	let rule = account_data
		.global
		.get(body.kind.clone(), &body.rule_id)
		.map(Into::into);

	if let Some(rule) = rule {
		Ok(get_pushrule::v3::Response {
			rule,
		})
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "Push rule not found."))
	}
}

/// # `PUT /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}`
///
/// Creates a single specified push rule for this user.
pub(crate) async fn set_pushrule_route(
	State(services): State<crate::State>, body: Ruma<set_pushrule::v3::Request>,
) -> Result<set_pushrule::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	let body = body.body;

	if body.scope != RuleScope::Global {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Scopes other than 'global' are not supported.",
		));
	}

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let mut account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?;

	if let Err(error) =
		account_data
			.content
			.global
			.insert(body.rule.clone(), body.after.as_deref(), body.before.as_deref())
	{
		let err = match error {
			InsertPushRuleError::ServerDefaultRuleId => Error::BadRequest(
				ErrorKind::InvalidParam,
				"Rule IDs starting with a dot are reserved for server-default rules.",
			),
			InsertPushRuleError::InvalidRuleId => {
				Error::BadRequest(ErrorKind::InvalidParam, "Rule ID containing invalid characters.")
			},
			InsertPushRuleError::RelativeToServerDefaultRule => Error::BadRequest(
				ErrorKind::InvalidParam,
				"Can't place a push rule relatively to a server-default rule.",
			),
			InsertPushRuleError::UnknownRuleId => {
				Error::BadRequest(ErrorKind::NotFound, "The before or after rule could not be found.")
			},
			InsertPushRuleError::BeforeHigherThanAfter => Error::BadRequest(
				ErrorKind::InvalidParam,
				"The before rule has a higher priority than the after rule.",
			),
			_ => Error::BadRequest(ErrorKind::InvalidParam, "Invalid data."),
		};

		return Err(err);
	}

	services
		.account_data
		.update(
			None,
			sender_user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(account_data).expect("to json value always works"),
		)
		.await?;

	Ok(set_pushrule::v3::Response {})
}

/// # `GET /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}/actions`
///
/// Gets the actions of a single specified push rule for this user.
pub(crate) async fn get_pushrule_actions_route(
	State(services): State<crate::State>, body: Ruma<get_pushrule_actions::v3::Request>,
) -> Result<get_pushrule_actions::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if body.scope != RuleScope::Global {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Scopes other than 'global' are not supported.",
		));
	}

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?
		.content;

	let global = account_data.global;
	let actions = global
		.get(body.kind.clone(), &body.rule_id)
		.map(|rule| rule.actions().to_owned())
		.ok_or(Error::BadRequest(ErrorKind::NotFound, "Push rule not found."))?;

	Ok(get_pushrule_actions::v3::Response {
		actions,
	})
}

/// # `PUT /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}/actions`
///
/// Sets the actions of a single specified push rule for this user.
pub(crate) async fn set_pushrule_actions_route(
	State(services): State<crate::State>, body: Ruma<set_pushrule_actions::v3::Request>,
) -> Result<set_pushrule_actions::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if body.scope != RuleScope::Global {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Scopes other than 'global' are not supported.",
		));
	}

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let mut account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?;

	if account_data
		.content
		.global
		.set_actions(body.kind.clone(), &body.rule_id, body.actions.clone())
		.is_err()
	{
		return Err(Error::BadRequest(ErrorKind::NotFound, "Push rule not found."));
	}

	services
		.account_data
		.update(
			None,
			sender_user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(account_data).expect("to json value always works"),
		)
		.await?;

	Ok(set_pushrule_actions::v3::Response {})
}

/// # `GET /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}/enabled`
///
/// Gets the enabled status of a single specified push rule for this user.
pub(crate) async fn get_pushrule_enabled_route(
	State(services): State<crate::State>, body: Ruma<get_pushrule_enabled::v3::Request>,
) -> Result<get_pushrule_enabled::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if body.scope != RuleScope::Global {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Scopes other than 'global' are not supported.",
		));
	}

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?;

	let global = account_data.content.global;
	let enabled = global
		.get(body.kind.clone(), &body.rule_id)
		.map(ruma::push::AnyPushRuleRef::enabled)
		.ok_or(Error::BadRequest(ErrorKind::NotFound, "Push rule not found."))?;

	Ok(get_pushrule_enabled::v3::Response {
		enabled,
	})
}

/// # `PUT /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}/enabled`
///
/// Sets the enabled status of a single specified push rule for this user.
pub(crate) async fn set_pushrule_enabled_route(
	State(services): State<crate::State>, body: Ruma<set_pushrule_enabled::v3::Request>,
) -> Result<set_pushrule_enabled::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if body.scope != RuleScope::Global {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Scopes other than 'global' are not supported.",
		));
	}

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let mut account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?;

	if account_data
		.content
		.global
		.set_enabled(body.kind.clone(), &body.rule_id, body.enabled)
		.is_err()
	{
		return Err(Error::BadRequest(ErrorKind::NotFound, "Push rule not found."));
	}

	services
		.account_data
		.update(
			None,
			sender_user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(account_data).expect("to json value always works"),
		)
		.await?;

	Ok(set_pushrule_enabled::v3::Response {})
}

/// # `DELETE /_matrix/client/r0/pushrules/{scope}/{kind}/{ruleId}`
///
/// Deletes a single specified push rule for this user.
pub(crate) async fn delete_pushrule_route(
	State(services): State<crate::State>, body: Ruma<delete_pushrule::v3::Request>,
) -> Result<delete_pushrule::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if body.scope != RuleScope::Global {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Scopes other than 'global' are not supported.",
		));
	}

	let event = services
		.account_data
		.get(None, sender_user, GlobalAccountDataEventType::PushRules.to_string().into())
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::NotFound, "PushRules event not found."))?;

	let mut account_data = serde_json::from_str::<PushRulesEvent>(event.get())
		.map_err(|_| Error::bad_database("Invalid account data event in db."))?;

	if let Err(error) = account_data
		.content
		.global
		.remove(body.kind.clone(), &body.rule_id)
	{
		let err = match error {
			RemovePushRuleError::ServerDefault => {
				Error::BadRequest(ErrorKind::InvalidParam, "Cannot delete a server-default pushrule.")
			},
			RemovePushRuleError::NotFound => Error::BadRequest(ErrorKind::NotFound, "Push rule not found."),
			_ => Error::BadRequest(ErrorKind::InvalidParam, "Invalid data."),
		};

		return Err(err);
	}

	services
		.account_data
		.update(
			None,
			sender_user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(account_data).expect("to json value always works"),
		)
		.await?;

	Ok(delete_pushrule::v3::Response {})
}

/// # `GET /_matrix/client/r0/pushers`
///
/// Gets all currently active pushers for the sender user.
pub(crate) async fn get_pushers_route(
	State(services): State<crate::State>, body: Ruma<get_pushers::v3::Request>,
) -> Result<get_pushers::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	Ok(get_pushers::v3::Response {
		pushers: services.pusher.get_pushers(sender_user).await,
	})
}

/// # `POST /_matrix/client/r0/pushers/set`
///
/// Adds a pusher for the sender user.
///
/// - TODO: Handle `append`
pub(crate) async fn set_pushers_route(
	State(services): State<crate::State>, body: Ruma<set_pusher::v3::Request>,
) -> Result<set_pusher::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	services.pusher.set_pusher(sender_user, &body.action);

	Ok(set_pusher::v3::Response::default())
}

/// user somehow has bad push rules, these must always exist per spec.
/// so recreate it and return server default silently
async fn recreate_push_rules_and_return(
	services: &Services, sender_user: &ruma::UserId,
) -> Result<get_pushrules_all::v3::Response> {
	services
		.account_data
		.update(
			None,
			sender_user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(PushRulesEvent {
				content: PushRulesEventContent {
					global: Ruleset::server_default(sender_user),
				},
			})
			.expect("to json always works"),
		)
		.await?;

	Ok(get_pushrules_all::v3::Response {
		global: Ruleset::server_default(sender_user),
	})
}
