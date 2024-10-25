use std::time::Duration;

use axum::extract::State;
use conduit::{utils::ReadyExt, Err};
use rand::Rng;
use ruma::{
	api::client::{error::ErrorKind, room::report_content},
	events::room::message,
	int, EventId, RoomId, UserId,
};
use tokio::time::sleep;
use tracing::info;

use crate::{
	debug_info,
	service::{pdu::PduEvent, Services},
	utils::HtmlEscape,
	Error, Result, Ruma,
};

/// # `POST /_matrix/client/v3/rooms/{roomId}/report/{eventId}`
///
/// Reports an inappropriate event to homeserver admins
pub(crate) async fn report_event_route(
	State(services): State<crate::State>, body: Ruma<report_content::v3::Request>,
) -> Result<report_content::v3::Response> {
	// user authentication
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	info!(
		"Received /report request by user {sender_user} for room {} and event ID {}",
		body.room_id, body.event_id
	);

	delay_response().await;

	// check if we know about the reported event ID or if it's invalid
	let Ok(pdu) = services.rooms.timeline.get_pdu(&body.event_id).await else {
		return Err!(Request(NotFound("Event ID is not known to us or Event ID is invalid")));
	};

	is_report_valid(
		&services,
		&pdu.event_id,
		&body.room_id,
		sender_user,
		&body.reason,
		body.score,
		&pdu,
	)
	.await?;

	// send admin room message that we received the report with an @room ping for
	// urgency
	services
		.admin
		.send_message(message::RoomMessageEventContent::text_html(
			format!(
				"@room Report received from: {}\n\nEvent ID: {}\nRoom ID: {}\nSent By: {}\n\nReport Score: {}\nReport \
				 Reason: {}",
				sender_user.to_owned(),
				pdu.event_id,
				pdu.room_id,
				pdu.sender.clone(),
				body.score.unwrap_or_else(|| ruma::Int::from(0)),
				body.reason.as_deref().unwrap_or("")
			),
			format!(
				"<details><summary>@room Report received from: <a href=\"https://matrix.to/#/{0}\">{0}\
                </a></summary><ul><li>Event Info<ul><li>Event ID: <code>{1}</code>\
                <a href=\"https://matrix.to/#/{2}/{1}\">🔗</a></li><li>Room ID: <code>{2}</code>\
                </li><li>Sent By: <a href=\"https://matrix.to/#/{3}\">{3}</a></li></ul></li><li>\
                Report Info<ul><li>Report Score: {4}</li><li>Report Reason: {5}</li></ul></li>\
                </ul></details>",
				sender_user.to_owned(),
				pdu.event_id.clone(),
				pdu.room_id.clone(),
				pdu.sender.clone(),
				body.score.unwrap_or_else(|| ruma::Int::from(0)),
				HtmlEscape(body.reason.as_deref().unwrap_or(""))
			),
		))
		.await
		.ok();

	Ok(report_content::v3::Response {})
}

/// in the following order:
///
/// check if the room ID from the URI matches the PDU's room ID
/// check if score is in valid range
/// check if report reasoning is less than or equal to 750 characters
/// check if reporting user is in the reporting room
async fn is_report_valid(
	services: &Services, event_id: &EventId, room_id: &RoomId, sender_user: &UserId, reason: &Option<String>,
	score: Option<ruma::Int>, pdu: &std::sync::Arc<PduEvent>,
) -> Result<()> {
	debug_info!("Checking if report from user {sender_user} for event {event_id} in room {room_id} is valid");

	if room_id != pdu.room_id {
		return Err(Error::BadRequest(
			ErrorKind::NotFound,
			"Event ID does not belong to the reported room",
		));
	}

	if score.is_some_and(|s| s > int!(0) || s < int!(-100)) {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Invalid score, must be within 0 to -100",
		));
	};

	if reason.as_ref().is_some_and(|s| s.len() > 750) {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Reason too long, should be 750 characters or fewer",
		));
	};

	if !services
		.rooms
		.state_cache
		.room_members(room_id)
		.ready_any(|user_id| user_id == sender_user)
		.await
	{
		return Err(Error::BadRequest(
			ErrorKind::NotFound,
			"You are not in the room you are reporting.",
		));
	}

	Ok(())
}

/// even though this is kinda security by obscurity, let's still make a small
/// random delay sending a response per spec suggestion regarding
/// enumerating for potential events existing in our server.
async fn delay_response() {
	let time_to_wait = rand::thread_rng().gen_range(3..10);
	debug_info!("Got successful /report request, waiting {time_to_wait} seconds before sending successful response.");
	sleep(Duration::from_secs(time_to_wait)).await;
}
