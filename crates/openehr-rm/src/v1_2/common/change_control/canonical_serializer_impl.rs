// @generated-from-template templates/openehr-rm/common/change_control/canonical_serializer_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The RFC 8785 canonical form written from a value's `Serialize` impl, with no
//! `serde_json::Value` in between (#5849); the bytes equal
//! `canonical_json(&serde_json::to_value(x))`. A name given twice keeps its
//! last value.

use std::cmp::Ordering;
use std::ops::Range;

use serde::Serialize;
use serde::ser::{self, Impossible};
use serde_json::Number;

use super::canonical_form_impl::{canonical_json, utf16_order, write_number, write_string};

/// Returns the RFC 8785 canonical form of `value`.
///
/// # Errors
/// A [`serde_json::Error`] wherever `serde_json::to_value` of `value` fails
/// (a map key that is no string, a number past 64 bits) or a number has no
/// finite IEEE 754 double form.
pub fn canonical_to_string<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    canonical_object_without(value, &[])
}

/// Returns the RFC 8785 canonical form of `value` without the top-level
/// members named in `skip`; a non-object is written whole.
///
/// # Errors
/// As [`canonical_to_string`].
pub fn canonical_object_without<T: Serialize + ?Sized>(
    value: &T,
    skip: &[&str],
) -> Result<String, serde_json::Error> {
    let mut canonical = Canonical {
        out: String::new(),
        scratch: String::new(),
        names: String::new(),
        members: Vec::new(),
        depth: 0,
        skip,
    };
    value.serialize(&mut canonical)?;
    Ok(canonical.out)
}

/// An RFC 8785 object assembled member by member, in any order.
#[derive(Debug)]
pub struct CanonicalObject {
    /// The writer the members are written into, at depth one.
    canonical: Canonical<'static>,
}

impl Default for CanonicalObject {
    fn default() -> Self {
        Self {
            canonical: Canonical {
                out: String::from("{"),
                scratch: String::new(),
                names: String::new(),
                members: Vec::new(),
                depth: 1,
                skip: &[],
            },
        }
    }
}

impl CanonicalObject {
    /// The object being assembled, as the writer's open object.
    fn object(&mut self) -> Object<'_, 'static> {
        Object {
            canonical: &mut self.canonical,
            start: 0,
            first: 0,
            names_start: 0,
            pending: Pending::None,
        }
    }

    /// Adds the member `name` with the canonical form of `value`.
    ///
    /// # Errors
    /// As [`canonical_to_string`] of `value`.
    pub fn member<T: Serialize + ?Sized>(
        &mut self,
        name: &str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.object().field(name, value)
    }

    /// Adds the member `name` whose value is `canonical`, a canonical form
    /// this module wrote.
    pub fn member_canonical(&mut self, name: &str, canonical: &str) {
        let mut object = self.object();
        let from = object.canonical.names.len();
        object.canonical.names.push_str(name);
        object.named(from);
        if let Pending::Member(name, at) = std::mem::take(&mut object.pending) {
            object.canonical.out.push_str(canonical);
            let text = at..object.canonical.out.len();
            object.canonical.members.push(Member { name, text });
        }
    }

    /// Whether no member was added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.canonical.members.is_empty()
    }

    /// Returns the canonical form of the object.
    ///
    /// # Errors
    /// Never for members this module wrote; a [`serde_json::Error`] marks a
    /// member text outside the object's buffer.
    pub fn finish(mut self) -> Result<String, serde_json::Error> {
        self.object().close()?;
        Ok(self.canonical.out)
    }
}

/// The name `serde_json` gives the one field of a raw JSON value.
const RAW_VALUE: &str = "$serde_json::private::RawValue";

/// One run of the writer.
#[derive(Debug)]
struct Canonical<'s> {
    /// The canonical form written so far.
    out: String,
    /// The buffer an object's sorted members are assembled in.
    scratch: String,
    /// The raw names of the members of every open object.
    names: String,
    /// The members of every open object, outermost first.
    members: Vec<Member>,
    /// How many arrays and objects enclose the value being written.
    depth: usize,
    /// The members of the top-level object left out.
    skip: &'s [&'s str],
}

/// One member of an open object.
#[derive(Debug, Clone)]
struct Member {
    /// Its raw name, in [`Canonical::names`].
    name: Range<usize>,
    /// Its `"name":value` text, in [`Canonical::out`].
    text: Range<usize>,
}

impl<'s> Canonical<'s> {
    /// Writes `number` as the value `serde_json` would build of it.
    fn number(&mut self, number: impl Into<Number>) -> Result<(), serde_json::Error> {
        write_number(&number.into(), &mut self.out)
    }

    /// Writes `float` as `serde_json` builds it: a number when finite, else
    /// `null`.
    fn float(&mut self, float: f64) -> Result<(), serde_json::Error> {
        if let Some(number) = Number::from_f64(float) {
            write_number(&number, &mut self.out)
        } else {
            self.out.push_str("null");
            Ok(())
        }
    }

    /// Opens an array.
    fn open_array(&mut self) -> Array<'_, 's> {
        self.out.push('[');
        self.depth += 1;
        Array {
            canonical: self,
            first: true,
        }
    }

    /// Opens an object.
    fn open_object(&mut self) -> Object<'_, 's> {
        self.depth += 1;
        let start = self.out.len();
        self.out.push('{');
        Object {
            start,
            first: self.members.len(),
            names_start: self.names.len(),
            pending: Pending::None,
            canonical: self,
        }
    }

    /// Whether a member named `name` of the object at the current depth is
    /// left out.
    fn skipped(&self, name: &str) -> bool {
        self.depth == 1 && self.skip.contains(&name)
    }
}

/// A writer that has opened an array.
#[derive(Debug)]
struct Array<'c, 's> {
    canonical: &'c mut Canonical<'s>,
    first: bool,
}

impl Array<'_, '_> {
    /// Writes `value` as the next element.
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), serde_json::Error> {
        if !self.first {
            self.canonical.out.push(',');
        }
        self.first = false;
        value.serialize(&mut *self.canonical)
    }

    /// Closes the array.
    fn close(self) {
        self.canonical.out.push(']');
        self.canonical.depth -= 1;
    }
}

/// A writer that has opened an object.
#[derive(Debug)]
struct Object<'c, 's> {
    canonical: &'c mut Canonical<'s>,
    /// Where the object starts in the output.
    start: usize,
    /// Its first member in [`Canonical::members`].
    first: usize,
    /// Where its names start in [`Canonical::names`].
    names_start: usize,
    /// The member whose key was written and whose value was not.
    pending: Pending,
}

/// The member of an object whose key was written and whose value was not.
#[derive(Debug, Default)]
enum Pending {
    /// No key waits for its value.
    #[default]
    None,
    /// The key names a member left out.
    Skipped,
    /// The member's raw name, and where its text starts in the output.
    Member(Range<usize>, usize),
}

impl Object<'_, '_> {
    /// Takes the name the key serializer wrote from `from` on, and writes its
    /// `"name":` unless the member is skipped.
    #[expect(
        clippy::expect_used,
        reason = "`from` is the length of `names` before the key was appended, and appending \
                  only pushes whole `str`s, so `from..len` is a char-boundary range of it"
    )]
    fn named(&mut self, from: usize) {
        let canonical = &mut *self.canonical;
        let range = from..canonical.names.len();
        let name = canonical
            .names
            .get(range.clone())
            .expect("a key should be a whole range of the names buffer");
        if canonical.skipped(name) {
            canonical.names.truncate(from);
            self.pending = Pending::Skipped;
            return;
        }
        if canonical.members.len() > self.first {
            canonical.out.push(',');
        }
        let at = canonical.out.len();
        write_string(name, &mut canonical.out);
        canonical.out.push(':');
        self.pending = Pending::Member(range, at);
    }

    /// Writes `key` as the next member's name.
    fn key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), serde_json::Error> {
        let from = self.canonical.names.len();
        key.serialize(MapKey {
            names: &mut self.canonical.names,
        })?;
        self.named(from);
        Ok(())
    }

    /// Writes `value` as the value of the member whose key was written last.
    fn value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), serde_json::Error> {
        match std::mem::take(&mut self.pending) {
            Pending::Member(name, at) => {
                value.serialize(&mut *self.canonical)?;
                let text = at..self.canonical.out.len();
                self.canonical.members.push(Member { name, text });
                Ok(())
            }
            Pending::Skipped => Ok(()),
            Pending::None => Err(ser::Error::custom("a map value was given before its key")),
        }
    }

    /// Writes the member `name` with `value`.
    fn field<T: Serialize + ?Sized>(
        &mut self,
        name: &str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        let from = self.canonical.names.len();
        self.canonical.names.push_str(name);
        self.named(from);
        self.value(value)
    }

    /// Closes the object, sorting and deduplicating its members when they were
    /// not written in name order.
    #[expect(
        clippy::expect_used,
        reason = "`first` is the member count when the object opened and members are only \
                  pushed after it, and each name range was taken from the names buffer and \
                  names are truncated only when their object closes"
    )]
    fn close(self) -> Result<(), serde_json::Error> {
        let Canonical {
            out,
            scratch,
            names,
            members,
            depth,
            ..
        } = self.canonical;
        let own = members
            .get_mut(self.first..)
            .expect("an object's members should follow its first");
        let name = |member: &Member| {
            names
                .get(member.name.clone())
                .expect("a member's name should lie in the names buffer")
        };
        let ordered = own
            .iter()
            .zip(own.iter().skip(1))
            .all(|(a, b)| utf16_order(name(a), name(b)) == Ordering::Less);
        if ordered {
            out.push('}');
            members.truncate(self.first);
            names.truncate(self.names_start);
            *depth -= 1;
            return Ok(());
        }
        own.sort_by(|a, b| utf16_order(name(a), name(b)));
        scratch.clear();
        scratch.push('{');
        let mut written = 0_usize;
        for (index, member) in own.iter().enumerate() {
            let next_is_same = own
                .get(index + 1)
                .is_some_and(|next| name(next) == name(member));
            if next_is_same {
                continue;
            }
            if written > 0 {
                scratch.push(',');
            }
            written += 1;
            scratch.push_str(out.get(member.text.clone()).ok_or_else(|| {
                ser::Error::custom("a member's text lies outside the canonical form")
            })?);
        }
        scratch.push('}');
        out.truncate(self.start);
        out.push_str(scratch);
        members.truncate(self.first);
        names.truncate(self.names_start);
        *depth -= 1;
        Ok(())
    }
}

impl<'c, 's> ser::Serializer for &'c mut Canonical<'s> {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = Array<'c, 's>;
    type SerializeTuple = Array<'c, 's>;
    type SerializeTupleStruct = Array<'c, 's>;
    type SerializeTupleVariant = Variant<'c, 's>;
    type SerializeMap = Object<'c, 's>;
    type SerializeStruct = Struct<'c, 's>;
    type SerializeStructVariant = Variant<'c, 's>;

    fn serialize_bool(self, value: bool) -> Result<(), serde_json::Error> {
        self.out.push_str(if value { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i8(self, value: i8) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_i16(self, value: i16) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_i32(self, value: i32) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_i64(self, value: i64) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_i128(self, value: i128) -> Result<(), serde_json::Error> {
        if let Ok(value) = u64::try_from(value) {
            self.number(value)
        } else if let Ok(value) = i64::try_from(value) {
            self.number(value)
        } else {
            Err(ser::Error::custom("number out of range"))
        }
    }

    fn serialize_u8(self, value: u8) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_u16(self, value: u16) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_u32(self, value: u32) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_u64(self, value: u64) -> Result<(), serde_json::Error> {
        self.number(value)
    }

    fn serialize_u128(self, value: u128) -> Result<(), serde_json::Error> {
        match u64::try_from(value) {
            Ok(value) => self.number(value),
            Err(_) => Err(ser::Error::custom("number out of range")),
        }
    }

    fn serialize_f32(self, value: f32) -> Result<(), serde_json::Error> {
        self.float(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> Result<(), serde_json::Error> {
        self.float(value)
    }

    fn serialize_char(self, value: char) -> Result<(), serde_json::Error> {
        let mut buffer = [0_u8; 4];
        write_string(value.encode_utf8(&mut buffer), &mut self.out);
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<(), serde_json::Error> {
        write_string(value, &mut self.out);
        Ok(())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), serde_json::Error> {
        let mut array = self.open_array();
        for byte in value {
            array.element(byte)?;
        }
        array.close();
        Ok(())
    }

    fn serialize_none(self) -> Result<(), serde_json::Error> {
        self.serialize_unit()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), serde_json::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), serde_json::Error> {
        self.out.push_str("null");
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), serde_json::Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), serde_json::Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        let mut object = self.open_object();
        object.field(variant, value)?;
        object.close()
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Array<'c, 's>, serde_json::Error> {
        Ok(self.open_array())
    }

    fn serialize_tuple(self, _len: usize) -> Result<Array<'c, 's>, serde_json::Error> {
        Ok(self.open_array())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Array<'c, 's>, serde_json::Error> {
        Ok(self.open_array())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Variant<'c, 's>, serde_json::Error> {
        Ok(Variant::open(self, variant, true))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Object<'c, 's>, serde_json::Error> {
        Ok(self.open_object())
    }

    fn serialize_struct(
        self,
        name: &'static str,
        _len: usize,
    ) -> Result<Struct<'c, 's>, serde_json::Error> {
        // NOTE: serde_json's `raw_value` feature, on in this workspace, writes
        // a raw value as this one-field struct, and `to_value` parses it.
        if name == RAW_VALUE {
            return Ok(Struct::Raw(self));
        }
        Ok(Struct::Object(self.open_object()))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Variant<'c, 's>, serde_json::Error> {
        Ok(Variant::open(self, variant, false))
    }
}

impl ser::SerializeSeq for Array<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_element<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), serde_json::Error> {
        self.close();
        Ok(())
    }
}

impl ser::SerializeTuple for Array<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_element<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), serde_json::Error> {
        self.close();
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Array<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), serde_json::Error> {
        self.close();
        Ok(())
    }
}

impl ser::SerializeMap for Object<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), serde_json::Error> {
        self.key(key)
    }

    fn serialize_value<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.value(value)
    }

    fn end(self) -> Result<(), serde_json::Error> {
        self.close()
    }
}

/// A struct being written: an object, or a raw JSON value.
#[derive(Debug)]
enum Struct<'c, 's> {
    /// An ordinary struct, written as an object.
    Object(Object<'c, 's>),
    /// `serde_json`'s raw value, written as the canonical form of its text.
    Raw(&'c mut Canonical<'s>),
}

impl ser::SerializeStruct for Struct<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    #[expect(
        clippy::disallowed_types,
        reason = "a raw JSON value is parsed as `serde_json::to_value` parses it, to write its \
                  canonical form (#1694 boundary class: canonical form)"
    )]
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        name: &'static str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        match self {
            Self::Object(object) => object.field(name, value),
            Self::Raw(canonical) => {
                let mut text = String::new();
                value.serialize(MapKey { names: &mut text })?;
                let parsed: serde_json::Value = serde_json::from_str(&text)?;
                canonical.out.push_str(&canonical_json(&parsed)?);
                Ok(())
            }
        }
    }

    fn end(self) -> Result<(), serde_json::Error> {
        match self {
            Self::Object(object) => object.close(),
            Self::Raw(_) => Ok(()),
        }
    }
}

/// A tuple or struct variant: the object `{variant: ...}` around an array or
/// an object.
#[derive(Debug)]
struct Variant<'c, 's> {
    /// The outer object, its one member open.
    outer: Object<'c, 's>,
    /// Where the inner value starts in the output.
    inner_start: usize,
    /// The inner array's state, or the inner object's.
    inner: Inner,
}

/// The state of a variant's inner value.
#[derive(Debug)]
enum Inner {
    /// An array, with whether no element was written yet.
    Array(bool),
    /// An object, with where its members and names start.
    Object {
        first: usize,
        names_start: usize,
        pending: Pending,
    },
}

impl<'c, 's> Variant<'c, 's> {
    /// Opens `{variant:` and then `[` or the inner object.
    fn open(canonical: &'c mut Canonical<'s>, variant: &'static str, array: bool) -> Self {
        let mut outer = canonical.open_object();
        let from = outer.canonical.names.len();
        outer.canonical.names.push_str(variant);
        outer.named(from);
        let inner_start = outer.canonical.out.len();
        outer.canonical.depth += 1;
        let inner = if array {
            outer.canonical.out.push('[');
            Inner::Array(true)
        } else {
            outer.canonical.out.push('{');
            Inner::Object {
                first: outer.canonical.members.len(),
                names_start: outer.canonical.names.len(),
                pending: Pending::None,
            }
        };
        Self {
            outer,
            inner_start,
            inner,
        }
    }

    /// The inner object as an [`Object`] over the same writer, run on `f`,
    /// its state kept.
    fn with_inner<R>(
        &mut self,
        f: impl FnOnce(&mut Object<'_, '_>) -> Result<R, serde_json::Error>,
    ) -> Result<R, serde_json::Error> {
        let Inner::Object {
            first,
            names_start,
            pending,
        } = &mut self.inner
        else {
            return Err(ser::Error::custom("a field was given to a tuple variant"));
        };
        let mut object = Object {
            canonical: &mut *self.outer.canonical,
            start: self.inner_start,
            first: *first,
            names_start: *names_start,
            pending: std::mem::take(pending),
        };
        let result = f(&mut object);
        *pending = std::mem::take(&mut object.pending);
        result
    }

    /// Closes the inner value and then the outer object.
    fn close(mut self) -> Result<(), serde_json::Error> {
        match self.inner {
            Inner::Array(_) => {
                self.outer.canonical.out.push(']');
                self.outer.canonical.depth -= 1;
            }
            Inner::Object {
                first, names_start, ..
            } => {
                Object {
                    canonical: &mut *self.outer.canonical,
                    start: self.inner_start,
                    first,
                    names_start,
                    pending: Pending::None,
                }
                .close()?;
            }
        }
        match std::mem::take(&mut self.outer.pending) {
            Pending::Member(name, at) => {
                let text = at..self.outer.canonical.out.len();
                self.outer.canonical.members.push(Member { name, text });
            }
            Pending::Skipped => self.outer.canonical.out.truncate(self.inner_start),
            Pending::None => {}
        }
        self.outer.close()
    }
}

impl ser::SerializeTupleVariant for Variant<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        let Inner::Array(first) = &mut self.inner else {
            return Err(ser::Error::custom(
                "an element was given to a struct variant",
            ));
        };
        if !*first {
            self.outer.canonical.out.push(',');
        }
        *first = false;
        value.serialize(&mut *self.outer.canonical)
    }

    fn end(self) -> Result<(), serde_json::Error> {
        self.close()
    }
}

impl ser::SerializeStructVariant for Variant<'_, '_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        name: &'static str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.with_inner(|object| object.field(name, value))
    }

    fn end(self) -> Result<(), serde_json::Error> {
        self.close()
    }
}

/// Writes a map key as `serde_json`'s map key serializer reads it: a string,
/// a character, a boolean, an integer or a finite float as its text.
#[derive(Debug)]
struct MapKey<'n> {
    names: &'n mut String,
}

/// The error `serde_json` gives a map key that is no string.
fn key_must_be_a_string() -> serde_json::Error {
    ser::Error::custom("key must be a string")
}

impl MapKey<'_> {
    /// Appends `text` as the key.
    fn text(self, text: &str) {
        self.names.push_str(text);
    }

    /// Appends the finite `float` as `serde_json` writes it.
    fn float<F: Serialize + Into<f64> + Copy>(self, float: F) -> Result<(), serde_json::Error> {
        if !float.into().is_finite() {
            return Err(ser::Error::custom(
                "float key must be finite (got NaN or +/-inf)",
            ));
        }
        let text = serde_json::to_string(&float)?;
        self.text(&text);
        Ok(())
    }
}

impl ser::Serializer for MapKey<'_> {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = Impossible<(), serde_json::Error>;
    type SerializeTuple = Impossible<(), serde_json::Error>;
    type SerializeTupleStruct = Impossible<(), serde_json::Error>;
    type SerializeTupleVariant = Impossible<(), serde_json::Error>;
    type SerializeMap = Impossible<(), serde_json::Error>;
    type SerializeStruct = Impossible<(), serde_json::Error>;
    type SerializeStructVariant = Impossible<(), serde_json::Error>;

    fn serialize_bool(self, value: bool) -> Result<(), serde_json::Error> {
        self.text(if value { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i8(self, value: i8) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_i16(self, value: i16) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_i32(self, value: i32) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_i64(self, value: i64) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_i128(self, value: i128) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_u8(self, value: u8) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_u16(self, value: u16) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_u32(self, value: u32) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_u64(self, value: u64) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_u128(self, value: u128) -> Result<(), serde_json::Error> {
        self.text(&value.to_string());
        Ok(())
    }

    fn serialize_f32(self, value: f32) -> Result<(), serde_json::Error> {
        self.float(value)
    }

    fn serialize_f64(self, value: f64) -> Result<(), serde_json::Error> {
        self.float(value)
    }

    fn serialize_char(self, value: char) -> Result<(), serde_json::Error> {
        let mut buffer = [0_u8; 4];
        self.text(value.encode_utf8(&mut buffer));
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<(), serde_json::Error> {
        self.text(value);
        Ok(())
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<(), serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_none(self) -> Result<(), serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<(), serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_unit(self) -> Result<(), serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), serde_json::Error> {
        self.text(variant);
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_seq(
        self,
        _len: Option<usize>,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_tuple(
        self,
        _len: usize,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_map(
        self,
        _len: Option<usize>,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Impossible<(), serde_json::Error>, serde_json::Error> {
        Err(key_must_be_a_string())
    }
}
