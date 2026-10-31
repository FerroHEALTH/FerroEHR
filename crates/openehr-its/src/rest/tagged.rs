// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Decoders for ITS-REST payload members that must name their `_type`.
//!
//! A canonical-JSON RM reader takes an object without `_type` as the slot's
//! own concrete class, so a `DV_TEXT` slot reads `{"value": "x"}` as a
//! `DV_TEXT`. Some ITS-REST payload members are stricter: the validation
//! bundle types them with a `U*` union whose every member lists `_type` under
//! `required` (`schemas/data_types/UDvText.yaml` over `UMDvText.yaml` and
//! `UMDvCodedText.yaml`), and ITS-REST `docs/overview/Specifications.md` names
//! that bundle as the one used "to validate openEHR request and response
//! payloads". The generated DTO fields `emit-rest` marks for this
//! (`plan::rest_tagged` in the generator) decode through
//! [`required`] or [`optional`], which pass the object to the RM reader
//! unchanged and refuse it when it ends without a `_type` member.

use std::fmt;
use std::marker::PhantomData;

use serde::de::value::StringDeserializer;
use serde::de::{DeserializeSeed, Deserializer, Error, MapAccess, Visitor};
use serde::{Deserialize, forward_to_deserialize_any};

/// The `_type` member name of a canonical-JSON object.
const TYPE_KEY: &str = "_type";

/// Decodes a member that must be an object carrying `_type`.
///
/// # Errors
/// The deserializer's error when the value is not an object, when the object
/// carries no `_type` member, or when `T` refuses it.
pub fn required<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_map(TaggedVisitor(PhantomData))
}

/// Decodes an optional member that, when present and not `null`, must be an
/// object carrying `_type`.
///
/// # Errors
/// As [`required`], for a present non-`null` value.
pub fn optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_option(OptionalVisitor(PhantomData))
}

/// The [`optional`] visitor: `null` is `None`, anything else goes to
/// [`required`].
struct OptionalVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for OptionalVisitor<T> {
    type Value = Option<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("null or an object carrying `_type`")
    }

    fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        required(deserializer).map(Some)
    }
}

/// The [`required`] visitor: hands the map to `T` through [`Witness`].
struct TaggedVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for TaggedVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object carrying `_type`")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        T::deserialize(Witness { map, seen: false })
    }
}

/// A map that forwards every entry to `T` and refuses to end before a `_type`
/// key has passed.
struct Witness<A> {
    /// The map being read.
    map: A,
    /// Whether a `_type` key has passed.
    seen: bool,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Witness<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        match self.map.next_key::<String>()? {
            Some(key) => {
                self.seen |= key == TYPE_KEY;
                seed.deserialize(StringDeserializer::<A::Error>::new(key))
                    .map(Some)
            }
            None if self.seen => Ok(None),
            None => Err(A::Error::custom(
                "missing required `_type`: the ITS-REST validation schema types this \
                 member with a union whose every member requires `_type`",
            )),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        self.map.next_value_seed(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        self.map.size_hint()
    }
}

impl<'de, A: MapAccess<'de>> Deserializer<'de> for Witness<A> {
    type Error = A::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(self)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use openehr_rm::prelude::DvText;
    use serde::Deserialize;

    /// A carrier with one tagged member of each kind.
    #[derive(Debug, Deserialize)]
    struct Carrier {
        #[serde(deserialize_with = "super::required")]
        reason: DvText,
        #[serde(default, deserialize_with = "super::optional")]
        description: Option<DvText>,
    }

    fn read(json: &str) -> Result<Carrier, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn tagged_members_decode_in_both_forms() {
        let c = read(
            r#"{"reason": {"value": "r", "_type": "DV_TEXT"},
                "description": {"_type": "DV_CODED_TEXT", "value": "d",
                  "defining_code": {"terminology_id": {"value": "local"}, "code_string": "1"}}}"#,
        )
        .expect("tagged members decode");
        assert!(matches!(c.reason, DvText::DvText(_)));
        assert!(matches!(c.description, Some(DvText::DvCodedText(_))));
    }

    #[test]
    fn an_absent_or_null_optional_member_is_none() {
        let absent = read(r#"{"reason": {"_type": "DV_TEXT", "value": "r"}}"#).expect("absent");
        assert!(absent.description.is_none());
        let null = read(r#"{"reason": {"_type": "DV_TEXT", "value": "r"}, "description": null}"#)
            .expect("null");
        assert!(null.description.is_none());
    }

    #[test]
    fn an_untagged_member_is_refused() {
        let required = read(r#"{"reason": {"value": "r"}}"#).expect_err("untagged reason");
        assert!(
            required.to_string().contains("missing required `_type`"),
            "{required}"
        );
        let optional = read(
            r#"{"reason": {"_type": "DV_TEXT", "value": "r"}, "description": {"value": "d"}}"#,
        )
        .expect_err("untagged description");
        assert!(
            optional.to_string().contains("missing required `_type`"),
            "{optional}"
        );
    }

    #[test]
    fn a_non_object_member_is_refused() {
        read(r#"{"reason": "plain"}"#).expect_err("a string is not a DV_TEXT object");
    }

    #[test]
    fn the_rm_reader_still_judges_the_object() {
        read(r#"{"reason": {"_type": "DV_TEXT", "value": "r", "undeclared": 1}}"#)
            .expect_err("the strict RM reader refuses an undeclared key");
        read(r#"{"reason": {"_type": "DV_QUANTITY", "magnitude": 1.0}}"#)
            .expect_err("a tag outside the slot is refused");
    }
}
