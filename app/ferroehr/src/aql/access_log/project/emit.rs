// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The serializer that writes what a path's last step reaches as JSON text,
//! its members in their own order, without the members the shown view
//! leaves out. No openEHR spec governs AQL over the access log: our own
//! design.

use std::borrow::Cow;

use serde::ser::{
    Impossible, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeTuple,
    SerializeTupleStruct, Serializer,
};

use super::{Error, Reach, key_text, no_variant, utf8};

/// The JSON text of `value` as it is shown.
pub(super) fn emitted<T: Serialize + ?Sized>(
    reach: &mut Reach<'_>,
    value: &T,
) -> Result<String, Error> {
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
pub(super) struct Emit<'r, 'p, 'o> {
    /// The path followed, which knows what is shown.
    pub(super) reach: &'r mut Reach<'p>,
    /// The text written.
    pub(super) out: &'o mut Vec<u8>,
    /// Whether a list here is crossed: each element is reached by itself.
    pub(super) crosses: bool,
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

    passthrough_and_no_variants!(Crossed);

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

    fn serialize_map(self, _: Option<usize>) -> Result<EmitMap<'r, 'p, 'o>, Error> {
        Ok(EmitMap::open(self.reach, self.out))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<EmitMap<'r, 'p, 'o>, Error> {
        Ok(EmitMap::open(self.reach, self.out))
    }
}

/// A list being written, or crossed with each element reached by itself.
pub(super) struct EmitSeq<'r, 'p, 'o> {
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

element_seq!(EmitSeq<'_, '_, '_>, Crossed, |self| Ok(self.close()));

/// An object being written, its members in their own order; where the
/// subject view leaves members out, in the order its removal leaves.
pub(super) struct EmitMap<'r, 'p, 'o> {
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

keyed_map!(
    EmitMap<'_, '_, '_>,
    Crossed,
    |self, key, value| self.member(key, value),
    |self| self.close(),
);

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
