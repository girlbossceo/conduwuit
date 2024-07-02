mod tests;

use std::{
	collections::VecDeque,
	fmt::{Display, Formatter},
	str::FromStr,
	sync::Arc,
};

use conduit::{debug_info, Server};
use database::Database;
use lru_cache::LruCache;
use ruma::{
	api::{
		client::{self, error::ErrorKind, space::SpaceHierarchyRoomsChunk},
		federation::{
			self,
			space::{SpaceHierarchyChildSummary, SpaceHierarchyParentSummary},
		},
	},
	events::{
		room::{
			avatar::RoomAvatarEventContent,
			canonical_alias::RoomCanonicalAliasEventContent,
			create::RoomCreateEventContent,
			join_rules::{JoinRule, RoomJoinRulesEventContent},
			topic::RoomTopicEventContent,
		},
		space::child::{HierarchySpaceChildEvent, SpaceChildEventContent},
		StateEventType,
	},
	serde::Raw,
	space::SpaceRoomJoinRule,
	OwnedRoomId, OwnedServerName, RoomId, ServerName, UInt, UserId,
};
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::{services, Error, Result};

pub struct CachedSpaceHierarchySummary {
	summary: SpaceHierarchyParentSummary,
}

pub enum SummaryAccessibility {
	Accessible(Box<SpaceHierarchyParentSummary>),
	Inaccessible,
}

// TODO: perhaps use some better form of token rather than just room count
#[derive(Debug, Eq, PartialEq)]
pub struct PaginationToken {
	/// Path down the hierarchy of the room to start the response at,
	/// excluding the root space.
	pub short_room_ids: Vec<u64>,
	pub limit: UInt,
	pub max_depth: UInt,
	pub suggested_only: bool,
}

impl FromStr for PaginationToken {
	type Err = Error;

	fn from_str(value: &str) -> Result<Self> {
		let mut values = value.split('_');

		let mut pag_tok = || {
			let mut rooms = vec![];

			for room in values.next()?.split(',') {
				rooms.push(u64::from_str(room).ok()?);
			}

			Some(Self {
				short_room_ids: rooms,
				limit: UInt::from_str(values.next()?).ok()?,
				max_depth: UInt::from_str(values.next()?).ok()?,
				suggested_only: {
					let slice = values.next()?;

					if values.next().is_none() {
						if slice == "true" {
							true
						} else if slice == "false" {
							false
						} else {
							None?
						}
					} else {
						None?
					}
				},
			})
		};

		if let Some(token) = pag_tok() {
			Ok(token)
		} else {
			Err(Error::BadRequest(ErrorKind::InvalidParam, "invalid token"))
		}
	}
}

impl Display for PaginationToken {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{}_{}_{}_{}",
			self.short_room_ids
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
				.join(","),
			self.limit,
			self.max_depth,
			self.suggested_only
		)
	}
}

/// Identifier used to check if rooms are accessible
///
/// None is used if you want to return the room, no matter if accessible or not
enum Identifier<'a> {
	UserId(&'a UserId),
	ServerName(&'a ServerName),
}

pub struct Service {
	pub roomid_spacehierarchy_cache: Mutex<LruCache<OwnedRoomId, Option<CachedSpaceHierarchySummary>>>,
}

// Here because cannot implement `From` across ruma-federation-api and
// ruma-client-api types
impl From<CachedSpaceHierarchySummary> for SpaceHierarchyRoomsChunk {
	fn from(value: CachedSpaceHierarchySummary) -> Self {
		let SpaceHierarchyParentSummary {
			canonical_alias,
			name,
			num_joined_members,
			room_id,
			topic,
			world_readable,
			guest_can_join,
			avatar_url,
			join_rule,
			room_type,
			children_state,
			..
		} = value.summary;

		Self {
			canonical_alias,
			name,
			num_joined_members,
			room_id,
			topic,
			world_readable,
			guest_can_join,
			avatar_url,
			join_rule,
			room_type,
			children_state,
		}
	}
}

impl Service {
	pub fn build(server: &Arc<Server>, _db: &Arc<Database>) -> Result<Self> {
		let config = &server.config;
		Ok(Self {
			roomid_spacehierarchy_cache: Mutex::new(LruCache::new(
				(f64::from(config.roomid_spacehierarchy_cache_capacity) * config.conduit_cache_capacity_modifier)
					as usize,
			)),
		})
	}

	/// Gets the response for the space hierarchy over federation request
	///
	/// Errors if the room does not exist, so a check if the room exists should
	/// be done
	pub async fn get_federation_hierarchy(
		&self, room_id: &RoomId, server_name: &ServerName, suggested_only: bool,
	) -> Result<federation::space::get_hierarchy::v1::Response> {
		match self
			.get_summary_and_children_local(&room_id.to_owned(), Identifier::ServerName(server_name))
			.await?
		{
			Some(SummaryAccessibility::Accessible(room)) => {
				let mut children = Vec::new();
				let mut inaccessible_children = Vec::new();

				for (child, _via) in get_parent_children_via(&room, suggested_only) {
					match self
						.get_summary_and_children_local(&child, Identifier::ServerName(server_name))
						.await?
					{
						Some(SummaryAccessibility::Accessible(summary)) => {
							children.push((*summary).into());
						},
						Some(SummaryAccessibility::Inaccessible) => {
							inaccessible_children.push(child);
						},
						None => (),
					}
				}

				Ok(federation::space::get_hierarchy::v1::Response {
					room: *room,
					children,
					inaccessible_children,
				})
			},
			Some(SummaryAccessibility::Inaccessible) => {
				Err(Error::BadRequest(ErrorKind::NotFound, "The requested room is inaccessible"))
			},
			None => Err(Error::BadRequest(ErrorKind::NotFound, "The requested room was not found")),
		}
	}

	/// Gets the summary of a space using solely local information
	async fn get_summary_and_children_local(
		&self, current_room: &OwnedRoomId, identifier: Identifier<'_>,
	) -> Result<Option<SummaryAccessibility>> {
		if let Some(cached) = self
			.roomid_spacehierarchy_cache
			.lock()
			.await
			.get_mut(&current_room.to_owned())
			.as_ref()
		{
			return Ok(if let Some(cached) = cached {
				if is_accessable_child(
					current_room,
					&cached.summary.join_rule,
					&identifier,
					&cached.summary.allowed_room_ids,
				) {
					Some(SummaryAccessibility::Accessible(Box::new(cached.summary.clone())))
				} else {
					Some(SummaryAccessibility::Inaccessible)
				}
			} else {
				None
			});
		}

		Ok(
			if let Some(children_pdus) = get_stripped_space_child_events(current_room).await? {
				let summary = Self::get_room_summary(current_room, children_pdus, &identifier);
				if let Ok(summary) = summary {
					self.roomid_spacehierarchy_cache.lock().await.insert(
						current_room.clone(),
						Some(CachedSpaceHierarchySummary {
							summary: summary.clone(),
						}),
					);

					Some(SummaryAccessibility::Accessible(Box::new(summary)))
				} else {
					None
				}
			} else {
				None
			},
		)
	}

	/// Gets the summary of a space using solely federation
	#[tracing::instrument(skip(self))]
	async fn get_summary_and_children_federation(
		&self, current_room: &OwnedRoomId, suggested_only: bool, user_id: &UserId, via: &[OwnedServerName],
	) -> Result<Option<SummaryAccessibility>> {
		for server in via {
			debug_info!("Asking {server} for /hierarchy");
			if let Ok(response) = services()
				.sending
				.send_federation_request(
					server,
					federation::space::get_hierarchy::v1::Request {
						room_id: current_room.to_owned(),
						suggested_only,
					},
				)
				.await
			{
				debug_info!("Got response from {server} for /hierarchy\n{response:?}");
				let summary = response.room.clone();

				self.roomid_spacehierarchy_cache.lock().await.insert(
					current_room.clone(),
					Some(CachedSpaceHierarchySummary {
						summary: summary.clone(),
					}),
				);

				for child in response.children {
					let mut guard = self.roomid_spacehierarchy_cache.lock().await;
					if !guard.contains_key(current_room) {
						guard.insert(
							current_room.clone(),
							Some(CachedSpaceHierarchySummary {
								summary: {
									let SpaceHierarchyChildSummary {
										canonical_alias,
										name,
										num_joined_members,
										room_id,
										topic,
										world_readable,
										guest_can_join,
										avatar_url,
										join_rule,
										room_type,
										allowed_room_ids,
									} = child;

									SpaceHierarchyParentSummary {
										canonical_alias,
										name,
										num_joined_members,
										room_id: room_id.clone(),
										topic,
										world_readable,
										guest_can_join,
										avatar_url,
										join_rule,
										room_type,
										children_state: get_stripped_space_child_events(&room_id).await?.unwrap(),
										allowed_room_ids,
									}
								},
							}),
						);
					}
				}
				if is_accessable_child(
					current_room,
					&response.room.join_rule,
					&Identifier::UserId(user_id),
					&response.room.allowed_room_ids,
				) {
					return Ok(Some(SummaryAccessibility::Accessible(Box::new(summary.clone()))));
				}

				return Ok(Some(SummaryAccessibility::Inaccessible));
			}
		}

		self.roomid_spacehierarchy_cache
			.lock()
			.await
			.insert(current_room.clone(), None);

		Ok(None)
	}

	/// Gets the summary of a space using either local or remote (federation)
	/// sources
	async fn get_summary_and_children_client(
		&self, current_room: &OwnedRoomId, suggested_only: bool, user_id: &UserId, via: &[OwnedServerName],
	) -> Result<Option<SummaryAccessibility>> {
		if let Ok(Some(response)) = self
			.get_summary_and_children_local(current_room, Identifier::UserId(user_id))
			.await
		{
			Ok(Some(response))
		} else {
			self.get_summary_and_children_federation(current_room, suggested_only, user_id, via)
				.await
		}
	}

	fn get_room_summary(
		current_room: &OwnedRoomId, children_state: Vec<Raw<HierarchySpaceChildEvent>>, identifier: &Identifier<'_>,
	) -> Result<SpaceHierarchyParentSummary, Error> {
		let room_id: &RoomId = current_room;

		let join_rule = services()
			.rooms
			.state_accessor
			.room_state_get(room_id, &StateEventType::RoomJoinRules, "")?
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

		let allowed_room_ids = services()
			.rooms
			.state_accessor
			.allowed_room_ids(join_rule.clone());

		if !is_accessable_child(current_room, &join_rule.clone().into(), identifier, &allowed_room_ids) {
			debug!("User is not allowed to see room {room_id}");
			// This error will be caught later
			return Err(Error::BadRequest(ErrorKind::forbidden(), "User is not allowed to see the room"));
		}

		let join_rule = join_rule.into();

		Ok(SpaceHierarchyParentSummary {
			canonical_alias: services()
				.rooms
				.state_accessor
				.room_state_get(room_id, &StateEventType::RoomCanonicalAlias, "")?
				.map_or(Ok(None), |s| {
					serde_json::from_str(s.content.get())
						.map(|c: RoomCanonicalAliasEventContent| c.alias)
						.map_err(|_| Error::bad_database("Invalid canonical alias event in database."))
				})?,
			name: services().rooms.state_accessor.get_name(room_id)?,
			num_joined_members: services()
				.rooms
				.state_cache
				.room_joined_count(room_id)?
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
				.room_state_get(room_id, &StateEventType::RoomTopic, "")?
				.map_or(Ok(None), |s| {
					serde_json::from_str(s.content.get())
						.map(|c: RoomTopicEventContent| Some(c.topic))
						.map_err(|_| {
							error!("Invalid room topic event in database for room {}", room_id);
							Error::bad_database("Invalid room topic event in database.")
						})
				})?,
			world_readable: services().rooms.state_accessor.is_world_readable(room_id)?,
			guest_can_join: services().rooms.state_accessor.guest_can_join(room_id)?,
			avatar_url: services()
                .rooms
                .state_accessor
                .room_state_get(room_id, &StateEventType::RoomAvatar, "")?
                .map(|s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomAvatarEventContent| c.url)
                        .map_err(|_| Error::bad_database("Invalid room avatar event in database."))
                })
                .transpose()?
                // url is now an Option<String> so we must flatten
                .flatten(),
			join_rule,
			room_type: services()
				.rooms
				.state_accessor
				.room_state_get(room_id, &StateEventType::RoomCreate, "")?
				.map(|s| {
					serde_json::from_str::<RoomCreateEventContent>(s.content.get()).map_err(|e| {
						error!("Invalid room create event in database: {}", e);
						Error::BadDatabase("Invalid room create event in database.")
					})
				})
				.transpose()?
				.and_then(|e| e.room_type),
			children_state,
			allowed_room_ids,
		})
	}

	pub async fn get_client_hierarchy(
		&self, sender_user: &UserId, room_id: &RoomId, limit: usize, short_room_ids: Vec<u64>, max_depth: usize,
		suggested_only: bool,
	) -> Result<client::space::get_hierarchy::v1::Response> {
		let mut parents = VecDeque::new();

		// Don't start populating the results if we have to start at a specific room.
		let mut populate_results = short_room_ids.is_empty();

		let mut stack = vec![vec![(
			room_id.to_owned(),
			match room_id.server_name() {
				Some(server_name) => vec![server_name.into()],
				None => vec![],
			},
		)]];

		let mut results = Vec::new();

		while let Some((current_room, via)) = { next_room_to_traverse(&mut stack, &mut parents) } {
			if limit > results.len() {
				match (
					self.get_summary_and_children_client(&current_room, suggested_only, sender_user, &via)
						.await?,
					current_room == room_id,
				) {
					(Some(SummaryAccessibility::Accessible(summary)), _) => {
						let mut children: Vec<(OwnedRoomId, Vec<OwnedServerName>)> =
							get_parent_children_via(&summary, suggested_only)
								.into_iter()
								.filter(|(room, _)| parents.iter().all(|parent| parent != room))
								.rev()
								.collect();

						if populate_results {
							results.push(summary_to_chunk(*summary.clone()));
						} else {
							children = children
                                .into_iter()
                                .rev()
                                .skip_while(|(room, _)| {
                                    if let Ok(short) = services().rooms.short.get_shortroomid(room)
                                    {
                                        short.as_ref() != short_room_ids.get(parents.len())
                                    } else {
                                        false
                                    }
                                })
                                .collect::<Vec<_>>()
                                // skip_while doesn't implement DoubleEndedIterator, which is needed for rev
                                .into_iter()
                                .rev()
                                .collect();

							if children.is_empty() {
								return Err(Error::BadRequest(
									ErrorKind::InvalidParam,
									"Room IDs in token were not found.",
								));
							}

							// We have reached the room after where we last left off
							if parents.len() + 1 == short_room_ids.len() {
								populate_results = true;
							}
						}

						if !children.is_empty() && parents.len() < max_depth {
							parents.push_back(current_room.clone());
							stack.push(children);
						}
						// Root room in the space hierarchy, we return an error
						// if this one fails.
					},
					(Some(SummaryAccessibility::Inaccessible), true) => {
						return Err(Error::BadRequest(ErrorKind::forbidden(), "The requested room is inaccessible"));
					},
					(None, true) => {
						return Err(Error::BadRequest(ErrorKind::forbidden(), "The requested room was not found"));
					},
					// Just ignore other unavailable rooms
					(None | Some(SummaryAccessibility::Inaccessible), false) => (),
				}
			} else {
				break;
			}
		}

		Ok(client::space::get_hierarchy::v1::Response {
			next_batch: if let Some((room, _)) = next_room_to_traverse(&mut stack, &mut parents) {
				parents.pop_front();
				parents.push_back(room);

				let mut short_room_ids = vec![];

				for room in parents {
					short_room_ids.push(services().rooms.short.get_or_create_shortroomid(&room)?);
				}

				Some(
					PaginationToken {
						short_room_ids,
						limit: UInt::new(max_depth as u64).expect("When sent in request it must have been valid UInt"),
						max_depth: UInt::new(max_depth as u64)
							.expect("When sent in request it must have been valid UInt"),
						suggested_only,
					}
					.to_string(),
				)
			} else {
				None
			},
			rooms: results,
		})
	}
}

fn next_room_to_traverse(
	stack: &mut Vec<Vec<(OwnedRoomId, Vec<OwnedServerName>)>>, parents: &mut VecDeque<OwnedRoomId>,
) -> Option<(OwnedRoomId, Vec<OwnedServerName>)> {
	while stack.last().map_or(false, Vec::is_empty) {
		stack.pop();
		parents.pop_back();
	}

	stack.last_mut().and_then(Vec::pop)
}

/// Simply returns the stripped m.space.child events of a room
async fn get_stripped_space_child_events(
	room_id: &RoomId,
) -> Result<Option<Vec<Raw<HierarchySpaceChildEvent>>>, Error> {
	if let Some(current_shortstatehash) = services().rooms.state.get_room_shortstatehash(room_id)? {
		let state = services()
			.rooms
			.state_accessor
			.state_full_ids(current_shortstatehash)
			.await?;
		let mut children_pdus = Vec::new();
		for (key, id) in state {
			let (event_type, state_key) = services().rooms.short.get_statekey_from_short(key)?;
			if event_type != StateEventType::SpaceChild {
				continue;
			}

			let pdu = services()
				.rooms
				.timeline
				.get_pdu(&id)?
				.ok_or_else(|| Error::bad_database("Event in space state not found"))?;

			if serde_json::from_str::<SpaceChildEventContent>(pdu.content.get())
				.ok()
				.map(|c| c.via)
				.map_or(true, |v| v.is_empty())
			{
				continue;
			}

			if OwnedRoomId::try_from(state_key).is_ok() {
				children_pdus.push(pdu.to_stripped_spacechild_state_event());
			}
		}
		Ok(Some(children_pdus))
	} else {
		Ok(None)
	}
}

/// With the given identifier, checks if a room is accessable
fn is_accessable_child(
	current_room: &OwnedRoomId, join_rule: &SpaceRoomJoinRule, identifier: &Identifier<'_>,
	allowed_room_ids: &Vec<OwnedRoomId>,
) -> bool {
	// Note: unwrap_or_default for bool means false
	match identifier {
		Identifier::ServerName(server_name) => {
			let room_id: &RoomId = current_room;

			// Checks if ACLs allow for the server to participate
			if services()
				.rooms
				.event_handler
				.acl_check(server_name, room_id)
				.is_err()
			{
				return false;
			}
		},
		Identifier::UserId(user_id) => {
			if services()
				.rooms
				.state_cache
				.is_joined(user_id, current_room)
				.unwrap_or_default()
				|| services()
					.rooms
					.state_cache
					.is_invited(user_id, current_room)
					.unwrap_or_default()
			{
				return true;
			}
		},
	} // Takes care of join rules
	match join_rule {
		SpaceRoomJoinRule::Restricted => {
			for room in allowed_room_ids {
				match identifier {
					Identifier::UserId(user) => {
						if services()
							.rooms
							.state_cache
							.is_joined(user, room)
							.unwrap_or_default()
						{
							return true;
						}
					},
					Identifier::ServerName(server) => {
						if services()
							.rooms
							.state_cache
							.server_in_room(server, room)
							.unwrap_or_default()
						{
							return true;
						}
					},
				}
			}
			false
		},
		SpaceRoomJoinRule::Public | SpaceRoomJoinRule::Knock | SpaceRoomJoinRule::KnockRestricted => true,
		// Invite only, Private, or Custom join rule
		_ => false,
	}
}

/// Here because cannot implement `From` across ruma-federation-api and
/// ruma-client-api types
fn summary_to_chunk(summary: SpaceHierarchyParentSummary) -> SpaceHierarchyRoomsChunk {
	let SpaceHierarchyParentSummary {
		canonical_alias,
		name,
		num_joined_members,
		room_id,
		topic,
		world_readable,
		guest_can_join,
		avatar_url,
		join_rule,
		room_type,
		children_state,
		..
	} = summary;

	SpaceHierarchyRoomsChunk {
		canonical_alias,
		name,
		num_joined_members,
		room_id,
		topic,
		world_readable,
		guest_can_join,
		avatar_url,
		join_rule,
		room_type,
		children_state,
	}
}

/// Returns the children of a SpaceHierarchyParentSummary, making use of the
/// children_state field
fn get_parent_children_via(
	parent: &SpaceHierarchyParentSummary, suggested_only: bool,
) -> Vec<(OwnedRoomId, Vec<OwnedServerName>)> {
	parent
		.children_state
		.iter()
		.filter_map(|raw_ce| {
			raw_ce.deserialize().map_or(None, |ce| {
				if suggested_only && !ce.content.suggested {
					None
				} else {
					Some((ce.state_key, ce.content.via))
				}
			})
		})
		.collect()
}
