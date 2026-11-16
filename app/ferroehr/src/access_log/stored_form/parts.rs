// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The row and shared parts of a record, written straight from its
//! `Serialize` impl into their canonical forms, each member once (#5849).
//!
//! The record's members are routed as [`super::split`] routes the members of
//! its JSON value: the shared members to the shared part, the split members'
//! attributes to the part that keeps them, and the access details' elements
//! by their at-code. Each member is written by the canonical serializer of
//! `openehr-rm`, so no `serde_json::Value` of the record is built. How a
//! record is stored is our own design; no openEHR spec governs it.

use std::fmt;

use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::{
    CanonicalObject, canonical_to_string,
};
use serde::Serialize;
use serde::ser::{self, Impossible};

use super::{DETAILS, ITEMS, NODE, SHARED, SHARED_DETAILS, SPLIT};

/// A record's stored parts, in their canonical forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    /// The row part.
    pub row: String,
    /// The shared part; `None` for a record that keeps none, such as a
    /// tombstone.
    pub shared: Option<String>,
}

/// Returns the stored parts of `record`, leaving out its top-level members
/// named in `skip`.
///
/// # Errors
/// A [`serde_json::Error`] when `record` is no object or a member does not
/// serialise to a canonical form.
pub fn parts<T: Serialize + ?Sized>(record: &T, skip: &[&str]) -> Result<Parts, serde_json::Error> {
    let mut top = Top {
        skip,
        row: CanonicalObject::default(),
        shared: CanonicalObject::default(),
    };
    record
        .serialize(ObjectOnly(&mut top))
        .map_err(RouteError::into_json)?;
    let shared = if top.shared.is_empty() {
        None
    } else {
        Some(top.shared.finish()?)
    };
    Ok(Parts {
        row: top.row.finish()?,
        shared,
    })
}

/// Why a value could not be routed.
#[derive(Debug)]
enum RouteError {
    /// The value is no object, or no array where one was asked for.
    Shape,
    /// The value is no string where one was asked for.
    NotString,
    /// A member did not serialise.
    Json(serde_json::Error),
    /// The value's `Serialize` impl refused.
    Custom(String),
}

impl RouteError {
    /// This error as the one a caller reads.
    fn into_json(self) -> serde_json::Error {
        match self {
            Self::Json(error) => error,
            other => ser::Error::custom(other),
        }
    }
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape => f.write_str("a stored record serialises as an object"),
            Self::NotString => f.write_str("a member name is no string"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Custom(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for RouteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl ser::Error for RouteError {
    fn custom<M: fmt::Display>(message: M) -> Self {
        Self::Custom(message.to_string())
    }
}

impl From<serde_json::Error> for RouteError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// What a routed object does with each of its members.
trait Route {
    /// Takes the member `name` with `value`.
    fn member<T: Serialize + ?Sized>(&mut self, name: &str, value: &T) -> Result<(), RouteError>;
}

/// The record's top level.
struct Top<'s> {
    skip: &'s [&'s str],
    row: CanonicalObject,
    shared: CanonicalObject,
}

impl Route for Top<'_> {
    fn member<T: Serialize + ?Sized>(&mut self, name: &str, value: &T) -> Result<(), RouteError> {
        if self.skip.contains(&name) {
            return Ok(());
        }
        if SHARED.contains(&name) {
            return Ok(self.shared.member(name, value)?);
        }
        if let Some((_, own_keys)) = SPLIT.iter().find(|(split, _)| *split == name) {
            let mut split = Split {
                own_keys,
                own: CanonicalObject::default(),
                common: CanonicalObject::default(),
            };
            match value.serialize(ObjectOnly(&mut split)) {
                Ok(()) => {
                    if !split.own.is_empty() {
                        self.row.member_canonical(name, &split.own.finish()?);
                    }
                    self.shared.member_canonical(name, &split.common.finish()?);
                    return Ok(());
                }
                Err(RouteError::Shape) => return Ok(self.row.member(name, value)?),
                Err(error) => return Err(error),
            }
        }
        if name == DETAILS {
            let mut details = Details {
                envelope: CanonicalObject::default(),
                own: Vec::new(),
                common: Vec::new(),
            };
            match value.serialize(ObjectOnly(&mut details)) {
                Ok(()) => {
                    details
                        .envelope
                        .member_canonical(ITEMS, &array(&details.common));
                    self.shared
                        .member_canonical(DETAILS, &details.envelope.finish()?);
                    let mut kept = CanonicalObject::default();
                    kept.member_canonical(ITEMS, &array(&details.own));
                    self.row.member_canonical(DETAILS, &kept.finish()?);
                    return Ok(());
                }
                Err(RouteError::Shape) => return Ok(self.row.member(name, value)?),
                Err(error) => return Err(error),
            }
        }
        Ok(self.row.member(name, value)?)
    }
}

/// The canonical array of the canonical forms `elements`.
fn array(elements: &[String]) -> String {
    let mut out = String::from("[");
    for (index, element) in elements.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(element);
    }
    out.push(']');
    out
}

/// A split member: its attributes in `own_keys` go to the row, the rest to
/// the shared part.
struct Split<'k> {
    own_keys: &'k [&'k str],
    own: CanonicalObject,
    common: CanonicalObject,
}

impl Route for Split<'_> {
    fn member<T: Serialize + ?Sized>(&mut self, name: &str, value: &T) -> Result<(), RouteError> {
        let part = if self.own_keys.contains(&name) {
            &mut self.own
        } else {
            &mut self.common
        };
        Ok(part.member(name, value)?)
    }
}

/// The access details: the envelope and the shared elements go to the
/// shared part, the other elements to the row.
struct Details {
    envelope: CanonicalObject,
    own: Vec<String>,
    common: Vec<String>,
}

impl Route for Details {
    fn member<T: Serialize + ?Sized>(&mut self, name: &str, value: &T) -> Result<(), RouteError> {
        if name == ITEMS {
            let mut elements = Elements {
                own: Vec::new(),
                common: Vec::new(),
            };
            match value.serialize(ArrayOnly(&mut elements)) {
                Ok(()) => {
                    self.own = elements.own;
                    self.common = elements.common;
                    return Ok(());
                }
                Err(RouteError::Shape) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(self.envelope.member(name, value)?)
    }
}

/// The elements of the access details, each written once and kept by the
/// part its at-code names.
struct Elements {
    own: Vec<String>,
    common: Vec<String>,
}

impl Elements {
    /// Takes the next element.
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), RouteError> {
        let mut probe = Probe { code: None };
        match value.serialize(ObjectOnly(&mut probe)) {
            Ok(()) | Err(RouteError::Shape) => {}
            Err(error) => return Err(error),
        }
        let text = canonical_to_string(value)?;
        if probe
            .code
            .is_some_and(|code| SHARED_DETAILS.contains(&code.as_str()))
        {
            self.common.push(text);
        } else {
            self.own.push(text);
        }
        Ok(())
    }
}

/// Reads an element's at-code.
struct Probe {
    code: Option<String>,
}

impl Route for Probe {
    fn member<T: Serialize + ?Sized>(&mut self, name: &str, value: &T) -> Result<(), RouteError> {
        if name == NODE {
            // NOTE: an at-code that is no string names no shared detail, as
            // `shared_detail` reads it, so the element stays on the row.
            self.code = match value.serialize(StrOnly) {
                Ok(code) => Some(code),
                Err(RouteError::NotString) => None,
                Err(error) => return Err(error),
            };
        }
        Ok(())
    }
}

/// Writes the methods of a serializer that refuse a shape, each with `$error`.
macro_rules! refuse {
    ($ok:ty, $error:expr; $($method:ident($($arg:ty),*)),* $(,)?) => {
        $(
            fn $method(self, $(_: $arg),*) -> Result<$ok, RouteError> {
                Err($error)
            }
        )*
    };
}

/// The refusals every routing serializer shares: every scalar.
macro_rules! refuse_scalars {
    ($ok:ty, $error:expr) => {
        refuse!($ok, $error;
            serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32),
            serialize_i64(i64), serialize_i128(i128), serialize_u8(u8), serialize_u16(u16),
            serialize_u32(u32), serialize_u64(u64), serialize_u128(u128), serialize_f32(f32),
            serialize_f64(f64), serialize_char(char), serialize_bytes(&[u8]),
            serialize_none(), serialize_unit(), serialize_unit_struct(&'static str),
            serialize_unit_variant(&'static str, u32, &'static str),
        );
        fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<$ok, RouteError> {
            value.serialize(self)
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(
            self,
            _name: &'static str,
            value: &T,
        ) -> Result<$ok, RouteError> {
            value.serialize(self)
        }
    };
}

/// Routes the members of an object to `R`; refuses any other shape with
/// [`RouteError::Shape`].
struct ObjectOnly<'r, R>(&'r mut R);

/// An open object being routed, with the name of the member whose value
/// comes next.
struct OpenObject<'r, R> {
    route: &'r mut R,
    key: Option<String>,
}

impl<'r, R: Route> ser::Serializer for ObjectOnly<'r, R> {
    type Ok = ();
    type Error = RouteError;
    type SerializeSeq = Impossible<(), RouteError>;
    type SerializeTuple = Impossible<(), RouteError>;
    type SerializeTupleStruct = Impossible<(), RouteError>;
    type SerializeTupleVariant = Impossible<(), RouteError>;
    type SerializeMap = OpenObject<'r, R>;
    type SerializeStruct = OpenObject<'r, R>;
    type SerializeStructVariant = Impossible<(), RouteError>;

    refuse_scalars!((), RouteError::Shape);

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), RouteError> {
        self.0.member(variant, value)
    }

    fn serialize_str(self, _value: &str) -> Result<(), RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, RouteError> {
        Err(ser::Error::custom(
            "a stored record's member is a tuple variant, which no access-log codec writes",
        ))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<OpenObject<'r, R>, RouteError> {
        Ok(OpenObject {
            route: self.0,
            key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<OpenObject<'r, R>, RouteError> {
        Ok(OpenObject {
            route: self.0,
            key: None,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, RouteError> {
        Err(ser::Error::custom(
            "a stored record's member is a struct variant, which no access-log codec writes",
        ))
    }
}

impl<R: Route> ser::SerializeMap for OpenObject<'_, R> {
    type Ok = ();
    type Error = RouteError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), RouteError> {
        self.key = Some(key.serialize(StrOnly)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), RouteError> {
        let Some(key) = self.key.take() else {
            return Err(ser::Error::custom("a map value was given before its key"));
        };
        self.route.member(&key, value)
    }

    fn end(self) -> Result<(), RouteError> {
        Ok(())
    }
}

impl<R: Route> ser::SerializeStruct for OpenObject<'_, R> {
    type Ok = ();
    type Error = RouteError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        name: &'static str,
        value: &T,
    ) -> Result<(), RouteError> {
        self.route.member(name, value)
    }

    fn end(self) -> Result<(), RouteError> {
        Ok(())
    }
}

/// Hands each element of an array to [`Elements`]; refuses any other shape
/// with [`RouteError::Shape`].
struct ArrayOnly<'e>(&'e mut Elements);

impl ser::Serializer for ArrayOnly<'_> {
    type Ok = ();
    type Error = RouteError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Impossible<(), RouteError>;
    type SerializeTupleVariant = Impossible<(), RouteError>;
    type SerializeMap = Impossible<(), RouteError>;
    type SerializeStruct = Impossible<(), RouteError>;
    type SerializeStructVariant = Impossible<(), RouteError>;

    refuse_scalars!((), RouteError::Shape);

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_str(self, _value: &str) -> Result<(), RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self, RouteError> {
        Ok(self)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self, RouteError> {
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, RouteError> {
        Err(RouteError::Shape)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, RouteError> {
        Err(RouteError::Shape)
    }
}

impl ser::SerializeSeq for ArrayOnly<'_> {
    type Ok = ();
    type Error = RouteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), RouteError> {
        self.0.element(value)
    }

    fn end(self) -> Result<(), RouteError> {
        Ok(())
    }
}

impl ser::SerializeTuple for ArrayOnly<'_> {
    type Ok = ();
    type Error = RouteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), RouteError> {
        self.0.element(value)
    }

    fn end(self) -> Result<(), RouteError> {
        Ok(())
    }
}

/// Reads a string; refuses any other shape with [`RouteError::NotString`].
struct StrOnly;

impl ser::Serializer for StrOnly {
    type Ok = String;
    type Error = RouteError;
    type SerializeSeq = Impossible<String, RouteError>;
    type SerializeTuple = Impossible<String, RouteError>;
    type SerializeTupleStruct = Impossible<String, RouteError>;
    type SerializeTupleVariant = Impossible<String, RouteError>;
    type SerializeMap = Impossible<String, RouteError>;
    type SerializeStruct = Impossible<String, RouteError>;
    type SerializeStructVariant = Impossible<String, RouteError>;

    refuse_scalars!(String, RouteError::NotString);

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<String, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_str(self, value: &str) -> Result<String, RouteError> {
        Ok(value.to_owned())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, RouteError> {
        Err(RouteError::NotString)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, RouteError> {
        Err(RouteError::NotString)
    }
}

#[cfg(test)]
mod tests;
