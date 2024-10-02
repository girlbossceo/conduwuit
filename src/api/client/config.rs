use axum::extract::State;
use conduit::err;
use ruma::{
	api::client::{
		config::{get_global_account_data, get_room_account_data, set_global_account_data, set_room_account_data},
		error::ErrorKind,
	},
	events::{AnyGlobalAccountDataEventContent, AnyRoomAccountDataEventContent},
	serde::Raw,
	OwnedUserId, RoomId,
};
use serde::Deserialize;
use serde_json::{json, value::RawValue as RawJsonValue};

use crate::{service::Services, Error, Result, Ruma};

/// # `PUT /_matrix/client/r0/user/{userId}/account_data/{type}`
///
/// Sets some account data for the sender user.
pub(crate) async fn set_global_account_data_route(
	State(services): State<crate::State>, body: Ruma<set_global_account_data::v3::Request>,
) -> Result<set_global_account_data::v3::Response> {
	set_account_data(
		&services,
		None,
		&body.sender_user,
		&body.event_type.to_string(),
		body.data.json(),
	)
	.await?;

	Ok(set_global_account_data::v3::Response {})
}

/// # `PUT /_matrix/client/r0/user/{userId}/rooms/{roomId}/account_data/{type}`
///
/// Sets some room account data for the sender user.
pub(crate) async fn set_room_account_data_route(
	State(services): State<crate::State>, body: Ruma<set_room_account_data::v3::Request>,
) -> Result<set_room_account_data::v3::Response> {
	set_account_data(
		&services,
		Some(&body.room_id),
		&body.sender_user,
		&body.event_type.to_string(),
		body.data.json(),
	)
	.await?;

	Ok(set_room_account_data::v3::Response {})
}

/// # `GET /_matrix/client/r0/user/{userId}/account_data/{type}`
///
/// Gets some account data for the sender user.
pub(crate) async fn get_global_account_data_route(
	State(services): State<crate::State>, body: Ruma<get_global_account_data::v3::Request>,
) -> Result<get_global_account_data::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let account_data: ExtractGlobalEventContent = services
		.account_data
		.get_global(sender_user, body.event_type.clone())
		.await
		.map_err(|_| err!(Request(NotFound("Data not found."))))?;

	Ok(get_global_account_data::v3::Response {
		account_data: account_data.content,
	})
}

/// # `GET /_matrix/client/r0/user/{userId}/rooms/{roomId}/account_data/{type}`
///
/// Gets some room account data for the sender user.
pub(crate) async fn get_room_account_data_route(
	State(services): State<crate::State>, body: Ruma<get_room_account_data::v3::Request>,
) -> Result<get_room_account_data::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let account_data: ExtractRoomEventContent = services
		.account_data
		.get_room(&body.room_id, sender_user, body.event_type.clone())
		.await
		.map_err(|_| err!(Request(NotFound("Data not found."))))?;

	Ok(get_room_account_data::v3::Response {
		account_data: account_data.content,
	})
}

async fn set_account_data(
	services: &Services, room_id: Option<&RoomId>, sender_user: &Option<OwnedUserId>, event_type: &str,
	data: &RawJsonValue,
) -> Result<()> {
	let sender_user = sender_user.as_ref().expect("user is authenticated");

	let data: serde_json::Value =
		serde_json::from_str(data.get()).map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Data is invalid."))?;

	services
		.account_data
		.update(
			room_id,
			sender_user,
			event_type.into(),
			&json!({
				"type": event_type,
				"content": data,
			}),
		)
		.await?;

	Ok(())
}

#[derive(Deserialize)]
struct ExtractRoomEventContent {
	content: Raw<AnyRoomAccountDataEventContent>,
}

#[derive(Deserialize)]
struct ExtractGlobalEventContent {
	content: Raw<AnyGlobalAccountDataEventContent>,
}
