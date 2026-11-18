// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cells of an AQL query over `ACCESS_LOG_ENTRY` (#5131), read out of a
//! typed record by its paths, with no JSON value of the record built (#6005).
//!
//! A record is serialised once per path through a serializer that follows
//! the path's steps: a member the step names is entered, a list a step
//! reaches is crossed, and what the last step reaches is written as JSON
//! text in the record's own member order, the order its ITS-JSON value
//! keeps. The subject view's members are left out as the view leaves them
//! out ([`crate::access_log::view::subject_removals`]). The cells are
//! the ones [`super::path::LogPath::read`] reads from the record's view, the
//! reference the tests hold them to. No openEHR spec governs AQL over the
//! access log: our own design.

use std::borrow::Cow;

use openehr_query::ast::Primitive;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use serde::ser::{
    self, Impossible, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeTuple,
    SerializeTupleStruct, Serializer,
};

use super::path::LogPath;
use crate::access_log::view::{LogReader, Restrictions, subject_removals};
use crate::aql::cell::Cell;

/// What of a record a reader is shown.
#[derive(Debug)]
pub enum Shown {
    /// The whole record, to a reviewer.
    Whole,
    /// The subject view of an entry, its accessor without the members the
    /// restrictions name for its kind.
    Subject(Vec<&'static str>),
    /// Nothing: a tombstone, which the person is never served.
    Nothing,
}

impl Shown {
    /// What `reader` is shown of `record` under `restrictions`.
    #[must_use]
    pub fn of(record: &AccessLogRecord, reader: LogReader, restrictions: &Restrictions) -> Self {
        match (record, reader) {
            (_, LogReader::Reviewer) => Self::Whole,
            (AccessLogRecord::AccessLogEntry(entry), LogReader::Person) => {
                Self::Subject(restrictions.members(&entry.accessor.kind).collect())
            }
            (AccessLogRecord::AccessLogTombstone(_), LogReader::Person) => Self::Nothing,
        }
    }
}

/// The cell `path` reads of `record`, shown as `shown`: what its last step
/// reaches, `null` where it reaches nothing, and for a path that crosses a
/// container the list of every value it reaches.
///
/// # Errors
/// A [`serde_json::Error`] when the record does not serialise.
pub fn cell(record: &AccessLogRecord, path: &LogPath, shown: &Shown) -> Result<Cell, Error> {
    let mut reach = Reach {
        steps: &path.steps,
        shown,
        at: Vec::new(),
        reached: Vec::new(),
    };
    match shown {
        Shown::Nothing => {}
        _ if path.steps.is_empty() => {
            let whole = emitted(&mut reach, record)?;
            reach.reached.push(whole);
        }
        _ => record.serialize(Seek {
            reach: &mut reach,
            depth: 0,
            crosses: false,
        })?,
    }
    let reached = reach.reached;
    Ok(Cell::of_text(if path.multiple {
        let mut out =
            String::with_capacity(reached.iter().map(|text| text.len() + 1).sum::<usize>() + 2);
        out.push('[');
        for (index, text) in reached.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(text);
        }
        out.push(']');
        out
    } else {
        reached
            .into_iter()
            .next()
            .unwrap_or_else(|| "null".to_owned())
    }))
}

/// The cell of the literal `primitive`.
#[must_use]
pub fn literal(primitive: &Primitive) -> Cell {
    match primitive {
        Primitive::String(text) => Cell::of_str(text),
        Primitive::Integer(number) => Cell::of_text(number.to_string()),
        // NOTE: a non-finite double has no JSON number, so it reads as null as its value did.
        Primitive::Real(number) if !number.is_finite() => Cell::null(),
        Primitive::Real(number) => Cell::of_text(json_of(*number)),
        Primitive::Boolean(flag) => Cell::of_text(flag.to_string()),
        Primitive::Null => Cell::null(),
    }
}

/// The JSON text of the finite double `number`, as `serde_json` writes it.
fn json_of(number: f64) -> String {
    let mut out = Vec::new();
    // NOTE: writing a finite double into a vector cannot fail; a failure reads as null.
    match serde_json::to_writer(&mut out, &number) {
        Ok(()) => utf8(out),
        Err(_) => "null".to_owned(),
    }
}

/// Why a record does not serialise into a cell.
pub type Error = serde_json::Error;

/// A path being followed through one record.
struct Reach<'p> {
    /// The path's steps.
    steps: &'p [String],
    /// What of the record is shown.
    shown: &'p Shown,
    /// The member names from the record to the value being serialised.
    at: Vec<Cow<'static, str>>,
    /// The JSON text of each value the last step reached, in order.
    reached: Vec<String>,
}

impl Reach<'_> {
    /// The members left out of the value at [`Self::at`], in the order the
    /// subject view removes them; empty for the whole record.
    fn removals(&self) -> Vec<&str> {
        match self.shown {
            Shown::Subject(restricted) => {
                let at: Vec<&str> = self.at.iter().map(AsRef::as_ref).collect();
                subject_removals(&at, restricted)
            }
            Shown::Whole | Shown::Nothing => Vec::new(),
        }
    }

    /// Whether the member `name` of the value at [`Self::at`] is left out of
    /// what is shown.
    fn withholds(&self, name: &str) -> bool {
        self.removals().contains(&name)
    }

    /// Serialises `value`, the member `name`, at `depth` steps: entered,
    /// and crossed when it is a list.
    fn member<T: Serialize + ?Sized>(
        &mut self,
        name: Cow<'static, str>,
        value: &T,
        depth: usize,
    ) -> Result<(), Error> {
        self.at.push(name);
        let result = if depth == self.steps.len() {
            let mut out = Vec::new();
            value
                .serialize(Emit {
                    reach: self,
                    out: &mut out,
                    crosses: true,
                })
                .map(|crossed| {
                    if !crossed {
                        self.reached.push(utf8(out));
                    }
                })
        } else {
            value.serialize(Seek {
                reach: self,
                depth,
                crosses: true,
            })
        };
        self.at.pop();
        result
    }
}

/// JSON text written by `serde_json`, which is UTF-8.
fn utf8(out: Vec<u8>) -> String {
    // NOTE: serde_json writes UTF-8 only, so the lossy branch is never taken.
    String::from_utf8(out)
        .unwrap_or_else(|bytes| String::from_utf8_lossy(bytes.as_bytes()).into_owned())
}

/// Why a record does not serialise into a cell: the records hold no enum
/// variant `serde` writes as an object of its name.
const NO_VARIANT: &str = "an access-log record holds no externally tagged variant";

/// The refusal of a variant written as an object of its name.
fn no_variant<T>() -> Result<T, Error> {
    Err(ser::Error::custom(NO_VARIANT))
}

/// A serializer that follows the path below `depth` steps: it enters the
/// member the next step names, and writes nothing of any other value.
struct Seek<'r, 'p> {
    /// The path followed.
    reach: &'r mut Reach<'p>,
    /// The steps taken.
    depth: usize,
    /// Whether a list here is crossed: true for a value a step just reached.
    crosses: bool,
}

impl Seek<'_, '_> {
    /// The member `name` with `value`, entered when the next step names it.
    fn step<T: Serialize + ?Sized>(
        &mut self,
        name: Cow<'static, str>,
        value: &T,
    ) -> Result<(), Error> {
        let named = self
            .reach
            .steps
            .get(self.depth)
            .is_some_and(|step| *step == name);
        if !named || self.reach.withholds(&name) {
            return Ok(());
        }
        self.reach.member(name, value, self.depth + 1)
    }
}

/// The text of a map key, as a JSON value's member name reads it.
fn key_text<T: Serialize + ?Sized>(key: &T) -> Result<String, Error> {
    key.serialize(KeyText)
}

macro_rules! seek_nothing {
    ($($method:ident($($arg:ty),*)),* $(,)?) => {
        // NOTE: a value that is no object or list holds no member a step names.
        $(fn $method(self, $(_: $arg),*) -> Result<(), Error> { Ok(()) })*
    };
}

impl<'r, 'p> Serializer for Seek<'r, 'p> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = SeekSeq<'r, 'p>;
    type SerializeTuple = SeekSeq<'r, 'p>;
    type SerializeTupleStruct = SeekSeq<'r, 'p>;
    type SerializeTupleVariant = Impossible<(), Error>;
    type SerializeMap = SeekMap<'r, 'p>;
    type SerializeStruct = Self;
    type SerializeStructVariant = Impossible<(), Error>;

    seek_nothing!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char),
        serialize_str(&str),
        serialize_bytes(&[u8]),
        serialize_none(),
        serialize_unit(),
        serialize_unit_struct(&'static str),
        serialize_unit_variant(&'static str, u32, &'static str),
    );

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), Error> {
        no_variant()
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<SeekSeq<'r, 'p>, Error> {
        Ok(SeekSeq(self.crosses.then_some(self)))
    }

    fn serialize_tuple(self, len: usize) -> Result<SeekSeq<'r, 'p>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<SeekSeq<'r, 'p>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Impossible<(), Error>, Error> {
        no_variant()
    }

    fn serialize_map(self, _: Option<usize>) -> Result<SeekMap<'r, 'p>, Error> {
        Ok(SeekMap {
            seek: self,
            key: None,
        })
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Error> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Impossible<(), Error>, Error> {
        no_variant()
    }
}

impl SerializeStruct for Seek<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.step(Cow::Borrowed(key), value)
    }

    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}

/// A list the path crosses, its elements each followed by themselves, or
/// passes by (`None`): a list in a list holds no member a step names.
struct SeekSeq<'r, 'p>(Option<Seek<'r, 'p>>);

impl SeekSeq<'_, '_> {
    /// Follows the path into the element `value`.
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let Some(seek) = self.0.as_mut() else {
            return Ok(());
        };
        value.serialize(Seek {
            reach: seek.reach,
            depth: seek.depth,
            crosses: false,
        })
    }
}

macro_rules! seek_seq {
    ($($trait:ident :: $method:ident),* $(,)?) => {
        $(impl $trait for SeekSeq<'_, '_> {
            type Ok = ();
            type Error = Error;

            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
                self.element(value)
            }

            fn end(self) -> Result<(), Error> {
                Ok(())
            }
        })*
    };
}

seek_seq!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
);

/// A map the path enters.
struct SeekMap<'r, 'p> {
    /// The path.
    seek: Seek<'r, 'p>,
    /// The key of the entry whose value comes next.
    key: Option<String>,
}

impl SerializeMap for SeekMap<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        self.key = Some(key_text(key)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let key = self.key.take().unwrap_or_default();
        self.seek.step(Cow::Owned(key), value)
    }

    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}

/// The JSON text of `value` as it is shown.
fn emitted<T: Serialize + ?Sized>(reach: &mut Reach<'_>, value: &T) -> Result<String, Error> {
    let mut out = Vec::new();
    value.serialize(Emit {
        reach,
        out: &mut out,
        crosses: false,
    })?;
    Ok(utf8(out))
}

/// A serializer that writes a value's JSON text, its members in their own
/// order, without the members the shown view leaves out.
struct Emit<'r, 'p, 'o> {
    /// The path followed, which knows what is shown.
    reach: &'r mut Reach<'p>,
    /// The text written.
    out: &'o mut Vec<u8>,
    /// Whether a list here is crossed: each element is reached by itself.
    crosses: bool,
}

/// What an [`Emit`] serialisation returns: whether it crossed a list, whose
/// elements were each reached by themselves.
type Crossed = bool;

impl Emit<'_, '_, '_> {
    /// Writes the scalar `value` as `serde_json` writes it.
    fn scalar<T: Serialize + ?Sized>(self, value: &T) -> Result<Crossed, Error> {
        serde_json::to_writer(&mut *self.out, value)?;
        Ok(false)
    }
}

macro_rules! emit_scalar {
    ($($method:ident($arg:ty)),* $(,)?) => {
        $(fn $method(self, value: $arg) -> Result<Crossed, Error> { self.scalar(&value) })*
    };
}

impl<'r, 'p, 'o> Serializer for Emit<'r, 'p, 'o> {
    type Ok = Crossed;
    type Error = Error;
    type SerializeSeq = EmitSeq<'r, 'p, 'o>;
    type SerializeTuple = EmitSeq<'r, 'p, 'o>;
    type SerializeTupleStruct = EmitSeq<'r, 'p, 'o>;
    type SerializeTupleVariant = Impossible<Crossed, Error>;
    type SerializeMap = EmitMap<'r, 'p, 'o>;
    type SerializeStruct = EmitMap<'r, 'p, 'o>;
    type SerializeStructVariant = Impossible<Crossed, Error>;

    emit_scalar!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f64(f64),
        serialize_char(char),
        serialize_str(&str),
        serialize_bytes(&[u8]),
    );

    fn serialize_f32(self, value: f32) -> Result<Crossed, Error> {
        // NOTE: a JSON value keeps a single as the double it widens to, and writes that.
        self.scalar(&f64::from(value))
    }

    fn serialize_none(self) -> Result<Crossed, Error> {
        self.scalar(&())
    }

    fn serialize_unit(self) -> Result<Crossed, Error> {
        self.scalar(&())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Crossed, Error> {
        self.scalar(&())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Crossed, Error> {
        self.scalar(variant)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Crossed, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Crossed, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Crossed, Error> {
        no_variant()
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<EmitSeq<'r, 'p, 'o>, Error> {
        if !self.crosses {
            self.out.push(b'[');
        }
        Ok(EmitSeq {
            reach: self.reach,
            out: self.out,
            crosses: self.crosses,
            first: true,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<EmitSeq<'r, 'p, 'o>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<EmitSeq<'r, 'p, 'o>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Impossible<Crossed, Error>, Error> {
        no_variant()
    }

    fn serialize_map(self, _: Option<usize>) -> Result<EmitMap<'r, 'p, 'o>, Error> {
        Ok(EmitMap::open(self.reach, self.out))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<EmitMap<'r, 'p, 'o>, Error> {
        Ok(EmitMap::open(self.reach, self.out))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Impossible<Crossed, Error>, Error> {
        no_variant()
    }
}

/// A list being written, or crossed with each element reached by itself.
struct EmitSeq<'r, 'p, 'o> {
    /// The path followed.
    reach: &'r mut Reach<'p>,
    /// The text written.
    out: &'o mut Vec<u8>,
    /// Whether the list is crossed.
    crosses: bool,
    /// Whether no element was written yet.
    first: bool,
}

impl EmitSeq<'_, '_, '_> {
    /// Writes the element `value`, or reaches it by itself.
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        if self.crosses {
            let text = emitted(self.reach, value)?;
            self.reach.reached.push(text);
            return Ok(());
        }
        if !self.first {
            self.out.push(b',');
        }
        self.first = false;
        value
            .serialize(Emit {
                reach: self.reach,
                out: self.out,
                crosses: false,
            })
            .map(drop)
    }

    /// Closes the list: whether it was crossed.
    fn close(self) -> Crossed {
        if !self.crosses {
            self.out.push(b']');
        }
        self.crosses
    }
}

macro_rules! emit_seq {
    ($($trait:ident :: $method:ident),* $(,)?) => {
        $(impl $trait for EmitSeq<'_, '_, '_> {
            type Ok = Crossed;
            type Error = Error;

            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
                self.element(value)
            }

            fn end(self) -> Result<Crossed, Error> {
                Ok(self.close())
            }
        })*
    };
}

emit_seq!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
);

/// An object being written, its members in their own order; where the
/// subject view leaves members out, in the order its removal leaves.
struct EmitMap<'r, 'p, 'o> {
    /// The path followed.
    reach: &'r mut Reach<'p>,
    /// The text written.
    out: &'o mut Vec<u8>,
    /// Whether no member was written yet.
    first: bool,
    /// The key of the entry whose value comes next.
    key: Option<String>,
    /// The members of an object the subject view leaves members out of,
    /// each name with its text, empty for one left out, and the names left
    /// out in the order they are removed.
    held: Option<Held>,
}

/// The members of an object as written, and the names removed from it.
type Held = (Vec<(Cow<'static, str>, Option<String>)>, Vec<String>);

impl<'r, 'p, 'o> EmitMap<'r, 'p, 'o> {
    /// Opens an object.
    fn open(reach: &'r mut Reach<'p>, out: &'o mut Vec<u8>) -> Self {
        let removals: Vec<String> = reach.removals().into_iter().map(str::to_owned).collect();
        let held = (!removals.is_empty()).then(|| (Vec::new(), removals));
        if held.is_none() {
            out.push(b'{');
        }
        Self {
            reach,
            out,
            first: true,
            key: None,
            held,
        }
    }

    /// Writes the member `name` with `value`, unless the view leaves it out.
    fn member<T: Serialize + ?Sized>(
        &mut self,
        name: Cow<'static, str>,
        value: &T,
    ) -> Result<(), Error> {
        if let Some((members, removed)) = self.held.as_mut() {
            if removed.iter().any(|gone| *gone == name) {
                members.push((name, None));
                return Ok(());
            }
            self.reach.at.push(name.clone());
            let text = emitted(self.reach, value);
            self.reach.at.pop();
            members.push((name, Some(text?)));
            return Ok(());
        }
        if !self.first {
            self.out.push(b',');
        }
        self.first = false;
        serde_json::to_writer(&mut *self.out, name.as_ref())?;
        self.out.push(b':');
        self.reach.at.push(name);
        let written = value.serialize(Emit {
            reach: self.reach,
            out: self.out,
            crosses: false,
        });
        self.reach.at.pop();
        written.map(drop)
    }

    /// Closes the object.
    ///
    /// # Errors
    /// A [`serde_json::Error`] when a held member name does not write.
    fn close(self) -> Result<Crossed, Error> {
        let Some((mut members, removed)) = self.held else {
            self.out.push(b'}');
            return Ok(false);
        };
        // NOTE: the subject view removes a member by swapping the last one into its place
        // (serde_json `Map::remove` with `preserve_order`), so its members keep that order.
        for gone in &removed {
            if let Some(index) = members.iter().position(|(name, _)| name == gone) {
                members.swap_remove(index);
            }
        }
        self.out.push(b'{');
        let mut first = true;
        for (name, text) in members {
            let Some(text) = text else {
                continue;
            };
            if !first {
                self.out.push(b',');
            }
            first = false;
            serde_json::to_writer(&mut *self.out, name.as_ref())?;
            self.out.push(b':');
            self.out.extend_from_slice(text.as_bytes());
        }
        self.out.push(b'}');
        Ok(false)
    }
}

impl SerializeMap for EmitMap<'_, '_, '_> {
    type Ok = Crossed;
    type Error = Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        self.key = Some(key_text(key)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let key = self.key.take().unwrap_or_default();
        self.member(Cow::Owned(key), value)
    }

    fn end(self) -> Result<Crossed, Error> {
        self.close()
    }
}

impl SerializeStruct for EmitMap<'_, '_, '_> {
    type Ok = Crossed;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.member(Cow::Borrowed(key), value)
    }

    fn end(self) -> Result<Crossed, Error> {
        self.close()
    }
}

/// Reads a map key as the member name a JSON value gives it: a string as it
/// is, a number or a boolean as its text.
struct KeyText;

macro_rules! key_display {
    ($($method:ident($arg:ty)),* $(,)?) => {
        $(fn $method(self, value: $arg) -> Result<String, Error> { Ok(value.to_string()) })*
    };
}

/// Why a key reads as no member name.
const NO_KEY: &str = "a map key is no string, number or boolean";

impl Serializer for KeyText {
    type Ok = String;
    type Error = Error;
    type SerializeSeq = Impossible<String, Error>;
    type SerializeTuple = Impossible<String, Error>;
    type SerializeTupleStruct = Impossible<String, Error>;
    type SerializeTupleVariant = Impossible<String, Error>;
    type SerializeMap = Impossible<String, Error>;
    type SerializeStruct = Impossible<String, Error>;
    type SerializeStructVariant = Impossible<String, Error>;

    key_display!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_char(char),
        serialize_str(&str),
    );

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<String, Error> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<String, Error> {
        value.serialize(self)
    }

    fn serialize_f32(self, _: f32) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_f64(self, _: f64) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_none(self) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_unit(self) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<String, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(ser::Error::custom(NO_KEY))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(ser::Error::custom(NO_KEY))
    }
}

#[cfg(test)]
mod tests;
