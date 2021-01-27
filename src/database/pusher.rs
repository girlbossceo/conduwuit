use crate::{Error, PduEvent, Result};
use ruma::{
    api::client::r0::push::{Pusher, PusherKind},
    events::{
        room::{
            member::MemberEventContent,
            message::{MessageEventContent, TextMessageEventContent},
        },
        EventType,
    },
    push::{PushCondition, Ruleset},
    UserId,
};

#[derive(Debug, Clone)]
pub struct PushData {
    /// UserId + pushkey -> Pusher
    pub(super) senderkey_pusher: sled::Tree,
}

impl PushData {
    pub fn new(db: &sled::Db) -> Result<Self> {
        Ok(Self {
            senderkey_pusher: db.open_tree("senderkey_pusher")?,
        })
    }

    pub fn set_pusher(&self, sender: &UserId, pusher: Pusher) -> Result<()> {
        let mut key = sender.as_bytes().to_vec();
        key.extend_from_slice(pusher.pushkey.as_bytes());

        self.senderkey_pusher.insert(
            key,
            &*serde_json::to_string(&pusher).expect("Pusher is valid JSON string"),
        )?;

        Ok(())
    }

    pub fn get_pusher(&self, sender: &UserId) -> Result<Vec<Pusher>> {
        self.senderkey_pusher
            .scan_prefix(sender.as_bytes())
            .values()
            .map(|push: std::result::Result<sled::IVec, _>| {
                let push = push.map_err(|_| Error::bad_database("Invalid push bytes in db."))?;
                Ok(serde_json::from_slice(&*push)
                    .map_err(|_| Error::bad_database("Invalid Pusher in db."))?)
            })
            .collect::<Result<Vec<_>>>()
    }
}

pub async fn send_push_notice(
    user: &UserId,
    pusher: &Pusher,
    ruleset: Ruleset,
    pdu: &PduEvent,
) -> Result<()> {
    for rule in ruleset.into_iter() {
        // TODO: can actions contain contradictory Actions
        if rule
            .actions
            .iter()
            .any(|act| matches!(act, ruma::push::Action::DontNotify))
            || !rule.enabled
        {
            continue;
        }

        match rule.rule_id.as_str() {
            ".m.rule.master" => {}
            ".m.rule.suppress_notices" => {}
            ".m.rule.invite_for_me" => {}
            ".m.rule.member_event" => {
                if let EventType::RoomMember = &pdu.kind {
                    // TODO use this?
                    let _member = serde_json::from_value::<MemberEventContent>(pdu.content.clone())
                        .map_err(|_| Error::bad_database("PDU contained bad message content"))?;
                    if let Some(conditions) = rule.conditions {
                        if conditions.iter().any(|cond| match cond {
                            PushCondition::EventMatch { key, pattern } => {
                                let mut json =
                                    serde_json::to_value(pdu).expect("PDU is valid JSON");
                                for key in key.split('.') {
                                    json = json[key].clone();
                                }
                                // TODO: this is baddddd
                                json.to_string().contains(pattern)
                            }
                            _ => false,
                        }) {}
                    }
                }
            }
            ".m.rule.contains_display_name" => {
                if let EventType::RoomMessage = &pdu.kind {
                    let msg_content =
                        serde_json::from_value::<MessageEventContent>(pdu.content.clone())
                            .map_err(|_| {
                                Error::bad_database("PDU contained bad message content")
                            })?;
                    if let MessageEventContent::Text(TextMessageEventContent { body, .. }) =
                        &msg_content
                    {
                        if body.contains(user.localpart()) {
                            send_notice(user, &pusher, &pdu).await?;
                        }
                    }
                }
            }
            ".m.rule.tombstone" => {}
            ".m.rule.roomnotif" => {}
            ".m.rule.contains_user_name" => {
                if let EventType::RoomMessage = &pdu.kind {
                    let msg_content =
                        serde_json::from_value::<MessageEventContent>(pdu.content.clone())
                            .map_err(|_| {
                                Error::bad_database("PDU contained bad message content")
                            })?;
                    if let MessageEventContent::Text(TextMessageEventContent { body, .. }) =
                        &msg_content
                    {
                        if body.contains(user.localpart()) {
                            send_notice(user, &pusher, &pdu).await?;
                        }
                    }
                }
            }
            ".m.rule.call" => {}
            ".m.rule.encrypted_room_one_to_one" => {}
            ".m.rule.room_one_to_one" => {}
            ".m.rule.message" => {}
            ".m.rule.encrypted" => {}
            _ => {}
        }
    }
    Ok(())
}

async fn send_notice(_sender: &UserId, pusher: &Pusher, _event: &PduEvent) -> Result<()> {
    if let Some(PusherKind::Http) = pusher.kind {
        log::error!("YAHOOO");
    } else {
        // EMAIL
        todo!("send an email")
    }
    Ok(())
}
