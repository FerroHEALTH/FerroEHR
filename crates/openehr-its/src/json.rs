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
///
/// The three are kept as separate DATA rather than one flattened sentence, so
/// the protocol edge can render them into a structured error body instead of
/// re-parsing prose.
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
    ///
    /// The wording is byte-identical to what the EMITTED reader raises for the
    /// same key (`openehr_base::serde_support::unknown_field`): the two doors
    /// are two implementations of one refusal, and the fast-vs-typed
    /// equivalence battery compares their messages directly.
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
    serde_json::to_string(value).expect("an openEHR spec value should always serialize")
}

/// Serialize an RM value to a canonical-JSON `serde_json::Value` (for the
/// boundary where the caller needs an in-memory tree).
///
/// The workspace pins `serde_json/preserve_order`, so the resulting object
/// keeps the canonical member order (`_type` first, then BMM declaration
/// order).
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
    serde_json::to_value(value).expect("an openEHR spec value should always serialize")
}

/// Deserialize an RM value from canonical JSON (`&str`).
///
/// The reader is STRICT: an undeclared wire key, a repeated key, an absent
/// mandatory attribute, a present-but-wrong `_type` and a malformed identifier
/// are all refusals. A missing `_type` on a concretely-typed slot is tolerated
/// (per ITS-JSON), members may arrive in any order, and trailing content after
/// the value is rejected.
///
/// # Errors
/// Returns a [`JsonParseError`] carrying the JSON path to the offending node.
pub fn from_canonical_json<T: DeserializeOwned>(json: &str) -> Result<T, JsonParseError> {
    // The happy path reads WITHOUT the path tracker: `serde_path_to_error`
    // wraps every key and value seed, and on this corpus that costs ~60% of
    // total read time. A failed read re-runs the same deterministic decode WITH
    // the tracker to build the diagnostic — so the path is never lost, and it
    // is paid for only when there is something to report.
    match serde_json::from_str::<T>(json) {
        Ok(value) => Ok(value),
        Err(plain) => Err(located_in_str::<T>(json, &plain)),
    }
}

/// Re-read `json` with the path tracker so a refusal names the offending node.
fn located_in_str<T: DeserializeOwned>(json: &str, plain: &serde_json::Error) -> JsonParseError {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    match serde_path_to_error::deserialize::<_, T>(&mut deserializer) {
        Err(located) => from_path_error(&located),
        // A deterministic re-read of the same bytes cannot succeed where the
        // first read failed; if it ever does, the original diagnostic still
        // describes the failure — just without its path.
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
/// The protocol adapter parses a canonical-JSON body into an untyped
/// `serde_json::Value` (so a committed document keeps the client's bytes
/// verbatim) and therefore never runs a typed decode at the door. This walk
/// gives that door the reader's undeclared-key refusal WITHOUT a decode, using
/// the generated `declared_fields` table — the same field view the typed reader
/// refuses from, so the two can never disagree.
///
/// Only `_type`-tagged nodes are judged: canonical JSON requires `_type` only
/// where the declared attribute type is abstract, so an untagged node's class
/// is not knowable from the value alone here. An undeclared key on such a node
/// is still refused, one tier later, by the typed dispatch
/// (`crate::wire_validate`) — this door narrows *which status* a refusal
/// carries, never *whether* one happens.
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

/// Validate a canonical-JSON value against the vendored ITS-JSON RM schema
/// (`openehr_rm_1.1.0_all.json`).
///
/// The schema's root dispatches on the top-level `_type` (draft-07
/// `if`/`then`) to the matching class definition, so any RM object with a
/// `_type` is validated against its own definition.
///
/// # Errors
/// Returns every schema violation (path + message), or a single-element error if
/// the schema itself failed to compile.
#[cfg(feature = "schema-validation")]
pub fn validate_canonical(value: &serde_json::Value) -> Result<(), Vec<String>> {
    match &*RM_VALIDATOR {
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
