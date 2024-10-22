#![cfg(test)]
#![allow(clippy::needless_borrows_for_generic_args)]

use std::fmt::Debug;

use arrayvec::ArrayVec;
use conduit::ruma::{serde::Raw, RoomId, UserId};
use serde::Serialize;

use crate::{
	de, ser,
	ser::{serialize_to_vec, Json},
	Ignore, Interfix,
};

#[test]
#[should_panic(expected = "serializing string at the top-level")]
fn ser_str() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let s = serialize_to_vec(&user_id).expect("failed to serialize user_id");
	assert_eq!(&s, user_id.as_bytes());
}

#[test]
fn ser_tuple() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let room_id: &RoomId = "!room:example.com".try_into().unwrap();

	let mut a = user_id.as_bytes().to_vec();
	a.push(0xFF);
	a.extend_from_slice(room_id.as_bytes());

	let b = (user_id, room_id);
	let b = serialize_to_vec(&b).expect("failed to serialize tuple");

	assert_eq!(a, b);
}

#[test]
#[should_panic(expected = "I/O error: failed to write whole buffer")]
fn ser_overflow() {
	const BUFSIZE: usize = 10;

	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let room_id: &RoomId = "!room:example.com".try_into().unwrap();

	assert!(BUFSIZE < user_id.as_str().len() + room_id.as_str().len());
	let mut buf = ArrayVec::<u8, BUFSIZE>::new();

	let val = (user_id, room_id);
	_ = ser::serialize(&mut buf, val).unwrap();
}

#[test]
fn ser_complex() {
	use conduit::ruma::Mxc;

	#[derive(Debug, Serialize)]
	struct Dim {
		width: u32,
		height: u32,
	}

	let mxc = Mxc {
		server_name: "example.com".try_into().unwrap(),
		media_id: "AbCdEfGhIjK",
	};

	let dim = Dim {
		width: 123,
		height: 456,
	};

	let mut a = Vec::new();
	a.extend_from_slice(b"mxc://");
	a.extend_from_slice(mxc.server_name.as_bytes());
	a.extend_from_slice(b"/");
	a.extend_from_slice(mxc.media_id.as_bytes());
	a.push(0xFF);
	a.extend_from_slice(&dim.width.to_be_bytes());
	a.extend_from_slice(&dim.height.to_be_bytes());
	a.push(0xFF);

	let d: &[u32] = &[dim.width, dim.height];
	let b = (mxc, d, Interfix);
	let b = serialize_to_vec(b).expect("failed to serialize complex");

	assert_eq!(a, b);
}

#[test]
fn ser_json() {
	use conduit::ruma::api::client::filter::FilterDefinition;

	let filter = FilterDefinition {
		event_fields: Some(vec!["content.body".to_owned()]),
		..Default::default()
	};

	let serialized = serialize_to_vec(Json(&filter)).expect("failed to serialize value");

	let s = String::from_utf8_lossy(&serialized);
	assert_eq!(&s, r#"{"event_fields":["content.body"]}"#);
}

#[test]
fn ser_json_value() {
	use conduit::ruma::api::client::filter::FilterDefinition;

	let filter = FilterDefinition {
		event_fields: Some(vec!["content.body".to_owned()]),
		..Default::default()
	};

	let value = serde_json::to_value(filter).expect("failed to serialize to serde_json::value");
	let serialized = serialize_to_vec(Json(value)).expect("failed to serialize value");

	let s = String::from_utf8_lossy(&serialized);
	assert_eq!(&s, r#"{"event_fields":["content.body"]}"#);
}

#[test]
fn ser_json_macro() {
	use serde_json::json;

	#[derive(Serialize)]
	struct Foo {
		foo: String,
	}

	let content = Foo {
		foo: "bar".to_owned(),
	};
	let content = serde_json::to_value(content).expect("failed to serialize content");
	let sender: &UserId = "@foo:example.com".try_into().unwrap();
	let serialized = serialize_to_vec(Json(json!({
		"sender": sender,
		"content": content,
	})))
	.expect("failed to serialize value");

	let s = String::from_utf8_lossy(&serialized);
	assert_eq!(&s, r#"{"content":{"foo":"bar"},"sender":"@foo:example.com"}"#);
}

#[test]
#[should_panic(expected = "serializing string at the top-level")]
fn ser_json_raw() {
	use conduit::ruma::api::client::filter::FilterDefinition;

	let filter = FilterDefinition {
		event_fields: Some(vec!["content.body".to_owned()]),
		..Default::default()
	};

	let value = serde_json::value::to_raw_value(&filter).expect("failed to serialize to raw value");
	let a = serialize_to_vec(value.get()).expect("failed to serialize raw value");
	let s = String::from_utf8_lossy(&a);
	assert_eq!(&s, r#"{"event_fields":["content.body"]}"#);
}

#[test]
#[should_panic(expected = "you can skip serialization instead")]
fn ser_json_raw_json() {
	use conduit::ruma::api::client::filter::FilterDefinition;

	let filter = FilterDefinition {
		event_fields: Some(vec!["content.body".to_owned()]),
		..Default::default()
	};

	let value = serde_json::value::to_raw_value(&filter).expect("failed to serialize to raw value");
	let a = serialize_to_vec(Json(value)).expect("failed to serialize json value");
	let s = String::from_utf8_lossy(&a);
	assert_eq!(&s, r#"{"event_fields":["content.body"]}"#);
}

#[test]
fn de_tuple() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let room_id: &RoomId = "!room:example.com".try_into().unwrap();

	let raw: &[u8] = b"@user:example.com\xFF!room:example.com";
	let (a, b): (&UserId, &RoomId) = de::from_slice(raw).expect("failed to deserialize");

	assert_eq!(a, user_id, "deserialized user_id does not match");
	assert_eq!(b, room_id, "deserialized room_id does not match");
}

#[test]
#[should_panic(expected = "failed to deserialize")]
fn de_tuple_invalid() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let room_id: &RoomId = "!room:example.com".try_into().unwrap();

	let raw: &[u8] = b"@user:example.com\xFF@user:example.com";
	let (a, b): (&UserId, &RoomId) = de::from_slice(raw).expect("failed to deserialize");

	assert_eq!(a, user_id, "deserialized user_id does not match");
	assert_eq!(b, room_id, "deserialized room_id does not match");
}

#[test]
#[should_panic(expected = "failed to deserialize")]
fn de_tuple_incomplete() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();

	let raw: &[u8] = b"@user:example.com";
	let (a, _): (&UserId, &RoomId) = de::from_slice(raw).expect("failed to deserialize");

	assert_eq!(a, user_id, "deserialized user_id does not match");
}

#[test]
#[should_panic(expected = "failed to deserialize")]
fn de_tuple_incomplete_with_sep() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();

	let raw: &[u8] = b"@user:example.com\xFF";
	let (a, _): (&UserId, &RoomId) = de::from_slice(raw).expect("failed to deserialize");

	assert_eq!(a, user_id, "deserialized user_id does not match");
}

#[test]
#[should_panic(expected = "deserialization failed to consume trailing bytes")]
fn de_tuple_unfinished() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let room_id: &RoomId = "!room:example.com".try_into().unwrap();

	let raw: &[u8] = b"@user:example.com\xFF!room:example.com\xFF@user:example.com";
	let (a, b): (&UserId, &RoomId) = de::from_slice(raw).expect("failed to deserialize");

	assert_eq!(a, user_id, "deserialized user_id does not match");
	assert_eq!(b, room_id, "deserialized room_id does not match");
}

#[test]
fn de_tuple_ignore() {
	let user_id: &UserId = "@user:example.com".try_into().unwrap();
	let room_id: &RoomId = "!room:example.com".try_into().unwrap();

	let raw: &[u8] = b"@user:example.com\xFF@user2:example.net\xFF!room:example.com";
	let (a, _, c): (&UserId, Ignore, &RoomId) = de::from_slice(raw).expect("failed to deserialize");

	assert_eq!(a, user_id, "deserialized user_id does not match");
	assert_eq!(c, room_id, "deserialized room_id does not match");
}

#[test]
fn de_json_array() {
	let a = &["foo", "bar", "baz"];
	let s = serde_json::to_vec(a).expect("failed to serialize to JSON array");

	let b: Raw<Vec<Raw<String>>> = de::from_slice(&s).expect("failed to deserialize");

	let d: Vec<String> = serde_json::from_str(b.json().get()).expect("failed to deserialize JSON");

	for (i, a) in a.iter().enumerate() {
		assert_eq!(*a, d[i]);
	}
}

#[test]
fn de_json_raw_array() {
	let a = &["foo", "bar", "baz"];
	let s = serde_json::to_vec(a).expect("failed to serialize to JSON array");

	let b: Raw<Vec<Raw<String>>> = de::from_slice(&s).expect("failed to deserialize");

	let c: Vec<Raw<String>> = serde_json::from_str(b.json().get()).expect("failed to deserialize JSON");

	for (i, a) in a.iter().enumerate() {
		let c = serde_json::to_value(c[i].json()).expect("failed to deserialize JSON to string");
		assert_eq!(*a, c);
	}
}

#[test]
fn ser_array() {
	let a: u64 = 123_456;
	let b: u64 = 987_654;

	let arr: &[u64] = &[a, b];

	let mut v = Vec::new();
	v.extend_from_slice(&a.to_be_bytes());
	v.extend_from_slice(&b.to_be_bytes());

	let s = serialize_to_vec(arr).expect("failed to serialize");
	assert_eq!(&s, &v, "serialization does not match");
}
