//! Helper tools for implementing filtering in the `/client/v3/sync` and
//! `/client/v3/rooms/:roomId/messages` endpoints.
//!
//! The default strategy for filtering is to generate all events, check them
//! against the filter, and drop events that were rejected. When significant
//! fraction of events are rejected, this results in a large amount of wasted
//! work computing events that will be dropped. In most cases, the structure of
//! our database doesn't allow for anything fancier, with only a few exceptions.
//!
//! The first exception is room filters (`room`/`not_room` pairs in
//! `filter.rooms` and `filter.rooms.{account_data,timeline,ephemeral,state}`).
//! In `/messages`, if the room is rejected by the filter, we can skip the
//! entire request. The outer loop of our `/sync` implementation is over rooms,
//! and so we are able to skip work for an entire room if it is rejected by the
//! top-level `filter.rooms.room`. Similarly, when a room is rejected for all
//! events in a particular category, we can skip work generating events in that
//! category for the rejected room.

use std::{collections::HashSet, hash::Hash};

use regex::RegexSet;
use ruma::{
	api::client::filter::{FilterDefinition, RoomEventFilter, RoomFilter, UrlFilter},
	RoomId, UserId,
};

use crate::{Error, PduEvent};

/// Structure for testing against an allowlist and a denylist with a single
/// `HashSet` lookup.
///
/// The denylist takes precedence (an item included in both the allowlist and
/// the denylist is denied).
pub(crate) enum AllowDenyList<'a, T: ?Sized> {
	/// TODO: fast-paths for allow-all and deny-all?
	Allow(HashSet<&'a T>),
	Deny(HashSet<&'a T>),
}

impl<'a, T: ?Sized + Hash + PartialEq + Eq> AllowDenyList<'a, T> {
	fn new<A, D>(allow: Option<A>, deny: D) -> AllowDenyList<'a, T>
	where
		A: Iterator<Item = &'a T>,
		D: Iterator<Item = &'a T>,
	{
		let deny_set = deny.collect::<HashSet<_>>();
		if let Some(allow) = allow {
			AllowDenyList::Allow(allow.filter(|x| !deny_set.contains(x)).collect())
		} else {
			AllowDenyList::Deny(deny_set)
		}
	}

	fn from_slices<O: AsRef<T>>(allow: Option<&'a [O]>, deny: &'a [O]) -> AllowDenyList<'a, T> {
		AllowDenyList::new(
			allow.map(|allow| allow.iter().map(AsRef::as_ref)),
			deny.iter().map(AsRef::as_ref),
		)
	}

	pub(crate) fn allowed(&self, value: &T) -> bool {
		match self {
			AllowDenyList::Allow(allow) => allow.contains(value),
			AllowDenyList::Deny(deny) => !deny.contains(value),
		}
	}
}

struct WildcardAllowDenyList {
	allow: Option<RegexSet>,
	deny: Option<RegexSet>,
}

/// Converts a wildcard pattern (like in filter.room.timeline.types) to a regex.
///
/// Wildcard patterns are all literal strings except for the `'*'` character,
/// which matches any sequence of characters.
fn wildcard_to_regex(pattern: &str) -> String {
	let mut regex_pattern = String::new();
	regex_pattern.push('^');
	let mut parts = pattern.split('*').peekable();
	while let Some(part) = parts.next() {
		regex_pattern.push_str(&regex::escape(part));
		if parts.peek().is_some() {
			regex_pattern.push_str(".*");
		}
	}
	regex_pattern.push('$');
	regex_pattern
}

impl WildcardAllowDenyList {
	fn new<S: AsRef<str>>(allow: Option<&[S]>, deny: &[S]) -> Result<WildcardAllowDenyList, regex::Error> {
		Ok(WildcardAllowDenyList {
			allow: allow
				.map(|allow| {
					RegexSet::new(
						allow
							.iter()
							.map(|pattern| wildcard_to_regex(pattern.as_ref())),
					)
				})
				.transpose()?,
			deny: if deny.is_empty() {
				None
			} else {
				Some(RegexSet::new(
					deny.iter()
						.map(|pattern| wildcard_to_regex(pattern.as_ref())),
				)?)
			},
		})
	}

	fn allowed(&self, value: &str) -> bool {
		self.allow
			.as_ref()
			.map_or(true, |allow| allow.is_match(value))
			&& self
				.deny
				.as_ref()
				.map_or(true, |deny| !deny.is_match(value))
	}
}

/// Wrapper for a [`ruma::api::client::filter::FilterDefinition`], preprocessed
/// to allow checking against the filter efficiently.
///
/// The preprocessing consists of merging the `X` and `not_X` pairs into
/// combined structures. For most fields, this is a [`AllowDenyList`]. For
/// `types`/`not_types`, this is a [`WildcardAllowDenyList`], because the type
/// filter fields support `'*'` wildcards.
pub(crate) struct CompiledFilterDefinition<'a> {
	pub(crate) room: CompiledRoomFilter<'a>,
}

pub(crate) struct CompiledRoomFilter<'a> {
	rooms: AllowDenyList<'a, RoomId>,
	pub(crate) timeline: CompiledRoomEventFilter<'a>,
	pub(crate) state: CompiledRoomEventFilter<'a>,
}

pub(crate) struct CompiledRoomEventFilter<'a> {
	// TODO: consider falling back a more-efficient AllowDenyList<TimelineEventType> when none of the type patterns
	// include a wildcard.
	types: WildcardAllowDenyList,
	rooms: AllowDenyList<'a, RoomId>,
	senders: AllowDenyList<'a, UserId>,
	url_filter: Option<UrlFilter>,
}

impl<'a> TryFrom<&'a FilterDefinition> for CompiledFilterDefinition<'a> {
	type Error = Error;

	fn try_from(source: &'a FilterDefinition) -> Result<CompiledFilterDefinition<'a>, Error> {
		Ok(CompiledFilterDefinition {
			room: (&source.room).try_into()?,
		})
	}
}

impl<'a> TryFrom<&'a RoomFilter> for CompiledRoomFilter<'a> {
	type Error = Error;

	fn try_from(source: &'a RoomFilter) -> Result<CompiledRoomFilter<'a>, Error> {
		Ok(CompiledRoomFilter {
			// TODO: consider calculating the intersection of room filters in
			// all of the sub-filters
			rooms: AllowDenyList::from_slices(source.rooms.as_deref(), &source.not_rooms),
			timeline: (&source.timeline).try_into()?,
			state: (&source.state).try_into()?,
		})
	}
}

impl<'a> TryFrom<&'a RoomEventFilter> for CompiledRoomEventFilter<'a> {
	type Error = Error;

	fn try_from(source: &'a RoomEventFilter) -> Result<CompiledRoomEventFilter<'a>, Error> {
		Ok(CompiledRoomEventFilter {
			types: WildcardAllowDenyList::new(source.types.as_deref(), &source.not_types)?,
			rooms: AllowDenyList::from_slices(source.rooms.as_deref(), &source.not_rooms),
			senders: AllowDenyList::from_slices(source.senders.as_deref(), &source.not_senders),
			url_filter: source.url_filter,
		})
	}
}

impl CompiledRoomFilter<'_> {
	/// Returns the top-level [`AllowDenyList`] for rooms (`rooms`/`not_rooms`
	/// in `filter.room`).
	///
	/// This is useful because, with an allowlist, iterating over allowed rooms
	/// and checking whether they are visible to a user can be faster than
	/// iterating over visible rooms and checking whether they are allowed.
	pub(crate) fn rooms(&self) -> &AllowDenyList<'_, RoomId> { &self.rooms }
}

impl CompiledRoomEventFilter<'_> {
	/// Returns `true` if a room is allowed by the `rooms` and `not_rooms`
	/// fields.
	///
	/// This does *not* test the room against the top-level `rooms` filter.
	/// It is expected that callers have already filtered rooms that are
	/// rejected by the top-level filter using
	/// [`CompiledRoomFilter::room_allowed`], if applicable.
	pub(crate) fn room_allowed(&self, room_id: &RoomId) -> bool { self.rooms.allowed(room_id) }

	/// Returns `true` if a PDU event is allowed by the filter.
	///
	/// This tests against the `senders`, `not_senders`, `types`, `not_types`,
	/// and `url_filter` fields.
	///
	/// This does *not* check whether the event's room is allowed. It is
	/// expected that callers have already filtered out rejected rooms using
	/// [`CompiledRoomEventFilter::room_allowed`] and
	/// [`CompiledRoomFilter::room_allowed`].
	pub(crate) fn pdu_event_allowed(&self, pdu: &PduEvent) -> bool {
		self.senders.allowed(&pdu.sender)
			&& self.types.allowed(&pdu.kind.to_string())
			&& self.allowed_by_url_filter(pdu)
	}

	fn allowed_by_url_filter(&self, pdu: &PduEvent) -> bool {
		let Some(filter) = self.url_filter else {
			return true;
		};
		// TODO: is this unwrap okay?
		let content: serde_json::Value = serde_json::from_str(pdu.content.get()).unwrap();
		match filter {
			UrlFilter::EventsWithoutUrl => !content["url"].is_string(),
			UrlFilter::EventsWithUrl => content["url"].is_string(),
		}
	}
}
