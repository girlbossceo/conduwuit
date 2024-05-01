use std::{
	fmt::{Display, Formatter},
	str::FromStr,
};

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
			guest_access::{GuestAccess, RoomGuestAccessEventContent},
			history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
			join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent, RoomMembership},
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

use crate::{debug_info, services, utils::server_name::server_is_ours, Error, Result};

pub(crate) struct CachedSpaceHierarchySummary {
	summary: SpaceHierarchyParentSummary,
}

enum SummaryAccessibility {
	Accessible(Box<SpaceHierarchyParentSummary>),
	Inaccessible,
}

struct Arena {
	nodes: Vec<Node>,
	max_depth: usize,
	first_untraversed: Option<NodeId>,
}

struct Node {
	parent: Option<NodeId>,
	// Next meaning:
	//   -->
	// o o o o
	next_sibling: Option<NodeId>,
	// First meaning:
	// |
	// v
	// o o o o
	first_child: Option<NodeId>,
	room_id: OwnedRoomId,
	via: Vec<OwnedServerName>,
	traversed: bool,
}

#[derive(Clone, Copy, PartialEq, Debug, PartialOrd)]
struct NodeId {
	index: usize,
}

impl Arena {
	/// Checks if a given node is traversed
	fn traversed(&self, id: NodeId) -> Option<bool> { Some(self.get(id)?.traversed) }

	/// Gets the previous sibling of a given node
	fn next_sibling(&self, id: NodeId) -> Option<NodeId> { self.get(id)?.next_sibling }

	/// Gets the parent of a given node
	fn parent(&self, id: NodeId) -> Option<NodeId> { self.get(id)?.parent }

	/// Gets the last child of a given node
	fn first_child(&self, id: NodeId) -> Option<NodeId> { self.get(id)?.first_child }

	/// Sets traversed to true for a given node
	fn traverse(&mut self, id: NodeId) { self.nodes[id.index].traversed = true; }

	/// Gets the node of a given id
	fn get(&self, id: NodeId) -> Option<&Node> { self.nodes.get(id.index) }

	/// Gets a mutable reference of a node of a given id
	fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> { self.nodes.get_mut(id.index) }

	/// Returns the first untraversed node, marking it as traversed in the
	/// process
	fn first_untraversed(&mut self) -> Option<NodeId> {
		if self.nodes.is_empty() {
			None
		} else if let Some(untraversed) = self.first_untraversed {
			let mut current = untraversed;

			self.traverse(untraversed);

			// Possible paths:
			// 1) Next child exists, and hence is not traversed
			// 2) Next child does not exist, so go to the parent, then repeat
			// 3) If both the parent and child do not exist, then we have just traversed the
			//    whole space tree.
			//
			// You should only ever encounter a traversed node when going up through parents
			while let Some(true) = self.traversed(current) {
				if let Some(next) = self.next_sibling(current) {
					current = next;
				} else if let Some(parent) = self.parent(current) {
					current = parent;
				} else {
					break;
				}
			}

			// Traverses down the children until it reaches one without children
			while let Some(child) = self.first_child(current) {
				current = child;
			}

			if self.traversed(current)? {
				self.first_untraversed = None;
			} else {
				self.first_untraversed = Some(current);
			}

			Some(untraversed)
		} else {
			None
		}
	}

	/// Adds all the given nodes as children of the parent node
	fn push(&mut self, parent: NodeId, mut children: Vec<(OwnedRoomId, Vec<OwnedServerName>)>) {
		if children.is_empty() {
			self.traverse(parent);
		} else if self.nodes.get(parent.index).is_some() {
			let mut parents = vec![(
				parent,
				self.get(parent)
                    .expect("It is some, as above")
                    .room_id
                    // Cloning cause otherwise when iterating over the parents, below, there would
                    // be a mutable and immutable reference to self.nodes
                    .clone(),
			)];

			while let Some(parent) = self.parent(parents.last().expect("Has at least one value, as above").0) {
				parents.push((
					parent,
					self.get(parent)
						.expect("It is some, as above")
						.room_id
						.clone(),
				));
			}

			// If at max_depth, don't add new rooms
			if self.max_depth < parents.len() {
				return;
			}

			children.reverse();

			let mut next_id = None;

			for (child, via) in children {
				// Prevent adding a child which is a parent (recursion)
				if !parents.iter().any(|parent| parent.1 == child) {
					self.nodes.push(Node {
						parent: Some(parent),
						next_sibling: next_id,
						first_child: None,
						room_id: child,
						traversed: false,
						via,
					});

					next_id = Some(NodeId {
						index: self.nodes.len() - 1,
					});
				}
			}

			if self.first_untraversed.is_none()
				|| parent
					>= self
						.first_untraversed
						.expect("Should have already continued if none")
			{
				self.first_untraversed = next_id;
			}

			self.traverse(parent);

			// This is done as if we use an if-let above, we cannot reference self.nodes
			// above as then we would have multiple mutable references
			let node = self
				.get_mut(parent)
				.expect("Must be some, as inside this block");

			node.first_child = next_id;
		}
	}

	fn new(root: OwnedRoomId, max_depth: usize) -> Self {
		let zero_depth = max_depth == 0;

		Arena {
			nodes: vec![Node {
				parent: None,
				next_sibling: None,
				first_child: None,
				room_id: root,
				traversed: zero_depth,
				via: vec![],
			}],
			max_depth,
			first_untraversed: if zero_depth {
				None
			} else {
				Some(NodeId {
					index: 0,
				})
			},
		}
	}
}

// Note: perhaps use some better form of token rather than just room count
#[derive(Debug, PartialEq)]
pub(crate) struct PagnationToken {
	pub(crate) skip: UInt,
	pub(crate) limit: UInt,
	pub(crate) max_depth: UInt,
	pub(crate) suggested_only: bool,
}

impl FromStr for PagnationToken {
	type Err = Error;

	fn from_str(value: &str) -> Result<Self> {
		let mut values = value.split('_');

		let mut pag_tok = || {
			Some(PagnationToken {
				skip: UInt::from_str(values.next()?).ok()?,
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

impl Display for PagnationToken {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}_{}_{}_{}", self.skip, self.limit, self.max_depth, self.suggested_only)
	}
}

/// Identifier used to check if rooms are accessible
///
/// None is used if you want to return the room, no matter if accessible or not
enum Identifier<'a> {
	UserId(&'a UserId),
	ServerName(&'a ServerName),
	None,
}

pub(crate) struct Service {
	pub(crate) roomid_spacehierarchy_cache: Mutex<LruCache<OwnedRoomId, Option<CachedSpaceHierarchySummary>>>,
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
}

impl Service {
	///Gets the response for the space hierarchy over federation request
	///
	///Panics if the room does not exist, so a check if the room exists should
	/// be done
	pub(crate) async fn get_federation_hierarchy(
		&self, room_id: &RoomId, server_name: &ServerName, suggested_only: bool,
	) -> Result<federation::space::get_hierarchy::v1::Response> {
		match self
			.get_summary_and_children_local(&room_id.to_owned(), Identifier::None)
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
				)? {
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

	async fn get_summary_and_children_federation(
		&self, current_room: &OwnedRoomId, suggested_only: bool, user_id: &UserId, via: &[OwnedServerName],
	) -> Result<Option<SummaryAccessibility>> {
		debug_info!("servers via for federation hierarchy: {via:?}");

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
				)? {
					return Ok(Some(SummaryAccessibility::Accessible(Box::new(summary.clone()))));
				}

				return Ok(Some(SummaryAccessibility::Inaccessible));
			}

			self.roomid_spacehierarchy_cache
				.lock()
				.await
				.insert(current_room.clone(), None);
		}
		Ok(None)
	}

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

		let allowed_room_ids = allowed_room_ids(join_rule.clone());

		if !is_accessable_child(current_room, &join_rule.clone().into(), identifier, &allowed_room_ids)? {
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
				})
				.unwrap_or(None),
			world_readable: world_readable(room_id)?,
			guest_can_join: guest_can_join(room_id)?,
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

	// TODO: make this a lot less messy
	pub(crate) async fn get_client_hierarchy(
		&self, sender_user: &UserId, room_id: &RoomId, limit: usize, skip: usize, max_depth: usize,
		suggested_only: bool,
	) -> Result<client::space::get_hierarchy::v1::Response> {
		// try to find more servers to fetch hierachy from if the only
		// choice is the room ID's server name (usually dead)
		//
		// all spaces are normal rooms, so they should always have at least
		// 1 admin in it which has a far higher chance of their server still
		// being alive
		let power_levels: ruma::events::room::power_levels::RoomPowerLevelsEventContent = services()
			.rooms
			.state_accessor
			.room_state_get(room_id, &StateEventType::RoomPowerLevels, "")?
			.map(|ev| {
				serde_json::from_str(ev.content.get())
					.map_err(|_| Error::bad_database("invalid m.room.power_levels event"))
			})
			.transpose()?
			.unwrap_or_default();

		// add server names of the list of admins in the room for backfill server
		let mut via = power_levels
			.users
			.iter()
			.filter(|(_, level)| **level > power_levels.users_default)
			.map(|(user_id, _)| user_id.server_name())
			.filter(|server| !server_is_ours(server))
			.map(ToOwned::to_owned)
			.collect::<Vec<_>>();

		if let Some(server_name) = room_id.server_name() {
			via.push(server_name.to_owned());
		}

		debug_info!("servers via for hierarchy: {via:?}");

		match self
			.get_summary_and_children_client(&room_id.to_owned(), suggested_only, sender_user, &via)
			.await?
		{
			Some(SummaryAccessibility::Accessible(summary)) => {
				let mut left_to_skip = skip;
				let mut arena = Arena::new(summary.room_id.clone(), max_depth);

				let mut results = Vec::new();
				let root = arena
					.first_untraversed()
					.expect("The node just added is not traversed");

				arena.push(root, get_parent_children_via(&summary, suggested_only));
				if left_to_skip > 0 {
					left_to_skip -= 1;
				} else {
					results.push(summary_to_chunk(*summary.clone()));
				}

				while let Some(current_room) = arena.first_untraversed() {
					if limit > results.len() {
						let node = arena
							.get(current_room)
							.expect("We added this node, it must exist");
						if let Some(SummaryAccessibility::Accessible(summary)) = self
							.get_summary_and_children_client(&node.room_id, suggested_only, sender_user, &node.via)
							.await?
						{
							let children = get_parent_children_via(&summary, suggested_only);
							arena.push(current_room, children);

							if left_to_skip > 0 {
								left_to_skip -= 1;
							} else {
								results.push(summary_to_chunk(*summary.clone()));
							}
						}
					} else {
						break;
					}
				}

				Ok(client::space::get_hierarchy::v1::Response {
					next_batch: if results.len() < limit {
						None
					} else {
						let skip = UInt::new((skip + limit) as u64);

						skip.map(|skip| {
							PagnationToken {
								skip,
								limit: UInt::new(max_depth as u64)
									.expect("When sent in request it must have been valid UInt"),
								max_depth: UInt::new(max_depth as u64)
									.expect("When sent in request it must have been valid UInt"),
								suggested_only,
							}
							.to_string()
						})
					},
					rooms: results,
				})
			},
			Some(SummaryAccessibility::Inaccessible) => {
				Err(Error::BadRequest(ErrorKind::forbidden(), "The requested room is inaccessible"))
			},
			None => Err(Error::BadRequest(ErrorKind::forbidden(), "The requested room was not found")),
		}
	}
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
) -> Result<bool, Error> {
	is_accessable_child_recurse(current_room, join_rule, identifier, allowed_room_ids, 0)
}

fn is_accessable_child_recurse(
	current_room: &OwnedRoomId, join_rule: &SpaceRoomJoinRule, identifier: &Identifier<'_>,
	allowed_room_ids: &Vec<OwnedRoomId>, recurse_num: usize,
) -> Result<bool, Error> {
	// Set limit at 10, as we cannot keep going up parents forever
	// and it is very unlikely to have 10 space parents
	if recurse_num < 10 {
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
					return Ok(false);
				}
			},
			Identifier::UserId(user_id) => {
				if services()
					.rooms
					.state_cache
					.is_joined(user_id, current_room)?
					|| services()
						.rooms
						.state_cache
						.is_invited(user_id, current_room)?
				{
					return Ok(true);
				}
			},
			Identifier::None => (),
		} // Takes care of joinrules
		Ok(match join_rule {
			SpaceRoomJoinRule::Restricted => {
				for room in allowed_room_ids {
					if let Ok((join_rule, allowed_room_ids)) = get_join_rule(room) {
						if let Ok(true) = is_accessable_child_recurse(
							room,
							&join_rule,
							identifier,
							&allowed_room_ids,
							recurse_num + 1,
						) {
							return Ok(true);
						}
					}
				}
				false
			},
			SpaceRoomJoinRule::Public | SpaceRoomJoinRule::Knock | SpaceRoomJoinRule::KnockRestricted => true,
			// Custom join rules, Invite, or Private
			_ => false,
		})
	} else {
		// If you need to go up 10 parents, we just assume it is inaccessable
		Ok(false)
	}
}

/// Checks if guests are able to join a given room
fn guest_can_join(room_id: &RoomId) -> Result<bool, Error> {
	services()
		.rooms
		.state_accessor
		.room_state_get(room_id, &StateEventType::RoomGuestAccess, "")?
		.map_or(Ok(false), |s| {
			serde_json::from_str(s.content.get())
				.map(|c: RoomGuestAccessEventContent| c.guest_access == GuestAccess::CanJoin)
				.map_err(|_| Error::bad_database("Invalid room guest access event in database."))
		})
}

/// Checks if guests are able to view room content without joining
fn world_readable(room_id: &RoomId) -> Result<bool, Error> {
	Ok(services()
		.rooms
		.state_accessor
		.room_state_get(room_id, &StateEventType::RoomHistoryVisibility, "")?
		.map_or(Ok(false), |s| {
			serde_json::from_str(s.content.get())
				.map(|c: RoomHistoryVisibilityEventContent| c.history_visibility == HistoryVisibility::WorldReadable)
				.map_err(|e| {
					error!(
						"Invalid room history visibility event in database for room {room_id}, assuming is \
						 \"shared\": {e} "
					);
					Error::bad_database("Invalid room history visibility event in database.")
				})
		})
		.unwrap_or(false))
}

/// Returns the join rule for a given room
fn get_join_rule(current_room: &RoomId) -> Result<(SpaceRoomJoinRule, Vec<OwnedRoomId>), Error> {
	Ok(services()
		.rooms
		.state_accessor
		.room_state_get(current_room, &StateEventType::RoomJoinRules, "")?
		.map(|s| {
			serde_json::from_str(s.content.get())
				.map(|c: RoomJoinRulesEventContent| (c.join_rule.clone().into(), allowed_room_ids(c.join_rule)))
				.map_err(|e| {
					error!("Invalid room join rule event in database: {}", e);
					Error::BadDatabase("Invalid room join rule event in database.")
				})
		})
		.transpose()?
		.unwrap_or((SpaceRoomJoinRule::Invite, vec![])))
}

// Here because cannot implement `From` across ruma-federation-api and
// ruma-client-api types
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

/// Returns an empty vec if not a restricted room
fn allowed_room_ids(join_rule: JoinRule) -> Vec<OwnedRoomId> {
	let mut room_ids = vec![];
	if let JoinRule::Restricted(r) | JoinRule::KnockRestricted(r) = join_rule {
		for rule in r.allow {
			if let AllowRule::RoomMembership(RoomMembership {
				room_id: membership,
			}) = rule
			{
				room_ids.push(membership.clone());
			}
		}
	}
	room_ids
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

#[cfg(test)]
mod tests {
	use ruma::{
		api::federation::space::SpaceHierarchyParentSummaryInit, events::room::join_rules::Restricted, owned_room_id,
		owned_server_name,
	};

	use super::*;

	fn first(arena: &mut Arena, room_id: &OwnedRoomId) {
		let first_untrav = arena.first_untraversed().unwrap();

		assert_eq!(arena.get(first_untrav).unwrap().room_id, *room_id);
	}

	#[test]
	fn zero_depth() {
		let mut arena = Arena::new(owned_room_id!("!foo:example.org"), 0);

		assert_eq!(arena.first_untraversed(), None);
	}

	#[test]
	fn two_depth() {
		let mut arena = Arena::new(owned_room_id!("!root:example.org"), 2);

		let root = arena.first_untraversed().unwrap();
		arena.push(
			root,
			vec![
				(owned_room_id!("!subspace1:example.org"), vec![]),
				(owned_room_id!("!subspace2:example.org"), vec![]),
				(owned_room_id!("!foo:example.org"), vec![]),
			],
		);

		let subspace1 = arena.first_untraversed().unwrap();
		let subspace2 = arena.first_untraversed().unwrap();

		arena.push(
			subspace1,
			vec![
				(owned_room_id!("!room1:example.org"), vec![]),
				(owned_room_id!("!room2:example.org"), vec![]),
			],
		);

		first(&mut arena, &owned_room_id!("!room1:example.org"));
		first(&mut arena, &owned_room_id!("!room2:example.org"));

		arena.push(
			subspace2,
			vec![
				(owned_room_id!("!room3:example.org"), vec![]),
				(owned_room_id!("!room4:example.org"), vec![]),
			],
		);
		first(&mut arena, &owned_room_id!("!room3:example.org"));
		first(&mut arena, &owned_room_id!("!room4:example.org"));

		let foo_node = NodeId {
			index: 1,
		};

		assert_eq!(arena.first_untraversed(), Some(foo_node));
		assert_eq!(
			arena.get(foo_node).map(|node| node.room_id.clone()),
			Some(owned_room_id!("!foo:example.org"))
		);
	}

	#[test]
	fn empty_push() {
		let mut arena = Arena::new(owned_room_id!("!root:example.org"), 5);

		let root = arena.first_untraversed().unwrap();
		arena.push(
			root,
			vec![
				(owned_room_id!("!room1:example.org"), vec![]),
				(owned_room_id!("!room2:example.org"), vec![]),
			],
		);

		let room1 = arena.first_untraversed().unwrap();
		arena.push(room1, vec![]);

		first(&mut arena, &owned_room_id!("!room2:example.org"));
		assert!(arena.first_untraversed().is_none());
	}

	#[test]
	fn beyond_max_depth() {
		let mut arena = Arena::new(owned_room_id!("!root:example.org"), 0);

		let root = NodeId {
			index: 0,
		};

		arena.push(root, vec![(owned_room_id!("!too_deep:example.org"), vec![])]);

		assert_eq!(arena.first_child(root), None);
		assert_eq!(arena.nodes.len(), 1);
	}

	#[test]
	fn order_check() {
		let mut arena = Arena::new(owned_room_id!("!root:example.org"), 3);

		let root = arena.first_untraversed().unwrap();
		arena.push(
			root,
			vec![
				(owned_room_id!("!subspace1:example.org"), vec![]),
				(owned_room_id!("!subspace2:example.org"), vec![]),
				(owned_room_id!("!foo:example.org"), vec![]),
			],
		);

		let subspace1 = arena.first_untraversed().unwrap();
		arena.push(
			subspace1,
			vec![
				(owned_room_id!("!room1:example.org"), vec![]),
				(owned_room_id!("!room3:example.org"), vec![]),
				(owned_room_id!("!room5:example.org"), vec![]),
			],
		);

		first(&mut arena, &owned_room_id!("!room1:example.org"));
		first(&mut arena, &owned_room_id!("!room3:example.org"));
		first(&mut arena, &owned_room_id!("!room5:example.org"));

		let subspace2 = arena.first_untraversed().unwrap();

		assert_eq!(arena.get(subspace2).unwrap().room_id, owned_room_id!("!subspace2:example.org"));

		arena.push(
			subspace2,
			vec![
				(owned_room_id!("!room1:example.org"), vec![]),
				(owned_room_id!("!room2:example.org"), vec![]),
			],
		);

		first(&mut arena, &owned_room_id!("!room1:example.org"));
		first(&mut arena, &owned_room_id!("!room2:example.org"));
		first(&mut arena, &owned_room_id!("!foo:example.org"));

		assert_eq!(arena.first_untraversed(), None);
	}

	#[test]
	fn get_summary_children() {
		let summary: SpaceHierarchyParentSummary = SpaceHierarchyParentSummaryInit {
			num_joined_members: UInt::from(1_u32),
			room_id: owned_room_id!("!root:example.org"),
			world_readable: true,
			guest_can_join: true,
			join_rule: SpaceRoomJoinRule::Public,
			children_state: vec![
				serde_json::from_str(
					r#"{
                      "content": {
                        "via": [
                          "example.org"
                        ],
                        "suggested": false
                      },
                      "origin_server_ts": 1629413349153,
                      "sender": "@alice:example.org",
                      "state_key": "!foo:example.org",
                      "type": "m.space.child"
                    }"#,
				)
				.unwrap(),
				serde_json::from_str(
					r#"{
                      "content": {
                        "via": [
                          "example.org"
                        ],
                        "suggested": true
                      },
                      "origin_server_ts": 1629413349157,
                      "sender": "@alice:example.org",
                      "state_key": "!bar:example.org",
                      "type": "m.space.child"
                    }"#,
				)
				.unwrap(),
				serde_json::from_str(
					r#"{
                      "content": {
                        "via": [
                          "example.org"
                        ]
                      },
                      "origin_server_ts": 1629413349160,
                      "sender": "@alice:example.org",
                      "state_key": "!baz:example.org",
                      "type": "m.space.child"
                    }"#,
				)
				.unwrap(),
			],
			allowed_room_ids: vec![],
		}
		.into();

		assert_eq!(
			get_parent_children_via(&summary, false),
			vec![
				(owned_room_id!("!foo:example.org"), vec![owned_server_name!("example.org")]),
				(owned_room_id!("!bar:example.org"), vec![owned_server_name!("example.org")]),
				(owned_room_id!("!baz:example.org"), vec![owned_server_name!("example.org")])
			]
		);
		assert_eq!(
			get_parent_children_via(&summary, true),
			vec![(owned_room_id!("!bar:example.org"), vec![owned_server_name!("example.org")])]
		);
	}

	#[test]
	fn allowed_room_ids_from_join_rule() {
		let restricted_join_rule = JoinRule::Restricted(Restricted {
			allow: vec![
				AllowRule::RoomMembership(RoomMembership {
					room_id: owned_room_id!("!foo:example.org"),
				}),
				AllowRule::RoomMembership(RoomMembership {
					room_id: owned_room_id!("!bar:example.org"),
				}),
				AllowRule::RoomMembership(RoomMembership {
					room_id: owned_room_id!("!baz:example.org"),
				}),
			],
		});

		let invite_join_rule = JoinRule::Invite;

		assert_eq!(
			allowed_room_ids(restricted_join_rule),
			vec![
				owned_room_id!("!foo:example.org"),
				owned_room_id!("!bar:example.org"),
				owned_room_id!("!baz:example.org")
			]
		);

		let empty_vec: Vec<OwnedRoomId> = vec![];

		assert_eq!(allowed_room_ids(invite_join_rule), empty_vec);
	}

	#[test]
	fn invalid_pagnation_tokens() {
		fn token_is_err(token: &str) {
			let token: Result<PagnationToken> = PagnationToken::from_str(token);
			token.unwrap_err();
		}

		token_is_err("231_2_noabool");
		token_is_err("");
		token_is_err("111_3_");
		token_is_err("foo_not_int");
		token_is_err("11_4_true_");
		token_is_err("___");
		token_is_err("__false");
	}

	#[test]
	fn valid_pagnation_tokens() {
		assert_eq!(
			PagnationToken {
				skip: UInt::from(40_u32),
				limit: UInt::from(20_u32),
				max_depth: UInt::from(1_u32),
				suggested_only: true
			},
			PagnationToken::from_str("40_20_1_true").unwrap()
		);

		assert_eq!(
			PagnationToken {
				skip: UInt::from(27645_u32),
				limit: UInt::from(97_u32),
				max_depth: UInt::from(10539_u32),
				suggested_only: false
			},
			PagnationToken::from_str("27645_97_10539_false").unwrap()
		);
	}

	#[test]
	fn pagnation_token_to_string() {
		assert_eq!(
			PagnationToken {
				skip: UInt::from(27645_u32),
				limit: UInt::from(97_u32),
				max_depth: UInt::from(9420_u32),
				suggested_only: false
			}
			.to_string(),
			"27645_97_9420_false"
		);

		assert_eq!(
			PagnationToken {
				skip: UInt::from(12_u32),
				limit: UInt::from(3_u32),
				max_depth: UInt::from(1_u32),
				suggested_only: true
			}
			.to_string(),
			"12_3_1_true"
		);
	}

	#[test]
	fn forbid_recursion() {
		let mut arena = Arena::new(owned_room_id!("!root:example.org"), 5);
		let root_node_id = arena.first_untraversed().unwrap();

		arena.push(
			root_node_id,
			vec![
				(owned_room_id!("!subspace1:example.org"), vec![]),
				(owned_room_id!("!room1:example.org"), vec![]),
				(owned_room_id!("!subspace2:example.org"), vec![]),
			],
		);

		let subspace1_node_id = arena.first_untraversed().unwrap();
		arena.push(
			subspace1_node_id,
			vec![
				(owned_room_id!("!subspace2:example.org"), vec![]),
				(owned_room_id!("!room1:example.org"), vec![]),
			],
		);

		let subspace2_node_id = arena.first_untraversed().unwrap();
		// Here, both subspaces should be ignored and not added, as they are both
		// parents of subspace2
		arena.push(
			subspace2_node_id,
			vec![
				(owned_room_id!("!subspace1:example.org"), vec![]),
				(owned_room_id!("!subspace2:example.org"), vec![]),
				(owned_room_id!("!room1:example.org"), vec![]),
			],
		);

		assert_eq!(arena.nodes.len(), 7);
		first(&mut arena, &owned_room_id!("!room1:example.org"));
		first(&mut arena, &owned_room_id!("!room1:example.org"));
		first(&mut arena, &owned_room_id!("!room1:example.org"));
		first(&mut arena, &owned_room_id!("!subspace2:example.org"));
		assert!(arena.first_untraversed().is_none());
	}
}
