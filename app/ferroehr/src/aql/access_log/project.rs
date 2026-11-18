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
use emit::{Emit, emitted};

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
        format!("[{}]", reached.join(","))
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

/// The [`Serializer`] methods a cell serializer with output `$ok` shares: an
/// option's value and a newtype's inner value serialize through it, and an
/// externally tagged variant is refused ([`no_variant`]).
macro_rules! passthrough_and_no_variants {
    ($ok:ty) => {
        fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<$ok, Error> {
            value.serialize(self)
        }

        fn serialize_newtype_struct<T: Serialize + ?Sized>(
            self,
            _: &'static str,
            value: &T,
        ) -> Result<$ok, Error> {
            value.serialize(self)
        }

        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: &T,
        ) -> Result<$ok, Error> {
            no_variant()
        }

        fn serialize_tuple_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Impossible<$ok, Error>, Error> {
            no_variant()
        }

        fn serialize_struct_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Impossible<$ok, Error>, Error> {
            no_variant()
        }
    };
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

    passthrough_and_no_variants!(());

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

    fn serialize_seq(self, _: Option<usize>) -> Result<SeekSeq<'r, 'p>, Error> {
        Ok(SeekSeq(self.crosses.then_some(self)))
    }

    fn serialize_tuple(self, len: usize) -> Result<SeekSeq<'r, 'p>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<SeekSeq<'r, 'p>, Error> {
        self.serialize_seq(Some(len))
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

/// Implements the three element-sequence [`Serializer`] traits for `$seq`
/// with output `$ok`: each element goes to `element`, and `end` yields `$end`.
macro_rules! element_seq {
    ($seq:ty, $ok:ty, |$this:ident| $end:expr) => {
        element_seq!(@one SerializeSeq, serialize_element, $seq, $ok, $this, $end);
        element_seq!(@one SerializeTuple, serialize_element, $seq, $ok, $this, $end);
        element_seq!(@one SerializeTupleStruct, serialize_field, $seq, $ok, $this, $end);
    };
    (@one $trait:ident, $method:ident, $seq:ty, $ok:ty, $this:ident, $end:expr) => {
        impl $trait for $seq {
            type Ok = $ok;
            type Error = Error;

            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
                self.element(value)
            }

            fn end($this) -> Result<$ok, Error> {
                $end
            }
        }
    };
}

element_seq!(SeekSeq<'_, '_>, (), |self| Ok(()));

/// A map the path enters.
struct SeekMap<'r, 'p> {
    /// The path.
    seek: Seek<'r, 'p>,
    /// The key of the entry whose value comes next.
    key: Option<String>,
}

/// Implements [`SerializeMap`] for `$map` with output `$ok`: each entry's key
/// is read as a member name ([`key_text`]) and the entry goes to `$member`
/// with it, and `end` yields `$end`.
macro_rules! keyed_map {
    (
        $map:ty,
        $ok:ty,
        |$this:ident, $key:ident, $value:ident| $member:expr,
        |$done:ident| $end:expr $(,)?
    ) => {
        impl SerializeMap for $map {
            type Ok = $ok;
            type Error = Error;

            fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
                self.key = Some(key_text(key)?);
                Ok(())
            }

            fn serialize_value<T: Serialize + ?Sized>(&mut $this, $value: &T) -> Result<(), Error> {
                let $key = Cow::Owned($this.key.take().unwrap_or_default());
                $member
            }

            fn end($done) -> Result<$ok, Error> {
                $end
            }
        }
    };
}

keyed_map!(
    SeekMap<'_, '_>,
    (),
    |self, key, value| self.seek.step(key, value),
    |self| Ok(()),
);

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

mod emit;

#[cfg(test)]
mod tests;
