// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! **ITS-JSON** — canonical JSON serialization: the named entry points.
//!
//! The canonical conventions (`_type` first on every object, snake_case keys,
//! omitted nulls/empties, integer-vs-real number typing, `{_type, value}` UIDs,
//! inline-base64 `DV_MULTIMEDIA.data`) are carried by the EMITTED manual
//! `serde::Serialize`/`serde::Deserialize` impls on the spec types themselves
//! (`openehr-codegen -- emit-json`, one `json_serde` module per spec crate over
//! the `openehr_base::serde_support` runtime) — never a serde derive, whose
//! four enum representations cannot express the canonical `_type` discriminator
//! (<https://serde.rs/enum-representations.html>). These functions are the
//! single named entry points so call sites read as canonical-JSON rather than
//! raw `serde_json`, and so every read is wrapped once in
//! `serde_path_to_error`.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The vendored ITS-JSON RM all-schema (draft-07), used to validate output in
/// the fidelity-gate tests, vendored at a pinned upstream commit (see the
/// bundled schema's provenance record).
#[cfg(feature = "schema-validation")]
pub const RM_SCHEMA_JSON: &str = include_str!("../schemas/json/openehr_rm_1.1.0_all.json");

/// A canonical-JSON deserialization failure: the message, the JSON path to the
/// offending node, and (for a syntax error) the source line/column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonParseError {
    message: String,
    /// `Some((line, column))`, 1-based, for a failure `serde_json` located in
    /// the source text.
    location: Option<(usize, usize)>,
    /// JSON path segments from the root to the failing node, outermost first
    /// (e.g. `.content`, `[0]`, `.data`). Empty for a root-level failure.
    path: Vec<String>,
}

impl JsonParseError {
    /// A failure with a message and no location (the path is filled in as it
    /// propagates up through [`Self::in_field`] / [`Self::in_index`]).
    #[must_use]
    pub fn custom(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: None,
            path: Vec::new(),
        }
    }

    /// An undeclared wire key on a concrete type, as raised by the pre-decode
    /// key walk ([`reject_undeclared_keys`]) and by the validation dispatcher.
    #[must_use]
    pub fn unknown_field(field: &str, ty: &str, known: &[&str]) -> Self {
        if known.is_empty() {
            return Self::custom(format!(
                "unknown field `{field}` on `{ty}`, there are no fields"
            ));
        }
        let list = known
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        Self::custom(format!(
            "unknown field `{field}` on `{ty}`, expected one of {list}"
        ))
    }

    /// Prepend a `.field` path segment (called as the failure propagates out of
    /// a named struct field).
    #[must_use]
    pub fn in_field(mut self, field: &str) -> Self {
        self.path.insert(0, format!(".{field}"));
        self
    }

    /// Prepend an `[index]` path segment (called as the failure propagates out
    /// of an array element).
    #[must_use]
    pub fn in_index(mut self, index: usize) -> Self {
        self.path.insert(0, format!("[{index}]"));
        self
    }

    /// Whether the document is nested deeper than the RM readers' one limit
    /// ([`openehr_base::nesting::MAX_RM_NESTING`]): a document no reader can
    /// read, whatever else it carries.
    #[must_use]
    pub fn is_too_deep(&self) -> bool {
        self.message
            .starts_with(&openehr_base::nesting::NestingTooDeep.to_string())
    }

    /// The failure message, without the path or location decoration.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The JSON path segments from the root to the offending node, outermost
    /// first.
    #[must_use]
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// The 1-based `(line, column)` of the failure in the source text, when
    /// `serde_json` located one.
    #[must_use]
    pub fn location(&self) -> Option<(usize, usize)> {
        self.location
    }
}

/// Build the canonical error from a `serde_path_to_error` failure: the path
/// segments come from the tracker, the line/column from `serde_json`.
fn from_path_error(error: &serde_path_to_error::Error<serde_json::Error>) -> JsonParseError {
    let inner = error.inner();
    let path: Vec<String> = error
        .path()
        .iter()
        .map(|segment| match segment {
            serde_path_to_error::Segment::Seq { index } => format!("[{index}]"),
            serde_path_to_error::Segment::Map { key } => format!(".{key}"),
            serde_path_to_error::Segment::Enum { variant } => format!(".{variant}"),
            serde_path_to_error::Segment::Unknown => ".?".to_owned(),
        })
        .collect();
    // `serde_json` reports (0, 0) for a failure it could not locate in the
    // source (every error raised from a `Value` deserializer, for instance).
    let location = match (inner.line(), inner.column()) {
        (0, _) => None,
        (line, column) => Some((line, column)),
    };
    JsonParseError {
        message: inner.to_string(),
        location,
        path,
    }
}

impl std::fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `serde_json`'s own message already ends with `at line N column M`
        // where it located the failure, so the parsed-out location is kept as
        // queryable DATA ([`Self::location`]) rather than re-rendered here.
        write!(f, "{}", self.message)?;
        if !self.path.is_empty() {
            write!(f, " (at ${})", self.path.join(""))?;
        }
        Ok(())
    }
}

impl std::error::Error for JsonParseError {}

/// Serialize an RM value to canonical JSON (compact).
///
/// Serialization is infallible for the spec types, so this returns a `String`
/// directly (see the `expect` reason below).
///
/// # Panics
/// Never for a spec type: the only `Err` `serde_json` can return here is an
/// I/O error (the writer is an in-memory `String`), a non-string map key, or a
/// `Serialize` impl that returns `Err` — none of which the emitted spec-type
/// impls can produce.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "`serde_json::to_string` can only fail on an I/O error (the writer is an in-memory String), a non-string map key, or a `Serialize` impl that returns Err; the emitted spec-type impls key every map with `String` and never return Err, so the Err variant is unreachable here — the Book ch9 sanctioned escape for a logically-impossible Err"
)]
pub fn to_canonical_json<T: Serialize + ?Sized>(value: &T) -> String {
    crate::conversion::record::<T>("to_canonical_json");
    serde_json::to_string(value).expect("an openEHR spec value should always serialize")
}

/// Serialize an RM value as canonical JSON (compact), appended to `buffer`.
///
/// # Panics
/// Never for a spec type, for the reason [`to_canonical_json`] gives: the
/// writer is an in-memory buffer.
#[expect(
    clippy::expect_used,
    reason = "`serde_json::to_writer` into a `Vec<u8>` can only fail on a non-string map key or a `Serialize` impl that returns Err; the emitted spec-type impls do neither, and writing to a Vec cannot fail, so the Err variant is unreachable here — the Book ch9 sanctioned escape for a logically-impossible Err"
)]
pub fn write_canonical_json<T: Serialize + ?Sized>(value: &T, buffer: &mut Vec<u8>) {
    crate::conversion::record::<T>("to_canonical_json");
    serde_json::to_writer(buffer, value).expect("an openEHR spec value should always serialize");
}

/// Serialize an RM value as canonical JSON (compact) into any writer.
///
/// # Errors
/// The writer's I/O error, as `serde_json` reports it.
pub fn write_canonical_json_to<W: std::io::Write, T: Serialize + ?Sized>(
    writer: W,
    value: &T,
) -> Result<(), serde_json::Error> {
    crate::conversion::record::<T>("to_canonical_json");
    serde_json::to_writer(writer, value)
}

/// Serialize an RM value to a canonical-JSON `serde_json::Value` (for the
/// boundary where the caller needs an in-memory tree).
///
/// # Panics
/// Never for a spec type: `serde_json::to_value` fails only on a non-string map
/// key or a `Serialize` impl that returns `Err`, and the emitted spec-type
/// impls do neither.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "`serde_json::to_value` fails only on a non-string map key or a `Serialize` impl that returns Err; the emitted spec-type impls do neither, so the Err variant is unreachable here — the Book ch9 sanctioned escape for a logically-impossible Err"
)]
pub fn to_canonical_value<T: Serialize + ?Sized>(value: &T) -> serde_json::Value {
    crate::conversion::record::<T>("to_canonical_value");
    serde_json::to_value(value).expect("an openEHR spec value should always serialize")
}

/// Deserialize an RM value from canonical JSON (`&str`).
///
/// # Errors
/// Returns a [`JsonParseError`] carrying the JSON path to the offending node.
pub fn from_canonical_json<T: DeserializeOwned>(json: &str) -> Result<T, JsonParseError> {
    crate::conversion::record::<T>("from_canonical_json");
    // The happy path skips the costly path tracker; a failed read re-runs the
    // deterministic decode with it to build the diagnostic.
    serde_json::from_str::<T>(json)
        .map_err(|plain| located::<_, T>(&mut serde_json::Deserializer::from_str(json), &plain))
}

/// Deserialize an RM value from canonical JSON bytes, checking their UTF-8
/// as it reads them.
///
/// # Errors
/// As [`from_canonical_json`].
pub fn from_canonical_slice<T: DeserializeOwned>(json: &[u8]) -> Result<T, JsonParseError> {
    crate::conversion::record::<T>("from_canonical_json");
    serde_json::from_slice::<T>(json)
        .map_err(|plain| located::<_, T>(&mut serde_json::Deserializer::from_slice(json), &plain))
}

/// Re-read with the path tracker so a refusal names the offending node.
fn located<'de, R: serde_json::de::Read<'de>, T: DeserializeOwned>(
    deserializer: &mut serde_json::Deserializer<R>,
    plain: &serde_json::Error,
) -> JsonParseError {
    match serde_path_to_error::deserialize::<_, T>(deserializer) {
        Err(located) => from_path_error(&located),
        // A deterministic re-read of the same bytes cannot succeed where the
        // first read failed; if it ever does, the original diagnostic still
        // describes the failure, without its path.
        Ok(_) => JsonParseError::custom(plain.to_string()),
    }
}

/// Deserialize an RM value from an already-parsed canonical-JSON
/// `serde_json::Value` (no re-stringifying — `&Value` is itself a
/// `Deserializer`). Same strictness as [`from_canonical_json`].
///
/// # Errors
/// Returns a [`JsonParseError`] carrying the JSON path to the offending node.
pub fn from_canonical_value<T: DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<T, JsonParseError> {
    crate::conversion::record::<T>("from_canonical_value");
    // Same two-phase shape as [`from_canonical_json`]: the untracked read on
    // the happy path, the tracked re-read only to describe a failure.
    match T::deserialize(value) {
        Ok(decoded) => Ok(decoded),
        Err(plain) => Err(match serde_path_to_error::deserialize::<_, T>(value) {
            Err(located) => from_path_error(&located),
            Ok(_) => JsonParseError::custom(plain.to_string()),
        }),
    }
}

/// The compiled ITS-JSON validator, built once from [`RM_SCHEMA_JSON`]. Stored
/// as a `Result` (not `expect`ed) so a schema-compile failure surfaces as a
/// validation error rather than a panic in library code.
///
/// NOTE: the two causes stay flattened into the stored `String` rather than
/// carried as a source (RFC 0201) — the schema is a compiled-in constant, so a
/// failure here is a packaging fault with no caller that could branch on it.
#[cfg(feature = "schema-validation")]
static RM_VALIDATOR: std::sync::LazyLock<Result<jsonschema::Validator, String>> =
    std::sync::LazyLock::new(|| {
        let schema: serde_json::Value =
            serde_json::from_str(RM_SCHEMA_JSON).map_err(|e| format!("parse RM schema: {e}"))?;
        jsonschema::validator_for(&schema).map_err(|e| format!("compile RM schema: {e}"))
    });

/// Refuse the first `_type`-tagged node of `value` that carries a wire key its
/// class does not declare — the **wire door's** half of the strict reader.
///
/// # Why the door and not validation
/// A document the reader cannot READ never converts, so it cannot reach the
/// convertible-but-semantically-invalid branch: ITS-REST overview
/// `Requests_and_responses.md` §HTTP status codes defines `400` as content that
/// "could not be parsed or is invalid" and `422` as content that is
/// "well-formed but was unable to be followed due to semantic errors".
///
/// # Errors
/// A [`JsonParseError`] naming the offending key, the class that does not
/// declare it, and the path to the node.
pub fn reject_undeclared_keys(value: &serde_json::Value) -> Result<(), JsonParseError> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(ty) = map.get("_type").and_then(serde_json::Value::as_str)
                && let Some(declared) =
                    crate::json_codec::generated::structural::declared_fields(ty)
                && let Some(key) = map
                    .keys()
                    .find(|k| k.as_str() != "_type" && declared.binary_search(&k.as_str()).is_err())
            {
                return Err(JsonParseError::unknown_field(key, ty, declared).in_field(key));
            }
            for (key, child) in map {
                reject_undeclared_keys(child).map_err(|e| e.in_field(key))?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_undeclared_keys(child).map_err(|e| e.in_index(index))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// What one pass over canonical-JSON text finds in its member names, with no
/// value built ([`scan_keys`]).
#[derive(Debug)]
pub struct KeyFindings {
    /// Whether a `_type`-tagged node carries a wire key its class does not
    /// declare: the verdict of [`reject_undeclared_keys`] on the parsed text.
    pub undeclared: bool,
    /// The first object that writes a member name twice, as the refusal the
    /// strict reader gives a repeated key, with the path to the object.
    pub repeated: Option<JsonParseError>,
}

/// Reads the member names of canonical-JSON text in one pass that builds no
/// value.
///
/// A commit door that hands the text on to the typed reader asks this first.
/// It refuses a repeated member name itself: an object that writes a name
/// twice is ambiguous (RFC 8259 §4: the names "SHOULD be unique", and the
/// behaviour of software that receives one that is not "is unpredictable"),
/// the canonical form signatures are computed over has none (RFC 8785 §3.1
/// builds on I-JSON, RFC 7493 §2.3: "Objects in I-JSON messages MUST NOT have
/// members with duplicate names"), and a parsed value would hold the last
/// occurrence and hide the first. For an undeclared key it takes the
/// value-building path of [`reject_undeclared_keys`], which names the key. A
/// repeated `_type` is read as its last occurrence for that verdict.
///
/// # Errors
/// The `serde_json` error of text that is not one JSON value.
pub fn scan_keys(json: &str) -> Result<KeyFindings, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let mut keys = Vec::new();
    let found = serde::de::DeserializeSeed::deserialize(KeyScan(&mut keys), &mut deserializer)?;
    deserializer.end()?;
    Ok(KeyFindings {
        undeclared: found.undeclared,
        repeated: found.repeated.map(|repeated| {
            let refusal = JsonParseError::custom(format!("duplicate field `{}`", repeated.name));
            repeated
                .path
                .iter()
                .fold(refusal, |refusal, step| match step {
                    Step::Member(name) => refusal.in_field(name),
                    Step::Item(index) => refusal.in_index(*index),
                })
        }),
    })
}

/// One step of the path to a value, from the value outward.
enum Step {
    Member(String),
    Item(usize),
}

/// The first repeated member name under a value, and the path from the
/// object that holds it out to that value.
struct Repeated {
    name: String,
    path: Vec<Step>,
}

/// What [`KeyScan`] found under one value.
#[derive(Default)]
struct Found {
    undeclared: bool,
    repeated: Option<Repeated>,
}

impl Found {
    /// Takes in what a child at `step` found.
    fn take(&mut self, child: Found, step: impl FnOnce() -> Step) {
        self.undeclared |= child.undeclared;
        if self.repeated.is_none()
            && let Some(mut repeated) = child.repeated
        {
            repeated.path.push(step());
            self.repeated = Some(repeated);
        }
    }
}

/// How many members an object may hold before its names are sorted to find a
/// repeated one, where fewer are compared pairwise.
const SORTED_FROM: usize = 24;

/// The first name `names` holds twice.
fn repeated_name<'a>(names: &'a [std::borrow::Cow<'_, str>]) -> Option<&'a str> {
    if names.len() < SORTED_FROM {
        return names.iter().enumerate().find_map(|(at, name)| {
            names
                .get(..at)
                .is_some_and(|before| before.contains(name))
                .then_some(name.as_ref())
        });
    }
    let mut sorted: Vec<&str> = names.iter().map(AsRef::as_ref).collect();
    sorted.sort_unstable();
    sorted
        .windows(2)
        .find_map(|pair| (pair.first() == pair.get(1)).then(|| pair.first().copied()))
        .flatten()
}

/// The pass of [`scan_keys`] over one JSON value. The names of the objects
/// being read share one buffer, each object owning its tail.
struct KeyScan<'a, 'de>(&'a mut Vec<std::borrow::Cow<'de, str>>);

impl<'de> serde::de::DeserializeSeed<'de> for KeyScan<'_, 'de> {
    type Value = Found;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Found, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for KeyScan<'_, 'de> {
    type Value = Found;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<Found, E> {
        Ok(Found::default())
    }

    fn visit_i64<E>(self, _: i64) -> Result<Found, E> {
        Ok(Found::default())
    }

    fn visit_u64<E>(self, _: u64) -> Result<Found, E> {
        Ok(Found::default())
    }

    fn visit_f64<E>(self, _: f64) -> Result<Found, E> {
        Ok(Found::default())
    }

    fn visit_str<E>(self, _: &str) -> Result<Found, E> {
        Ok(Found::default())
    }

    fn visit_unit<E>(self) -> Result<Found, E> {
        Ok(Found::default())
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut items: A) -> Result<Found, A::Error> {
        let mut found = Found::default();
        let mut index = 0_usize;
        while let Some(item) = items.next_element_seed(KeyScan(&mut *self.0))? {
            found.take(item, || Step::Item(index));
            index = index.saturating_add(1);
        }
        Ok(found)
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut members: A) -> Result<Found, A::Error> {
        let keys = self.0;
        let own = keys.len();
        let mut class: Option<std::borrow::Cow<'de, str>> = None;
        let mut found = Found::default();
        while let Some(key) = members.next_key_seed(KeyText)? {
            let child = if key == "_type" {
                match members.next_value_seed(TypeTag(&mut *keys))? {
                    Tag::Name(name) => {
                        class = Some(name);
                        Found::default()
                    }
                    Tag::Other(inner) => {
                        class = None;
                        inner
                    }
                }
            } else {
                members.next_value_seed(KeyScan(&mut *keys))?
            };
            found.take(child, || Step::Member(key.as_ref().to_owned()));
            keys.push(key);
        }
        let names = keys.get(own..).unwrap_or_default();
        if let Some(declared) = class
            .as_deref()
            .and_then(crate::json_codec::generated::structural::declared_fields)
        {
            found.undeclared |= names
                .iter()
                .any(|key| key != "_type" && declared.binary_search(&key.as_ref()).is_err());
        }
        // The object's own repeat comes first: its members were read before
        // it ended, and a refusal names the outermost defect.
        if let Some(name) = repeated_name(names) {
            found.repeated = Some(Repeated {
                name: name.to_owned(),
                path: Vec::new(),
            });
        }
        keys.truncate(own);
        Ok(found)
    }
}

/// An object key, borrowed from the text where it holds no escape.
struct KeyText;

impl<'de> serde::de::DeserializeSeed<'de> for KeyText {
    type Value = std::borrow::Cow<'de, str>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de> serde::de::Visitor<'de> for KeyText {
    type Value = std::borrow::Cow<'de, str>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an object key")
    }

    fn visit_borrowed_str<E>(self, key: &'de str) -> Result<Self::Value, E> {
        Ok(std::borrow::Cow::Borrowed(key))
    }

    fn visit_str<E>(self, key: &str) -> Result<Self::Value, E> {
        Ok(std::borrow::Cow::Owned(key.to_owned()))
    }
}

/// What a `_type` member holds: the class name, or any other value scanned.
enum Tag<'de> {
    Name(std::borrow::Cow<'de, str>),
    Other(Found),
}

/// The value of a `_type` member.
struct TypeTag<'a, 'de>(&'a mut Vec<std::borrow::Cow<'de, str>>);

impl<'de> serde::de::DeserializeSeed<'de> for TypeTag<'_, 'de> {
    type Value = Tag<'de>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for TypeTag<'_, 'de> {
    type Value = Tag<'de>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(Tag::Other(Found::default()))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(Tag::Other(Found::default()))
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(Tag::Other(Found::default()))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(Tag::Other(Found::default()))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Tag::Other(Found::default()))
    }

    fn visit_borrowed_str<E>(self, name: &'de str) -> Result<Self::Value, E> {
        Ok(Tag::Name(std::borrow::Cow::Borrowed(name)))
    }

    fn visit_str<E>(self, name: &str) -> Result<Self::Value, E> {
        Ok(Tag::Name(std::borrow::Cow::Owned(name.to_owned())))
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, items: A) -> Result<Self::Value, A::Error> {
        KeyScan(self.0).visit_seq(items).map(Tag::Other)
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, members: A) -> Result<Self::Value, A::Error> {
        KeyScan(self.0).visit_map(members).map(Tag::Other)
    }
}

/// The ITS-JSON validator whose root dispatches on the top-level `_type` to
/// EVERY class definition of [`RM_SCHEMA_JSON`], the published root listing
/// only some of them.
#[cfg(feature = "schema-validation")]
static RM_CLASS_VALIDATOR: std::sync::LazyLock<Result<jsonschema::Validator, String>> =
    std::sync::LazyLock::new(|| {
        let mut schema: serde_json::Value =
            serde_json::from_str(RM_SCHEMA_JSON).map_err(|e| format!("parse RM schema: {e}"))?;
        let names: Vec<String> = schema
            .get("definitions")
            .and_then(serde_json::Value::as_object)
            .map(|definitions| definitions.keys().cloned().collect())
            .ok_or_else(|| "the RM schema has no definitions".to_owned())?;
        // The schema keyword that names a class, never a canonical shape's own key.
        let discriminator = "_type";
        let mut root = vec![serde_json::json!({
            "required": [discriminator],
            "properties": { (discriminator): { "enum": names } }
        })];
        root.extend(names.iter().map(|class| {
            serde_json::json!({
                "if": { "properties": { (discriminator): { "const": class } } },
                "then": { "$ref": format!("#/definitions/{class}") }
            })
        }));
        if let Some(map) = schema.as_object_mut() {
            map.insert("allOf".to_owned(), serde_json::Value::Array(root));
        }
        jsonschema::validator_for(&schema).map_err(|e| format!("compile RM schema: {e}"))
    });

/// Which root of the vendored ITS-JSON RM schema (`openehr_rm_1.1.0_all.json`)
/// judges a value.
#[cfg(feature = "schema-validation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaRoot {
    /// The published root: it dispatches on the top-level `_type` (draft-07
    /// `if`/`then`) to the matching definition for the classes it lists, and a
    /// value of any other class passes it unjudged.
    Published,
    /// A root that dispatches on the top-level `_type` to every class
    /// definition the schema holds.
    EveryClass,
}

/// Validate a canonical-JSON value against the vendored ITS-JSON RM schema,
/// under the given `root`.
///
/// # Errors
/// Returns every schema violation (path + message), or a single-element error if
/// the schema itself failed to compile.
#[cfg(feature = "schema-validation")]
pub fn validate_canonical(value: &serde_json::Value, root: SchemaRoot) -> Result<(), Vec<String>> {
    let validator = match root {
        SchemaRoot::Published => &RM_VALIDATOR,
        SchemaRoot::EveryClass => &RM_CLASS_VALIDATOR,
    };
    collect_errors(validator, value)
}

#[cfg(feature = "schema-validation")]
fn collect_errors(
    validator: &Result<jsonschema::Validator, String>,
    value: &serde_json::Value,
) -> Result<(), Vec<String>> {
    match validator {
        Ok(validator) => {
            let errors: Vec<String> = validator
                .iter_errors(value)
                .map(|e| format!("{} (at {})", e, e.instance_path()))
                .collect();
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors)
            }
        }
        Err(e) => Err(vec![e.clone()]),
    }
}
