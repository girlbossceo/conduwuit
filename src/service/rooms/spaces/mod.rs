use std::sync::{Arc, Mutex};

use lru_cache::LruCache;
use ruma::{
    api::{
        client::{
            error::ErrorKind,
            space::{get_hierarchy, SpaceHierarchyRoomsChunk, SpaceRoomJoinRule},
        },
        federation,
    },
    directory::PublicRoomJoinRule,
    events::{
        room::{
            avatar::RoomAvatarEventContent,
            canonical_alias::RoomCanonicalAliasEventContent,
            create::RoomCreateEventContent,
            guest_access::{GuestAccess, RoomGuestAccessEventContent},
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            name::RoomNameEventContent,
            topic::RoomTopicEventContent,
        },
        StateEventType,
    },
    OwnedRoomId, RoomId, UserId,
};

use tracing::{debug, error, warn};

use crate::{services, Error, PduEvent, Result};

pub struct CachedSpaceChunk {
    chunk: SpaceHierarchyRoomsChunk,
    children: Vec<OwnedRoomId>,
    join_rule: JoinRule,
}

pub struct Service {
    pub roomid_spacechunk_cache: Mutex<LruCache<OwnedRoomId, Option<CachedSpaceChunk>>>,
}

impl Service {
    pub async fn get_hierarchy(
        &self,
        sender_user: &UserId,
        room_id: &RoomId,
        limit: usize,
        skip: usize,
        max_depth: usize,
        suggested_only: bool,
    ) -> Result<get_hierarchy::v1::Response> {
        let mut left_to_skip = skip;

        let mut rooms_in_path = Vec::new();
        let mut stack = vec![vec![room_id.to_owned()]];
        let mut results = Vec::new();

        while let Some(current_room) = {
            while stack.last().map_or(false, |s| s.is_empty()) {
                stack.pop();
            }
            if !stack.is_empty() {
                stack.last_mut().and_then(|s| s.pop())
            } else {
                None
            }
        } {
            rooms_in_path.push(current_room.clone());
            if results.len() >= limit {
                break;
            }

            if let Some(cached) = self
                .roomid_spacechunk_cache
                .lock()
                .unwrap()
                .get_mut(&current_room.to_owned())
                .as_ref()
            {
                if let Some(cached) = cached {
                    if let Some(_join_rule) =
                        self.handle_join_rule(&cached.join_rule, sender_user, &current_room)?
                    {
                        if left_to_skip > 0 {
                            left_to_skip -= 1;
                        } else {
                            results.push(cached.chunk.clone());
                        }
                        if rooms_in_path.len() < max_depth {
                            stack.push(cached.children.clone());
                        }
                    }
                }
                continue;
            }

            if let Some(current_shortstatehash) = services()
                .rooms
                .state
                .get_room_shortstatehash(&current_room)?
            {
                let state = services()
                    .rooms
                    .state_accessor
                    .state_full_ids(current_shortstatehash)
                    .await?;

                let mut children_ids = Vec::new();
                let mut children_pdus = Vec::new();
                for (key, id) in state {
                    let (event_type, state_key) =
                        services().rooms.short.get_statekey_from_short(key)?;
                    if event_type != StateEventType::SpaceChild {
                        continue;
                    }
                    if let Ok(room_id) = OwnedRoomId::try_from(state_key) {
                        children_ids.push(room_id);
                        children_pdus.push(services().rooms.timeline.get_pdu(&id)?.ok_or_else(
                            || Error::bad_database("Event in space state not found"),
                        )?);
                    }
                }

                // TODO: Sort children
                children_ids.reverse();

                let chunk = self.get_room_chunk(sender_user, &current_room, children_pdus);
                if let Ok(chunk) = chunk {
                    if left_to_skip > 0 {
                        left_to_skip -= 1;
                    } else {
                        results.push(chunk.clone());
                    }
                    let join_rule = services()
                        .rooms
                        .state_accessor
                        .room_state_get(&current_room, &StateEventType::RoomJoinRules, "")?
                        .map(|s| {
                            serde_json::from_str(s.content.get())
                                .map(|c: RoomJoinRulesEventContent| c.join_rule)
                                .map_err(|e| {
                                    error!("Invalid room join rule event in database: {}", e);
                                    Error::BadDatabase("Invalid room join rule event in database.")
                                })
                        })
                        .transpose()?
                        .unwrap_or(JoinRule::Invite);

                    self.roomid_spacechunk_cache.lock().unwrap().insert(
                        current_room.clone(),
                        Some(CachedSpaceChunk {
                            chunk,
                            children: children_ids.clone(),
                            join_rule,
                        }),
                    );
                }

                if rooms_in_path.len() < max_depth {
                    stack.push(children_ids);
                }
            } else {
                let server = current_room.server_name();
                if server == services().globals.server_name() {
                    continue;
                }
                if !results.is_empty() {
                    // Early return so the client can see some data already
                    break;
                }
                warn!("Asking {server} for /hierarchy");
                if let Ok(response) = services()
                    .sending
                    .send_federation_request(
                        &server,
                        federation::space::get_hierarchy::v1::Request {
                            room_id: current_room.to_owned(),
                            suggested_only,
                        },
                    )
                    .await
                {
                    warn!("Got response from {server} for /hierarchy\n{response:?}");
                    let join_rule = self.translate_pjoinrule(&response.room.join_rule)?;
                    let chunk = SpaceHierarchyRoomsChunk {
                        canonical_alias: response.room.canonical_alias,
                        name: response.room.name,
                        num_joined_members: response.room.num_joined_members,
                        room_id: response.room.room_id,
                        topic: response.room.topic,
                        world_readable: response.room.world_readable,
                        guest_can_join: response.room.guest_can_join,
                        avatar_url: response.room.avatar_url,
                        join_rule: self.translate_sjoinrule(&response.room.join_rule)?,
                        room_type: response.room.room_type,
                        children_state: response.room.children_state,
                    };
                    let children = response
                        .children
                        .iter()
                        .map(|c| c.room_id.clone())
                        .collect::<Vec<_>>();

                    if let Some(_join_rule) =
                        self.handle_join_rule(&join_rule, sender_user, &current_room)?
                    {
                        if left_to_skip > 0 {
                            left_to_skip -= 1;
                        } else {
                            results.push(chunk.clone());
                        }
                        if rooms_in_path.len() < max_depth {
                            stack.push(children.clone());
                        }
                    }

                    self.roomid_spacechunk_cache.lock().unwrap().insert(
                        current_room.clone(),
                        Some(CachedSpaceChunk {
                            chunk,
                            children,
                            join_rule,
                        }),
                    );

                    /* TODO:
                    for child in response.children {
                        roomid_spacechunk_cache.insert(
                            current_room.clone(),
                            CachedSpaceChunk {
                                chunk: child.chunk,
                                children,
                                join_rule,
                            },
                        );
                    }
                    */
                } else {
                    self.roomid_spacechunk_cache
                        .lock()
                        .unwrap()
                        .insert(current_room.clone(), None);
                }
            }
        }

        Ok(get_hierarchy::v1::Response {
            next_batch: if results.is_empty() {
                None
            } else {
                Some((skip + results.len()).to_string())
            },
            rooms: results,
        })
    }

    fn get_room_chunk(
        &self,
        sender_user: &UserId,
        room_id: &RoomId,
        children: Vec<Arc<PduEvent>>,
    ) -> Result<SpaceHierarchyRoomsChunk> {
        Ok(SpaceHierarchyRoomsChunk {
            canonical_alias: services()
                .rooms
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomCanonicalAlias, "")?
                .map_or(Ok(None), |s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomCanonicalAliasEventContent| c.alias)
                        .map_err(|_| {
                            Error::bad_database("Invalid canonical alias event in database.")
                        })
                })?,
            name: services().rooms.state_accessor.get_name(&room_id)?,
            num_joined_members: services()
                .rooms
                .state_cache
                .room_joined_count(&room_id)?
                .unwrap_or_else(|| {
                    warn!("Room {} has no member count", room_id);
                    0
                })
                .try_into()
                .expect("user count should not be that big"),
            room_id: room_id.to_owned(),
            topic: services()
                .rooms
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomTopic, "")?
                .map_or(Ok(None), |s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomTopicEventContent| Some(c.topic))
                        .map_err(|_| Error::bad_database("Invalid room topic event in database."))
                })?,
            world_readable: services()
                .rooms
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomHistoryVisibility, "")?
                .map_or(Ok(false), |s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomHistoryVisibilityEventContent| {
                            c.history_visibility == HistoryVisibility::WorldReadable
                        })
                        .map_err(|_| {
                            Error::bad_database(
                                "Invalid room history visibility event in database.",
                            )
                        })
                })?,
            guest_can_join: services()
                .rooms
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomGuestAccess, "")?
                .map_or(Ok(false), |s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomGuestAccessEventContent| {
                            c.guest_access == GuestAccess::CanJoin
                        })
                        .map_err(|_| {
                            Error::bad_database("Invalid room guest access event in database.")
                        })
                })?,
            avatar_url: services()
                .rooms
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomAvatar, "")?
                .map(|s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomAvatarEventContent| c.url)
                        .map_err(|_| Error::bad_database("Invalid room avatar event in database."))
                })
                .transpose()?
                // url is now an Option<String> so we must flatten
                .flatten(),
            join_rule: {
                let join_rule = services()
                    .rooms
                    .state_accessor
                    .room_state_get(&room_id, &StateEventType::RoomJoinRules, "")?
                    .map(|s| {
                        serde_json::from_str(s.content.get())
                            .map(|c: RoomJoinRulesEventContent| c.join_rule)
                            .map_err(|e| {
                                error!("Invalid room join rule event in database: {}", e);
                                Error::BadDatabase("Invalid room join rule event in database.")
                            })
                    })
                    .transpose()?
                    .unwrap_or(JoinRule::Invite);
                self.handle_join_rule(&join_rule, sender_user, room_id)?
                    .ok_or_else(|| {
                        debug!("User is not allowed to see room {room_id}");
                        // This error will be caught later
                        Error::BadRequest(
                            ErrorKind::Forbidden,
                            "User is not allowed to see the room",
                        )
                    })?
            },
            room_type: services()
                .rooms
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomCreate, "")?
                .map(|s| {
                    serde_json::from_str::<RoomCreateEventContent>(s.content.get()).map_err(|e| {
                        error!("Invalid room create event in database: {}", e);
                        Error::BadDatabase("Invalid room create event in database.")
                    })
                })
                .transpose()?
                .and_then(|e| e.room_type),
            children_state: children
                .into_iter()
                .map(|pdu| pdu.to_stripped_spacechild_state_event())
                .collect(),
        })
    }

    fn translate_pjoinrule(&self, join_rule: &PublicRoomJoinRule) -> Result<JoinRule> {
        match join_rule {
            PublicRoomJoinRule::Knock => Ok(JoinRule::Knock),
            PublicRoomJoinRule::Public => Ok(JoinRule::Public),
            _ => Err(Error::BadServerResponse("Unknown join rule")),
        }
    }

    fn translate_sjoinrule(&self, join_rule: &PublicRoomJoinRule) -> Result<SpaceRoomJoinRule> {
        match join_rule {
            PublicRoomJoinRule::Knock => Ok(SpaceRoomJoinRule::Knock),
            PublicRoomJoinRule::Public => Ok(SpaceRoomJoinRule::Public),
            _ => Err(Error::BadServerResponse("Unknown join rule")),
        }
    }

    fn handle_join_rule(
        &self,
        join_rule: &JoinRule,
        sender_user: &UserId,
        room_id: &RoomId,
    ) -> Result<Option<SpaceRoomJoinRule>> {
        match join_rule {
            JoinRule::Public => Ok::<_, Error>(Some(SpaceRoomJoinRule::Public)),
            JoinRule::Knock => Ok(Some(SpaceRoomJoinRule::Knock)),
            JoinRule::Invite => {
                if services()
                    .rooms
                    .state_cache
                    .is_joined(sender_user, &room_id)?
                {
                    Ok(Some(SpaceRoomJoinRule::Invite))
                } else {
                    Ok(None)
                }
            }
            JoinRule::Restricted(_r) => {
                // TODO: Check rules
                Ok(None)
            }
            JoinRule::KnockRestricted(_r) => {
                // TODO: Check rules
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
