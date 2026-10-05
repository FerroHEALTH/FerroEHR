// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The small shared runtime the emitted canonical-JSON `serde` impls call into.
//!
//! Hand-written; preserved across `openehr-codegen` regeneration (it carries no
//! generated-file marker, so `write_crate` keeps it and `lib.rs` auto-declares
//! `pub mod serde_support;`).
//!
//! Every per-class decision — which keys exist, which are mandatory, which
//! `_type` tags route to which variant — lives in the EMITTED impls
//! (`openehr-codegen -- emit-json`), where it is auditable next to the class.
//! This module holds only what cannot be emitted per class:
//!
//! - [`ExpectedType`] — the zero-allocation `_type` check a concrete class runs
//!   when the discriminator is present;
//! - [`MatchTag`] / [`TagMatch`] — the zero-allocation `_type` read a
//!   polymorphic slot dispatches on;
//! - [`SlotKey`] + [`read_slot_tag`] + [`TaggedRest`] — the two-path
//!   polymorphic-object reader: the fast path (the discriminator is the first
//!   member, which is what every canonical writer produces) streams straight
//!   into the chosen variant, and the slow path (JSON objects are unordered —
//!   RFC 8259 §4) buffers the members seen before the discriminator and replays
//!   them ahead of the rest of the stream.
//!
//! NOTE: no openEHR spec governs how a reader is *implemented* — our own
//! design/extension. The canonical WIRE contract it reproduces
//! (`_type` first, BMM declaration order, absent-not-null) is
//! `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
//! §JSON Format.

#![expect(
    clippy::disallowed_types,
    reason = "the canonical-JSON reader runtime buffers wire members as serde_json::Value before \
              `_type` dispatch — the wire value IS the domain here (#1694 boundary class)"
)]

use serde::de::{DeserializeSeed, Deserializer, Error as _, MapAccess, Visitor};

/// The canonical discriminator key.
pub const TYPE_KEY: &str = "_type";

// ── the `_type` check on a concrete class ────────────────────────────────────

/// Reads the `_type` member of a concrete class and checks it names that class.
///
/// Used as `map.next_value_seed(ExpectedType("DV_TEXT"))?`: the value never
/// leaves the visitor, so a matching discriminator costs no allocation.
#[derive(Debug, Clone, Copy)]
pub struct ExpectedType(
    /// The canonical class name the discriminator must equal.
    pub &'static str,
);

impl<'de> DeserializeSeed<'de> for ExpectedType {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for ExpectedType {
    type Value = ();

    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the `_type` discriminator \"{}\"", self.0)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
        if v == self.0 {
            Ok(())
        } else {
            Err(E::custom(format_args!(
                "expected _type \"{}\", found \"{v}\"",
                self.0
            )))
        }
    }
}

// ── the `_type` read on a polymorphic slot ───────────────────────────────────

/// The outcome of reading a polymorphic slot's `_type` against its closed tag
/// set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagMatch {
    /// The discriminator named one of the slot's permitted classes; the
    /// `&'static str` is the emitted tag, so the caller matches on it without
    /// touching the wire buffer.
    Known(&'static str),
    /// The discriminator named something else; the owned string is kept only to
    /// build the refusal message.
    Unknown(String),
}

/// Reads a polymorphic slot's `_type` member and matches it against the slot's
/// closed tag set, allocating only when the tag is unknown (i.e. only on the
/// refusal path).
#[derive(Debug, Clone, Copy)]
pub struct MatchTag(
    /// The permitted canonical class names, in emission order.
    pub &'static [&'static str],
);

impl<'de> DeserializeSeed<'de> for MatchTag {
    type Value = TagMatch;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<TagMatch, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for MatchTag {
    type Value = TagMatch;

    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a `_type` discriminator string")
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<TagMatch, E> {
        Ok(self
            .0
            .iter()
            .find(|t| **t == v)
            .map_or_else(|| TagMatch::Unknown(v.to_owned()), |t| TagMatch::Known(t)))
    }
}

// ── the polymorphic-object key + the two-path tag read ───────────────────────

/// A member key of a polymorphic object: the discriminator, or anything else.
///
/// The `Other` payload is owned because a member seen BEFORE the discriminator
/// has to be buffered and replayed (the slow path); on the fast path — the
/// discriminator first, which is what every canonical writer emits — no `Other`
/// key is ever produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotKey {
    /// The `_type` discriminator.
    Type,
    /// Any other member name.
    Other(String),
}

impl<'de> serde::Deserialize<'de> for SlotKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl Visitor<'_> for KeyVisitor {
            type Value = SlotKey;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("an object member name")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SlotKey, E> {
                Ok(if v == TYPE_KEY {
                    SlotKey::Type
                } else {
                    SlotKey::Other(v.to_owned())
                })
            }
        }
        deserializer.deserialize_identifier(KeyVisitor)
    }
}

/// The discriminator a polymorphic object carries (`None` when it carries
/// none), paired with the members read before it — which the caller replays
/// through [`TaggedRest`].
pub type SlotTag = (Option<TagMatch>, Vec<(String, serde_json::Value)>);

/// Read forward through `map` until the `_type` discriminator appears, matching
/// it against `tags`.
///
/// Returns the discriminator (`None` when the object carries none) together
/// with the members consumed before it, which the caller replays through
/// [`TaggedRest`]. Every canonical writer puts `_type` first, so the buffer is
/// empty on the overwhelming majority of documents; buffering exists because
/// JSON object members are unordered (RFC 8259 §4).
///
/// # Errors
/// Propagates any error the underlying map access raises, including a
/// duplicated `_type` member.
pub fn read_slot_tag<'de, A: MapAccess<'de>>(
    map: &mut A,
    tags: &'static [&'static str],
) -> Result<SlotTag, A::Error> {
    let mut buffered: Vec<(String, serde_json::Value)> = Vec::new();
    while let Some(key) = map.next_key::<SlotKey>()? {
        match key {
            SlotKey::Type => {
                let tag = map.next_value_seed(MatchTag(tags))?;
                return Ok((Some(tag), buffered));
            }
            SlotKey::Other(name) => {
                let value = map.next_value::<serde_json::Value>()?;
                if buffered.iter().any(|(k, _)| *k == name) {
                    return Err(A::Error::custom(format_args!("duplicate field `{name}`")));
                }
                buffered.push((name, value));
            }
        }
    }
    Ok((None, buffered))
}

/// The remainder of a polymorphic object after its `_type` has been read.
///
/// The discriminator itself, the members buffered ahead of it, and the
/// still-unread tail of the stream are presented as one [`Deserializer`] the
/// chosen variant deserializes from.
///
/// The discriminator is replayed rather than swallowed, because a tag may route
/// to an INTERMEDIATE variant that is itself polymorphic and has to dispatch on
/// it again (and because a concrete class verifies its own discriminator).
#[derive(Debug)]
pub struct TaggedRest<A> {
    tag: Option<&'static str>,
    pending_tag: Option<&'static str>,
    buffered: std::vec::IntoIter<(String, serde_json::Value)>,
    pending_value: Option<serde_json::Value>,
    map: A,
}

impl<A> TaggedRest<A> {
    /// Assemble the remainder from the discriminator (when the object carried
    /// one), the members buffered ahead of it, and the unread tail.
    pub fn new(
        tag: Option<&'static str>,
        buffered: Vec<(String, serde_json::Value)>,
        map: A,
    ) -> Self {
        Self {
            tag,
            pending_tag: None,
            buffered: buffered.into_iter(),
            pending_value: None,
            map,
        }
    }
}

/// The replayed `_type` value, as a [`Deserializer`].
///
/// [`serde::de::value::StrDeserializer`] forwards `deserialize_option` to
/// `deserialize_any`, so a target field typed `Option<String>` — which is how
/// a transport DTO declares an OPTIONAL discriminator property, e.g. the
/// ITS-REST `UpdateAudit._type` (`default: UPDATE_AUDIT`, absent from
/// `required`) — sees a bare string where it expects an option and refuses.
/// This wrapper answers `deserialize_option` with `visit_some`, so the
/// discriminator reads into `String`, `Option<String>` and a
/// field-identifier alike; every other form still sees the plain string.
#[derive(Debug)]
struct TagDeserializer<E> {
    tag: &'static str,
    error: std::marker::PhantomData<E>,
}

impl<'de, E: serde::de::Error> Deserializer<'de> for TagDeserializer<E> {
    type Error = E;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_str(self.tag)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for TaggedRest<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if let Some(tag) = self.tag.take() {
            self.pending_tag = Some(tag);
            return seed
                .deserialize(serde::de::value::StrDeserializer::<Self::Error>::new(
                    TYPE_KEY,
                ))
                .map(Some);
        }
        if let Some((key, value)) = self.buffered.next() {
            self.pending_value = Some(value);
            return seed
                .deserialize(serde::de::value::StringDeserializer::<Self::Error>::new(
                    key,
                ))
                .map(Some);
        }
        self.map.next_key_seed(seed)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        if let Some(tag) = self.pending_tag.take() {
            return seed.deserialize(TagDeserializer::<Self::Error> {
                tag,
                error: std::marker::PhantomData,
            });
        }
        if let Some(value) = self.pending_value.take() {
            return seed
                .deserialize(value)
                .map_err(|e: serde_json::Error| Self::Error::custom(e));
        }
        self.map.next_value_seed(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        None
    }
}

impl<'de, A: MapAccess<'de>> Deserializer<'de> for TaggedRest<A> {
    type Error = A::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

// ── the open extension-point carrier ─────────────────────────────────────────

/// A refused [`OpenSubtype`] construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OpenSubtypeError {
    /// The subtype tag was empty — a scheme instance must name its scheme.
    #[error("an open-subtype instance must carry a non-empty `_type` scheme name")]
    EmptyTypeName,
    /// The member set carried its own `_type` key, which belongs to the tag.
    #[error("`_type` is the subtype tag, not a member")]
    TypeAmongMembers,
}

/// A scheme-defined instance at a spec-declared OPEN polymorphic seam.
///
/// Some classes the specs deliberately leave open for downstream schemes —
/// `ACCESS_CONTROL_SETTINGS` is "Intended to support multiple access control
/// schemes. Currently implementation dependent."
/// (`RM/docs/UML/classes/org.openehr.rm.ehr.access_control_settings.adoc`)
/// — so a valid instance may carry a `_type` the published model cannot name.
/// This carrier keeps such an instance verbatim: the declared subtype tag and
/// every member in document order, re-serialized exactly as read.
///
/// Construction is validated, so an invalid carrier cannot exist: the tag is
/// non-empty and never duplicated among the members. The reader is otherwise
/// deliberately open — member names and shapes belong to the scheme, which is
/// exactly what the spec declines to constrain.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenSubtype {
    type_name: String,
    members: serde_json::Map<String, serde_json::Value>,
}

impl OpenSubtype {
    /// Builds a carrier from the subtype tag and its members.
    ///
    /// # Errors
    /// [`OpenSubtypeError`] when the tag is empty or `members` carries a
    /// `_type` key of its own.
    pub fn new(
        type_name: impl Into<String>,
        members: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Self, OpenSubtypeError> {
        let type_name = type_name.into();
        if type_name.is_empty() {
            return Err(OpenSubtypeError::EmptyTypeName);
        }
        if members.contains_key(TYPE_KEY) {
            return Err(OpenSubtypeError::TypeAmongMembers);
        }
        Ok(Self { type_name, members })
    }

    /// The declared subtype tag (the wire `_type`).
    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// The scheme's members, in document order, `_type` excluded.
    #[must_use]
    pub fn members(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.members
    }

    /// The value of member `key`, if present.
    #[must_use]
    pub fn member(&self, key: &str) -> Option<&serde_json::Value> {
        self.members.get(key)
    }
}

impl serde::Serialize for OpenSubtype {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(self.members.len() + 1))?;
        map.serialize_entry(TYPE_KEY, &self.type_name)?;
        for (key, value) in &self.members {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for OpenSubtype {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OpenVisitor;
        impl<'de> Visitor<'de> for OpenVisitor {
            type Value = OpenSubtype;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object carrying a `_type` scheme name")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut type_name: Option<String> = None;
                let mut members = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if key == TYPE_KEY {
                        if type_name.is_some() {
                            return Err(A::Error::duplicate_field(TYPE_KEY));
                        }
                        type_name = Some(map.next_value::<String>()?);
                        continue;
                    }
                    if members.contains_key(&key) {
                        return Err(A::Error::custom(format_args!(
                            "duplicate member `{key}` on an open-subtype instance"
                        )));
                    }
                    members.insert(key, map.next_value::<serde_json::Value>()?);
                }
                let Some(type_name) = type_name else {
                    return Err(A::Error::custom(
                        "an open-subtype instance must carry a `_type` scheme name",
                    ));
                };
                OpenSubtype::new(type_name, members).map_err(A::Error::custom)
            }
        }
        deserializer.deserialize_map(OpenVisitor)
    }
}

// ── the refusal a polymorphic slot raises ────────────────────────────────────

/// The refusal for a polymorphic slot whose `_type` names no permitted class.
#[must_use]
pub fn unexpected_type<E: serde::de::Error>(slot: &str, found: &str, expected: &str) -> E {
    E::custom(format_args!(
        "{slot}: unexpected `_type` {found:?} (expected one of: {expected})"
    ))
}

/// The refusal for a polymorphic slot that carries no `_type` at all and has no
/// concrete self-shape to fall back to.
#[must_use]
pub fn missing_type<E: serde::de::Error>(slot: &str, expected: &str) -> E {
    E::custom(format_args!(
        "{slot}: missing required `_type` on polymorphic slot (expected one of: {expected})"
    ))
}

/// The refusal for a wire key the class does not declare — serde's own
/// `unknown_field` wording, plus the CLASS the key was found on.
///
/// `serde::de::Error::unknown_field` cannot carry the class (its signature is
/// field + expected set), and a refusal that does not say WHICH class rejected
/// the key is materially worse to act on: the same member name is declared by
/// some RM classes and not others, so the reader names it here.
///
/// The strict reader itself is grounded on the released artifacts:
/// `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// L75 (XML — the wildcard-free ITS-XML schemas cannot validate an undeclared
/// element) and L87 (the same paragraph for JSON, at SHOULD strength), plus
/// openEHR's own ITS-JSON schemas closing 128 of 134 object definitions with
/// `additionalProperties: false`.
#[must_use]
pub fn unknown_field<E: serde::de::Error>(
    field: &str,
    class: &'static str,
    expected: &'static [&'static str],
) -> E {
    if expected.is_empty() {
        return E::custom(format_args!(
            "unknown field `{field}` on `{class}`, there are no fields"
        ));
    }
    let mut list = String::new();
    for (i, name) in expected.iter().enumerate() {
        if i > 0 {
            list.push_str(", ");
        }
        list.push('`');
        list.push_str(name);
        list.push('`');
    }
    E::custom(format_args!(
        "unknown field `{field}` on `{class}`, expected one of {list}"
    ))
}
