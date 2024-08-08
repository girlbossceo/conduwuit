use std::io::Write;

use conduit::{err, result::DebugInspect, utils::exchange, Error, Result};
use serde::{ser, Serialize};

#[inline]
pub(crate) fn serialize_to_vec<T>(val: &T) -> Result<Vec<u8>>
where
	T: Serialize + ?Sized,
{
	let mut buf = Vec::with_capacity(64);
	serialize(&mut buf, val)?;

	Ok(buf)
}

#[inline]
pub(crate) fn serialize<'a, W, T>(out: &'a mut W, val: &'a T) -> Result<&'a [u8]>
where
	W: Write + AsRef<[u8]>,
	T: Serialize + ?Sized,
{
	let mut serializer = Serializer {
		out,
		depth: 0,
		sep: false,
		fin: false,
	};

	val.serialize(&mut serializer)
		.map_err(|error| err!(SerdeSer("{error}")))
		.debug_inspect(|()| {
			debug_assert_eq!(serializer.depth, 0, "Serialization completed at non-zero recursion level");
		})?;

	Ok((*out).as_ref())
}

pub(crate) struct Serializer<'a, W: Write> {
	out: &'a mut W,
	depth: u32,
	sep: bool,
	fin: bool,
}

/// Directive to force separator serialization specifically for prefix keying
/// use. This is a quirk of the database schema and prefix iterations.
#[derive(Debug, Serialize)]
pub struct Interfix;

/// Directive to force separator serialization. Separators are usually
/// serialized automatically.
#[derive(Debug, Serialize)]
pub struct Separator;

impl<W: Write> Serializer<'_, W> {
	const SEP: &'static [u8] = b"\xFF";

	fn sequence_start(&mut self) {
		debug_assert!(!self.is_finalized(), "Sequence start with finalization set");
		debug_assert!(!self.sep, "Sequence start with separator set");
		if cfg!(debug_assertions) {
			self.depth = self.depth.saturating_add(1);
		}
	}

	fn sequence_end(&mut self) {
		self.sep = false;
		if cfg!(debug_assertions) {
			self.depth = self.depth.saturating_sub(1);
		}
	}

	fn record_start(&mut self) -> Result<()> {
		debug_assert!(!self.is_finalized(), "Starting a record after serialization finalized");
		exchange(&mut self.sep, true)
			.then(|| self.separator())
			.unwrap_or(Ok(()))
	}

	fn separator(&mut self) -> Result<()> {
		debug_assert!(!self.is_finalized(), "Writing a separator after serialization finalized");
		self.out.write_all(Self::SEP).map_err(Into::into)
	}

	fn set_finalized(&mut self) {
		debug_assert!(!self.is_finalized(), "Finalization already set");
		if cfg!(debug_assertions) {
			self.fin = true;
		}
	}

	fn is_finalized(&self) -> bool { self.fin }
}

impl<W: Write> ser::Serializer for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();
	type SerializeMap = Self;
	type SerializeSeq = Self;
	type SerializeStruct = Self;
	type SerializeStructVariant = Self;
	type SerializeTuple = Self;
	type SerializeTupleStruct = Self;
	type SerializeTupleVariant = Self;

	fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
		unimplemented!("serialize Map not implemented")
	}

	fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
		self.sequence_start();
		self.record_start()?;
		Ok(self)
	}

	fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple> {
		self.sequence_start();
		Ok(self)
	}

	fn serialize_tuple_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeTupleStruct> {
		self.sequence_start();
		Ok(self)
	}

	fn serialize_tuple_variant(
		self, _name: &'static str, _idx: u32, _var: &'static str, _len: usize,
	) -> Result<Self::SerializeTupleVariant> {
		self.sequence_start();
		Ok(self)
	}

	fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeStruct> {
		self.sequence_start();
		Ok(self)
	}

	fn serialize_struct_variant(
		self, _name: &'static str, _idx: u32, _var: &'static str, _len: usize,
	) -> Result<Self::SerializeStructVariant> {
		self.sequence_start();
		Ok(self)
	}

	fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _name: &'static str, _value: &T) -> Result<Self::Ok> {
		unimplemented!("serialize New Type Struct not implemented")
	}

	fn serialize_newtype_variant<T: Serialize + ?Sized>(
		self, _name: &'static str, _idx: u32, _var: &'static str, _value: &T,
	) -> Result<Self::Ok> {
		unimplemented!("serialize New Type Variant not implemented")
	}

	fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok> {
		match name {
			"Interfix" => {
				self.set_finalized();
			},
			"Separator" => {
				self.separator()?;
			},
			_ => unimplemented!("Unrecognized serialization directive: {name:?}"),
		};

		Ok(())
	}

	fn serialize_unit_variant(self, _name: &'static str, _idx: u32, _var: &'static str) -> Result<Self::Ok> {
		unimplemented!("serialize Unit Variant not implemented")
	}

	fn serialize_some<T: Serialize + ?Sized>(self, val: &T) -> Result<Self::Ok> { val.serialize(self) }

	fn serialize_none(self) -> Result<Self::Ok> { Ok(()) }

	fn serialize_char(self, v: char) -> Result<Self::Ok> {
		let mut buf: [u8; 4] = [0; 4];
		self.serialize_str(v.encode_utf8(&mut buf))
	}

	fn serialize_str(self, v: &str) -> Result<Self::Ok> { self.serialize_bytes(v.as_bytes()) }

	fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok> { self.out.write_all(v).map_err(Error::Io) }

	fn serialize_f64(self, _v: f64) -> Result<Self::Ok> { unimplemented!("serialize f64 not implemented") }

	fn serialize_f32(self, _v: f32) -> Result<Self::Ok> { unimplemented!("serialize f32 not implemented") }

	fn serialize_i64(self, v: i64) -> Result<Self::Ok> { self.out.write_all(&v.to_be_bytes()).map_err(Error::Io) }

	fn serialize_i32(self, _v: i32) -> Result<Self::Ok> { unimplemented!("serialize i32 not implemented") }

	fn serialize_i16(self, _v: i16) -> Result<Self::Ok> { unimplemented!("serialize i16 not implemented") }

	fn serialize_i8(self, _v: i8) -> Result<Self::Ok> { unimplemented!("serialize i8 not implemented") }

	fn serialize_u64(self, v: u64) -> Result<Self::Ok> { self.out.write_all(&v.to_be_bytes()).map_err(Error::Io) }

	fn serialize_u32(self, _v: u32) -> Result<Self::Ok> { unimplemented!("serialize u32 not implemented") }

	fn serialize_u16(self, _v: u16) -> Result<Self::Ok> { unimplemented!("serialize u16 not implemented") }

	fn serialize_u8(self, v: u8) -> Result<Self::Ok> { self.out.write_all(&[v]).map_err(Error::Io) }

	fn serialize_bool(self, _v: bool) -> Result<Self::Ok> { unimplemented!("serialize bool not implemented") }

	fn serialize_unit(self) -> Result<Self::Ok> { unimplemented!("serialize unit not implemented") }
}

impl<W: Write> ser::SerializeMap for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_key<T: Serialize + ?Sized>(&mut self, _key: &T) -> Result<Self::Ok> {
		unimplemented!("serialize Map Key not implemented")
	}

	fn serialize_value<T: Serialize + ?Sized>(&mut self, _val: &T) -> Result<Self::Ok> {
		unimplemented!("serialize Map Val not implemented")
	}

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}

impl<W: Write> ser::SerializeSeq for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_element<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> { val.serialize(&mut **self) }

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}

impl<W: Write> ser::SerializeStruct for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_field<T: Serialize + ?Sized>(&mut self, _key: &'static str, val: &T) -> Result<Self::Ok> {
		self.record_start()?;
		val.serialize(&mut **self)
	}

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}

impl<W: Write> ser::SerializeStructVariant for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_field<T: Serialize + ?Sized>(&mut self, _key: &'static str, val: &T) -> Result<Self::Ok> {
		self.record_start()?;
		val.serialize(&mut **self)
	}

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}

impl<W: Write> ser::SerializeTuple for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_element<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
		self.record_start()?;
		val.serialize(&mut **self)
	}

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}

impl<W: Write> ser::SerializeTupleStruct for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_field<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
		self.record_start()?;
		val.serialize(&mut **self)
	}

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}

impl<W: Write> ser::SerializeTupleVariant for &mut Serializer<'_, W> {
	type Error = Error;
	type Ok = ();

	fn serialize_field<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
		self.record_start()?;
		val.serialize(&mut **self)
	}

	fn end(self) -> Result<Self::Ok> {
		self.sequence_end();
		Ok(())
	}
}
