use std::time::Duration;

use rand::Rng;
use ruma::{
	api::client::{error::ErrorKind, room::report_content},
	events::room::message,
	int,
};
use tokio::time::sleep;
use tracing::{debug, info};

use crate::{services, utils::HtmlEscape, Error, Result, Ruma};

/// # `POST /_matrix/client/v3/rooms/{roomId}/report/{eventId}`
///
/// Reports an inappropriate event to homeserver admins
pub(crate) async fn report_event_route(
	body: Ruma<report_content::v3::Request>,
) -> Result<report_content::v3::Response> {
	// user authentication
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	info!("Received /report request by user {}", sender_user);

	// check if we know about the reported event ID or if it's invalid
	let Some(pdu) = services().rooms.timeline.get_pdu(&body.event_id)? else {
		return Err(Error::BadRequest(
			ErrorKind::NotFound,
			"Event ID is not known to us or Event ID is invalid",
		));
	};

	// check if the room ID from the URI matches the PDU's room ID
	if body.room_id != pdu.room_id {
		return Err(Error::BadRequest(
			ErrorKind::NotFound,
			"Event ID does not belong to the reported room",
		));
	}

	// check if reporting user is in the reporting room
	if !services()
		.rooms
		.state_cache
		.room_members(&pdu.room_id)
		.filter_map(Result::ok)
		.any(|user_id| user_id == *sender_user)
	{
		return Err(Error::BadRequest(
			ErrorKind::NotFound,
			"You are not in the room you are reporting.",
		));
	}

	// check if score is in valid range
	if let Some(true) = body.score.map(|s| s > int!(0) || s < int!(-100)) {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Invalid score, must be within 0 to -100",
		));
	};

	// check if report reasoning is less than or equal to 750 characters
	if let Some(true) = body.reason.clone().map(|s| s.chars().count() >= 750) {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Reason too long, should be 750 characters or fewer",
		));
	};

	// send admin room message that we received the report with an @room ping for
	// urgency
	services()
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
		));

	// even though this is kinda security by obscurity, let's still make a small
	// random delay sending a successful response per spec suggestion regarding
	// enumerating for potential events existing in our server.
	let time_to_wait = rand::thread_rng().gen_range(8..21);
	debug!(
		"Got successful /report request, waiting {} seconds before sending successful response.",
		time_to_wait
	);
	sleep(Duration::from_secs(time_to_wait)).await;

	Ok(report_content::v3::Response {})
}
