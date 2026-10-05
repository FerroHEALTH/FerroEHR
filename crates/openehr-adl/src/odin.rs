// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ODIN bridge — one home for reading an `openehr_lang::v1_1::odin` value tree
//! into AOM/BASE types, plus the `master03` lexical decoding both ODIN and cADL
//! share.
//!
//! ODIN is a LANG-component specification
//! (`docs/specs/openehr/LANG/docs/odin/`), parsed by `openehr_lang::v1_1::odin`; this
//! module is the *reading* layer over that tree: type-cast peeling
//! (`untyped`), scalar/list/map extraction, term-code and UUID conversion, and
//! the canonical-JSON encoding a `C_DEFINED_OBJECT.default_value` needs.
//!
//! Two lexical concerns sit here too, because they are shared by the ODIN
//! sections and the cADL definition rather than owned by either: the
//! string/character literal decoding of `ADL2/master03-file_encoding.adoc`
//! §File Encoding + §Special Character Sequences
//! (`decode_string`/`decode_character` — delimiter stripping over
//! [`openehr_lang::v1_1::escape`], which owns the escape
//! semantics for ODIN, BEL and cADL alike) and the delimited-regex handling of
//! `AOM2/master04.5` §Class Definitions (`C_STRING`).
//!
//! NOTE: a delimited regex (`/…/`, `^…^`) NEVER passes through the escape
//! decoder — `ADL2/master03-file_encoding.adoc` §Special Character Sequences,
//! final paragraph: backslash patterns in a regular expression "should not be
//! treated as anything other than literal strings, since they are processed by
//! a regular expression parser". Only the optional `;"assumed"` suffix beside a
//! regex is decoded.

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use std::collections::BTreeMap;

use openehr_base::prelude::{TerminologyCode, Uuid};
use openehr_base::v1_3::base_types::definitions::definitions_impl::LOCAL_TERMINOLOGY_ID;
use openehr_lang::v1_1::odin::{OdinInterval, OdinKey, OdinValue};

// ── ODIN tree reading ─────────────────────────────────────────────────────

/// Peel any `(TYPE)` casts off an ODIN value.
///
/// The `(TYPE)` prefix of `LANG/docs/odin/master05-content` §Adding Type
/// Information is dynamic-binding information for the reader ("if the type of
/// an object is not inferrable from the data, it must be indicated in an ODIN
/// document"), never part of the datum. Every accessor below therefore reads
/// straight through it: a section written
/// `details = (Hash<RESOURCE_DESCRIPTION_ITEM,String>) <…>` assembles exactly
/// like the uncast form, instead of silently yielding nothing. The cast itself
/// stays on the tree in [`OdinValue::Typed`] for any caller that wants it.
pub(crate) fn untyped(v: &OdinValue) -> &OdinValue {
    let mut cur = v;
    while let OdinValue::Typed { value, .. } = cur {
        cur = value;
    }
    cur
}

/// The object map an ODIN value carries, reading through any type cast.
pub(crate) fn as_object(v: &OdinValue) -> Option<&indexmap::IndexMap<String, OdinValue>> {
    match untyped(v) {
        OdinValue::Object(m) => Some(m),
        _ => None,
    }
}

/// The keyed-list entries an ODIN value carries, reading through any type cast.
pub(crate) fn as_keyed(v: &OdinValue) -> Option<&[(OdinKey, OdinValue)]> {
    match untyped(v) {
        OdinValue::KeyedList(items) => Some(items),
        _ => None,
    }
}

/// An ODIN keyed-list key as a plain [`String`].
pub(crate) fn key_str(k: &OdinKey) -> String {
    match k {
        OdinKey::String(s) | OdinKey::Date(s) | OdinKey::Time(s) | OdinKey::DateTime(s) => {
            s.clone()
        }
        OdinKey::Integer(i) => i.to_string(),
    }
}

/// A one-word name for an ODIN value's kind, for defect messages.
pub(crate) fn odin_kind(v: &OdinValue) -> &'static str {
    match v {
        OdinValue::Object(_) => "an object",
        OdinValue::KeyedList(_) => "a keyed list",
        OdinValue::List(_) => "a list",
        OdinValue::Typed { .. } => "a typed block",
        OdinValue::PathList(_) => "a path list",
        OdinValue::Empty => "an empty block",
        _ => "a leaf value",
    }
}

/// The scalar string an ODIN leaf carries (or a single-element list's leaf).
pub(crate) fn string_of(v: Option<&OdinValue>) -> Option<String> {
    match untyped(v?) {
        OdinValue::String(s)
        | OdinValue::Date(s)
        | OdinValue::Time(s)
        | OdinValue::DateTime(s)
        | OdinValue::Duration(s)
        | OdinValue::Path(s) => Some(s.clone()),
        OdinValue::Integer(i) => Some(i.to_string()),
        OdinValue::Real(r) => Some(r.to_string()),
        OdinValue::Boolean(b) => Some(b.to_string()),
        OdinValue::Character(c) => Some(c.to_string()),
        OdinValue::Uri(u) => Some(strip_uri_delims(u)),
        OdinValue::List(items) if items.len() == 1 => string_of(items.first()),
        _ => None,
    }
}

/// A list of strings from an ODIN `List` (or a single scalar as a one-element
/// list), dropping the trailing open-list marker.
pub(crate) fn string_list(v: &OdinValue) -> Vec<String> {
    match untyped(v) {
        OdinValue::List(items) => items
            .iter()
            .filter(|x| !matches!(x, OdinValue::ListContinue))
            .filter_map(|x| string_of(Some(x)))
            .collect(),
        other => string_of(Some(other)).into_iter().collect(),
    }
}

/// A `key → String` map from an ODIN keyed list or object of string leaves.
pub(crate) fn string_map(v: &OdinValue) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    match untyped(v) {
        OdinValue::KeyedList(items) => {
            for (k, val) in items {
                if let Some(s) = string_of(Some(val)) {
                    out.insert(key_str(k), s);
                }
            }
        }
        OdinValue::Object(map) => {
            for (k, val) in map {
                if let Some(s) = string_of(Some(val)) {
                    out.insert(k.clone(), s);
                }
            }
        }
        _ => {}
    }
    out
}

/// The `key → String` map at `field` of `obj` (empty if absent).
pub(crate) fn string_map_of(
    obj: &indexmap::IndexMap<String, OdinValue>,
    field: &str,
) -> BTreeMap<String, String> {
    obj.get(field).map(string_map).unwrap_or_default()
}

/// Every string leaf of an object as `key → String` (used to gather an
/// `ARCHETYPE_TERM`'s `other_items` before removing `text`/`description`).
pub(crate) fn term_other_items(
    obj: &indexmap::IndexMap<String, OdinValue>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if let Some(s) = string_of(Some(v)) {
            out.insert(k.clone(), s);
        }
    }
    out
}

/// Unwrap the deprecated `items = <…>` wrapper around a term/binding block
/// (`master07.13` §Deprecated Terminology Section Features); returns the inner
/// value, or `v` unchanged if there is no wrapper.
pub(crate) fn unwrap_items(v: &OdinValue) -> &OdinValue {
    let v = untyped(v);
    if let OdinValue::Object(map) = v
        && map.len() == 1
        && let Some(inner) = map.get("items")
    {
        return untyped(inner);
    }
    v
}

/// The URI string a binding value carries, stripped of ODIN `<>` delimiters.
pub(crate) fn uri_string(v: &OdinValue) -> String {
    match untyped(v) {
        OdinValue::Uri(u) => strip_uri_delims(u),
        OdinValue::TermCode(t) => t.clone(),
        OdinValue::PathList(ps) => ps.first().cloned().unwrap_or_default(),
        other => string_of(Some(other)).unwrap_or_default(),
    }
}

/// Strip the ODIN `<…>` URI delimiters.
pub(crate) fn strip_uri_delims(u: &str) -> String {
    u.trim_start_matches('<').trim_end_matches('>').to_owned()
}

/// A `TerminologyCode` from an ODIN term-code leaf (`[ISO_639-1::en]`).
pub(crate) fn term_code_of(v: &OdinValue) -> TerminologyCode {
    match untyped(v) {
        OdinValue::TermCode(code) => parse_term_code(code),
        other => string_of(Some(other)).map_or_else(
            || term_code(LOCAL_TERMINOLOGY_ID, ""),
            |s| parse_term_code(&s),
        ),
    }
}

/// Parse `[terminology::code]` (or `[terminology(version)::code]`) into a
/// [`TerminologyCode`].
pub(crate) fn parse_term_code(raw: &str) -> TerminologyCode {
    let inner = raw.trim().trim_start_matches('[').trim_end_matches(']');
    let (terminology_part, code_string) = match inner.split_once("::") {
        Some((t, c)) => (t.to_owned(), c.to_owned()),
        None => (LOCAL_TERMINOLOGY_ID.to_owned(), inner.to_owned()),
    };
    // Optional `(version)` suffix on the terminology id.
    let (terminology_id, terminology_version) = match terminology_part.split_once('(') {
        Some((id, ver)) => (id.to_owned(), Some(ver.trim_end_matches(')').to_owned())),
        None => (terminology_part, None),
    };
    TerminologyCode {
        terminology_id,
        terminology_version,
        code_string,
        uri: None,
    }
}

/// A [`TerminologyCode`] naming `terminology_id` and `code`.
pub(crate) fn term_code(terminology_id: &str, code: &str) -> TerminologyCode {
    TerminologyCode {
        terminology_id: terminology_id.to_owned(),
        terminology_version: None,
        code_string: code.to_owned(),
        uri: None,
    }
}

/// Parse a textual UUID, or `None` if it is not one.
pub(crate) fn parse_uuid(s: &str) -> Option<Uuid> {
    uuid::Uuid::parse_str(s.trim()).ok().map(Uuid::new)
}

/// The nil UUID (`00000000-0000-0000-0000-000000000000`).
pub(crate) const fn nil_uuid() -> Uuid {
    Uuid::new(uuid::Uuid::nil())
}

/// Whether an ODIN value is an interval, reading through any type cast.
///
/// An interval carries its own canonical-JSON `_type`
/// (`Point_interval`/`Proper_interval`, see [`odin_to_json`]), so the ODIN cast
/// that heads it — which names the generic slot type, `Interval<Quantity>` and
/// the like — must not overwrite that tag.
pub(crate) fn is_interval(v: &OdinValue) -> bool {
    matches!(untyped(v), OdinValue::Interval(_))
}

/// Why an ODIN value carries no canonical-JSON encoding as a
/// `C_DEFINED_OBJECT.default_value`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OdinJsonError {
    /// A `|centre +/- delta|` interval over endpoints that are not numeric.
    ///
    /// The form's meaning IS arithmetic on its endpoints —
    /// `LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered Primitive
    /// Types glosses `|5.0 +/-0.5|` as `4.5 ±5.5`, and
    /// `AM/docs/ADL1.4/master04-dadl.adoc` §Intervals of Ordered Primitive
    /// Types as "4.5 - 5.5" — but a `Date`/`Time`/`Date_time`/`Duration`
    /// endpoint is carried verbatim on the ODIN tree, so reducing
    /// `centre ± delta` to bounds would need calendar arithmetic over a typing
    /// the tree does not record. Refused instead of guessed.
    #[error(
        "a '|centre +/- delta|' interval over {centre}/{delta} endpoints has no lower/upper reduction: only Integer and Real endpoints reduce without type context"
    )]
    PlusMinusNotNumeric {
        /// The ODIN type of the centre endpoint.
        centre: &'static str,
        /// The ODIN type of the delta endpoint.
        delta: &'static str,
    },
    /// A numeric `|centre +/- delta|` interval whose bounds leave the
    /// representable range (an `Integer` sum that overflows, an `Integer`
    /// magnitude beyond exact `Real` representation, or a non-finite `Real`
    /// bound). Refused rather than silently rounded.
    #[error(
        "a '|centre +/- delta|' interval whose bounds leave the representable numeric range has no lower/upper reduction"
    )]
    PlusMinusOutOfRange,
    /// A plug-in-syntax block (`(syntax) <# … #>`,
    /// `LANG/docs/odin/master09-plug_in_syntaxes`) as a default value. Its
    /// body is raw foreign text for a plug-in parser — it denotes no RM
    /// instance, so it has no canonical-JSON encoding as a
    /// `C_DEFINED_OBJECT.default_value`. Refused rather than smuggled through
    /// as a string.
    #[error(
        "a plug-in-syntax block (({syntax}) <# … #>) is foreign text, not an RM instance; it has no default_value encoding"
    )]
    PlugInBlock {
        /// The plug-in syntax tag.
        syntax: String,
    },
}

/// Convert an [`OdinValue`] to canonical JSON for a
/// `C_DEFINED_OBJECT.default_value`.
///
/// NOTE: `AOM2/master04` types `C_DEFINED_OBJECT.default_value` as an instance
/// of the constrained RM type, and no openEHR spec mandates an intermediate
/// JSON shape for it — the canonical-JSON encoding used here is our own
/// design/extension. An `<>` / `<...>` empty block is a genuine "no value" and
/// maps to `null`.
///
/// An ODIN interval (`|0..5|`) encodes as the object shape the canonical
/// codec emits for a BASE `Interval<T>` — `_type` first, then the present
/// bounds, then the four boundary flags.
///
/// NOTE: `_type` is `Point_interval` exactly when the interval denotes a
/// single value (both sides bounded, included, and equal) — the same
/// predicate [`crate::aom::interval::point_value_i32`] adjudicates for the
/// constraint model.
///
/// # Errors
/// [`OdinJsonError`] for an interval form the bound representation cannot
/// carry faithfully.
pub(crate) fn odin_to_json(v: &OdinValue) -> Result<serde_json::Value, OdinJsonError> {
    let json = match v {
        OdinValue::String(s)
        | OdinValue::Date(s)
        | OdinValue::Time(s)
        | OdinValue::DateTime(s)
        | OdinValue::Duration(s)
        | OdinValue::TermCode(s)
        | OdinValue::Uri(s)
        | OdinValue::Path(s) => serde_json::Value::String(s.clone()),
        OdinValue::Integer(i) => serde_json::Value::from(*i),
        OdinValue::Real(r) => serde_json::Value::from(*r),
        OdinValue::Boolean(b) => serde_json::Value::from(*b),
        OdinValue::Character(c) => serde_json::Value::String(c.to_string()),
        OdinValue::Empty => serde_json::Value::Null,
        OdinValue::PlugIn { syntax, .. } => {
            return Err(OdinJsonError::PlugInBlock {
                syntax: syntax.clone(),
            });
        }
        OdinValue::Interval(iv) => interval_to_json(iv)?,
        OdinValue::ListContinue => serde_json::Value::String("...".to_owned()),
        OdinValue::List(items) => serde_json::Value::Array(
            items
                .iter()
                .map(odin_to_json)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        OdinValue::PathList(ps) => serde_json::Value::Array(
            ps.iter()
                .map(|p| serde_json::Value::String(p.clone()))
                .collect(),
        ),
        OdinValue::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), odin_to_json(val)?);
            }
            serde_json::Value::Object(out)
        }
        OdinValue::KeyedList(items) => {
            let mut out = serde_json::Map::new();
            for (k, val) in items {
                out.insert(key_str(k), odin_to_json(val)?);
            }
            serde_json::Value::Object(out)
        }
        OdinValue::Typed { rm_type, value } => {
            let mut inner = odin_to_json(value)?;
            if !is_interval(value)
                && let serde_json::Value::Object(m) = &mut inner
            {
                m.insert(
                    "_type".to_owned(),
                    serde_json::Value::String(class_name(rm_type).to_owned()),
                );
            }
            inner
        }
    };
    Ok(json)
}

/// The canonical-JSON object an ODIN interval denotes.
fn interval_to_json(iv: &OdinInterval) -> Result<serde_json::Value, OdinJsonError> {
    let bounds = match iv {
        OdinInterval::Range {
            lower,
            lower_included,
            upper,
            upper_included,
        } => IntervalBounds {
            lower: lower.as_deref().map(odin_to_json).transpose()?,
            upper: upper.as_deref().map(odin_to_json).transpose()?,
            lower_included: *lower_included,
            upper_included: *upper_included,
        },
        OdinInterval::PlusMinus { centre, delta } => plus_minus_bounds(centre, delta)?,
    };
    Ok(bounds.into_json())
}

/// The four boundary values of an interval, ready for canonical JSON.
struct IntervalBounds {
    /// The lower bound, `None` when the interval is unbounded below.
    lower: Option<serde_json::Value>,
    /// The upper bound, `None` when the interval is unbounded above.
    upper: Option<serde_json::Value>,
    /// Whether the lower bound is inclusive.
    lower_included: bool,
    /// Whether the upper bound is inclusive.
    upper_included: bool,
}

impl IntervalBounds {
    /// A closed interval over two computed bounds.
    fn closed(lower: serde_json::Value, upper: serde_json::Value) -> Self {
        Self {
            lower: Some(lower),
            upper: Some(upper),
            lower_included: true,
            upper_included: true,
        }
    }

    /// The canonical-JSON object, in the emitted codec's field order.
    fn into_json(self) -> serde_json::Value {
        let lower_unbounded = self.lower.is_none();
        let upper_unbounded = self.upper.is_none();
        let is_point = !lower_unbounded
            && !upper_unbounded
            && self.lower_included
            && self.upper_included
            && self.lower == self.upper;
        let mut m = serde_json::Map::new();
        m.insert(
            "_type".to_owned(),
            serde_json::Value::String(
                if is_point {
                    "Point_interval"
                } else {
                    "Proper_interval"
                }
                .to_owned(),
            ),
        );
        if let Some(lower) = self.lower {
            m.insert("lower".to_owned(), lower);
        }
        if let Some(upper) = self.upper {
            m.insert("upper".to_owned(), upper);
        }
        m.insert(
            "lower_unbounded".to_owned(),
            serde_json::Value::from(lower_unbounded),
        );
        m.insert(
            "upper_unbounded".to_owned(),
            serde_json::Value::from(upper_unbounded),
        );
        m.insert(
            "lower_included".to_owned(),
            serde_json::Value::from(self.lower_included),
        );
        m.insert(
            "upper_included".to_owned(),
            serde_json::Value::from(self.upper_included),
        );
        serde_json::Value::Object(m)
    }
}

/// Reduce `|centre +/- delta|` to its two bounds.
fn plus_minus_bounds(
    centre: &OdinValue,
    delta: &OdinValue,
) -> Result<IntervalBounds, OdinJsonError> {
    let (centre, delta) = (untyped(centre), untyped(delta));
    if !is_numeric_leaf(centre) || !is_numeric_leaf(delta) {
        return Err(OdinJsonError::PlusMinusNotNumeric {
            centre: leaf_type_name(centre),
            delta: leaf_type_name(delta),
        });
    }
    if let (OdinValue::Integer(c), OdinValue::Integer(d)) = (centre, delta) {
        let lower = c
            .checked_sub(*d)
            .ok_or(OdinJsonError::PlusMinusOutOfRange)?;
        let upper = c
            .checked_add(*d)
            .ok_or(OdinJsonError::PlusMinusOutOfRange)?;
        return Ok(IntervalBounds::closed(
            serde_json::Value::from(lower),
            serde_json::Value::from(upper),
        ));
    }
    let (Some(c), Some(d)) = (exact_f64(centre), exact_f64(delta)) else {
        return Err(OdinJsonError::PlusMinusOutOfRange);
    };
    let (lower, upper) = (c - d, c + d);
    if !lower.is_finite() || !upper.is_finite() {
        return Err(OdinJsonError::PlusMinusOutOfRange);
    }
    Ok(IntervalBounds::closed(
        serde_json::Value::from(lower),
        serde_json::Value::from(upper),
    ))
}

/// Whether an ODIN leaf is one of the two numeric primitive types.
fn is_numeric_leaf(v: &OdinValue) -> bool {
    matches!(v, OdinValue::Integer(_) | OdinValue::Real(_))
}

/// The `f64` an ODIN numeric leaf denotes, or `None` when the value is not
/// numeric or not exactly representable as one.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "the 2^53 magnitude guard proves the i64 → f64 conversion is exact; a wider magnitude returns None and is refused"
)]
fn exact_f64(v: &OdinValue) -> Option<f64> {
    match v {
        OdinValue::Real(r) => Some(*r),
        OdinValue::Integer(i) if i.unsigned_abs() <= (1u64 << 53) => Some(*i as f64),
        _ => None,
    }
}

/// The ODIN type name of a leaf value, for defect messages.
fn leaf_type_name(v: &OdinValue) -> &'static str {
    match v {
        OdinValue::Integer(_) => "Integer",
        OdinValue::Real(_) => "Real",
        OdinValue::Date(_) => "Date",
        OdinValue::Time(_) => "Time",
        OdinValue::DateTime(_) => "Date_time",
        OdinValue::Duration(_) => "Duration",
        OdinValue::String(_) => "String",
        OdinValue::Boolean(_) => "Boolean",
        OdinValue::Character(_) => "Character",
        OdinValue::TermCode(_) => "TERM_CODE_REF",
        OdinValue::Uri(_) => "URI",
        _ => "non-primitive",
    }
}

/// The class name a (possibly namespaced) ODIN type cast denotes.
///
/// `LANG/docs/odin/master05-content` §Adding Type Information (verbatim in
/// `AM/docs/ADL1.4/master04-dadl` §Adding Type Information) builds a qualified
/// type identifier "by prepending package names, separated by the '.'
/// character" to the type name, so the dotted prefix says which package the
/// class comes from and the class itself is the terminal segment:
/// `org.openehr.rm.ehr.content.ENTRY` and a bare `ENTRY` name the same type.
/// The canonical-JSON `_type` tag carries the class name, so the package path
/// is dropped on the way into JSON; the cast keeps its fully-qualified
/// spelling, as authored, on the ODIN tree.
///
/// A generic head is qualified independently of its parameters
/// (`org.openehr.base.Interval<Quantity>`), so only the text before the first
/// `<` is unqualified.
fn class_name(rm_type: &str) -> &str {
    let head_end = rm_type.find('<').unwrap_or(rm_type.len());
    let Some(head) = rm_type.get(..head_end) else {
        return rm_type;
    };
    match head.rfind('.') {
        Some(dot) => rm_type.get(dot + 1..).unwrap_or(rm_type),
        None => rm_type,
    }
}

// ── `master03` lexical decoding ───────────────────────────────────────────

/// Decode a double-quoted `master03` string literal (delimiters included).
///
/// The escape semantics themselves live in
/// [`openehr_lang::v1_1::escape`] — one home for ODIN, BEL and
/// cADL, since `ADL2/master03-file_encoding.adoc` §File Encoding + §Special
/// Character Sequences and their verbatim ODIN twin
/// (`LANG/docs/odin/master03-basics.adoc`) define one escape set.
///
/// # Errors
/// [`openehr_lang::v1_1::escape::EscapeError`] for a `\u` escape that denotes no
/// character. The cADL lexer's own escape check is STRUCTURAL only (4 or 8 hex
/// digits), so this is where such a defect is caught, with the offending
/// literal's span.
pub(crate) fn decode_string(raw: &str) -> Result<String, openehr_lang::v1_1::escape::EscapeError> {
    openehr_lang::v1_1::escape::decode_string_literal(raw)
}

/// Decode a single-quoted `CHARACTER` literal (delimiters included) into the
/// one-character string that carries it (`base_lexer.g4` `CHARACTER`).
///
/// # Errors
/// As [`decode_string`]. The lexer admits only the six quoted forms inside a
/// character literal, so no `\u` escape reaches here in practice.
pub(crate) fn decode_character(
    raw: &str,
) -> Result<String, openehr_lang::v1_1::escape::EscapeError> {
    openehr_lang::v1_1::escape::decode_character_literal(raw)
}

// ── delimited regex (`AOM2/master04.5` §`C_STRING`) ───────────────────────

/// The inner regex of a `/re/` or `^re^` delimited pattern.
pub(crate) fn regex_inner(delimited: &str) -> &str {
    let d = delimited.trim();
    for delimiter in ['/', '^'] {
        if let Some(inner) = d
            .strip_prefix(delimiter)
            .and_then(|rest| rest.strip_suffix(delimiter))
        {
            return inner;
        }
    }
    d
}

/// Backslash-escape every UNESCAPED `/` in a regex body.
///
/// `AOM2/master04.5` §`C_STRING` types `constraint` as "a list of literal
/// strings and / or regular expression strings **delimited by the '/'
/// character**", so the AOM carrier is always the `/…/` form — which makes the
/// `^…^` delimiter a purely lexical alternative that has to be normalised on
/// the way in. The chapter states the two forms' equivalence with its own
/// worked pair (`ADL1.4/master05-cadl.adoc` §Regular Expression L696-702:
/// "If the delimiter character is required in the pattern, it must be quoted
/// with the backslash ('\\') character, or else alternative delimiters can be
/// used … The following two patterns are equivalent: `{/km\\/h|mi\\/h/}` …
/// `{^km/h|mi/h^}`"), so escaping on normalisation is the spec's own mapping
/// and keeps parse → print → parse lossless. An already-escaped `\/` (a
/// slash-delimited source) is left alone, so the transform is idempotent.
pub(crate) fn escape_regex_delimiter(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                out.push(ch);
            }
            '/' => out.push_str("\\/"),
            _ => out.push(ch),
        }
    }
    out
}

/// The single delimited regex a `C_STRING` constraint carries, if any.
pub(crate) fn regex_of(constraint: &[String]) -> Option<&str> {
    match constraint {
        [one] if is_delimited_regex(one) => Some(one),
        _ => None,
    }
}

/// True if a `C_STRING` constraint entry is a single delimited regex (`/re/` or
/// `^re^`) — the `cadl2.g4` `CONTAINED_REGEXP` form the parser stores verbatim.
///
/// A plain string *value* that merely starts with `/` (e.g. a unit `"/min"`) is
/// not a regex — the delimiter must close, so both ends are required. Compared
/// VERBATIM: a leading or trailing space defeats the test (the printer's
/// round-trip depends on that strictness). The whitespace-tolerant sibling is
/// [`is_delimited_regex_trimmed`].
pub(crate) fn is_delimited_regex(s: &str) -> bool {
    (s.len() >= 2 && s.starts_with('/') && s.ends_with('/'))
        || (s.len() >= 2 && s.starts_with('^') && s.ends_with('^'))
}

/// True if a `C_STRING` constraint entry is a delimited regex (`/re/` or `^re^`)
/// rather than a literal string (`master04.5` §`C_STRING`), IGNORING surrounding
/// whitespace.
///
/// The whitespace-strict sibling is [`is_delimited_regex`].
pub(crate) fn is_delimited_regex_trimmed(s: &str) -> bool {
    let t = s.trim();
    t.len() >= 2
        && ((t.starts_with('/') && t.ends_with('/')) || (t.starts_with('^') && t.ends_with('^')))
}
#[cfg(test)]
mod tests {
    use super::{OdinJsonError, class_name, odin_to_json};

    /// A namespaced ODIN cast names the same class as its bare spelling
    /// (`LANG/docs/odin/master05-content` §Adding Type Information), so the
    /// package path is dropped on the way into the canonical-JSON `_type`
    /// tag while a generic head is unqualified independently of its
    /// parameters.
    #[test]
    fn a_namespaced_cast_reduces_to_its_class_name() {
        assert_eq!(class_name("ENTRY"), "ENTRY");
        assert_eq!(class_name("org.openehr.rm.ehr.content.ENTRY"), "ENTRY");
        assert_eq!(
            class_name("Core.Abstractions.Relationships.Relationship"),
            "Relationship"
        );
        assert_eq!(class_name("Interval<Quantity>"), "Interval<Quantity>");
        assert_eq!(
            class_name("org.openehr.base.Interval<org.openehr.base.Quantity>"),
            "Interval<org.openehr.base.Quantity>"
        );
    }

    /// The `_default` value block a namespaced cast heads carries the class
    /// name as its `_type`, so a qualified spelling in the source does not
    /// produce a dotted tag no RM reader recognises.
    #[test]
    fn a_namespaced_cast_tags_a_default_value_with_the_class_name() {
        let value = openehr_lang::v1_1::odin::parse(
            "a = (org.openehr.rm.data_types.text.DV_TEXT) <value = <\"x\">>",
        )
        .expect("the namespaced cast should parse");
        let json = odin_to_json(&value).expect("the cast block should encode");
        assert_eq!(json["a"]["_type"], serde_json::json!("DV_TEXT"));
        assert_eq!(json["a"]["value"], serde_json::json!("x"));
    }

    /// The canonical JSON of the interval `src` denotes, read as the sole
    /// attribute `a` of an ODIN text.
    fn interval_json(src: &str) -> serde_json::Value {
        let value = openehr_lang::v1_1::odin::parse(&format!("a = <{src}>"))
            .unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let json = odin_to_json(&value).unwrap_or_else(|e| panic!("{src} should encode: {e}"));
        json["a"].clone()
    }

    /// The `OdinJsonError` the interval `src` raises.
    fn interval_error(src: &str) -> OdinJsonError {
        let value = openehr_lang::v1_1::odin::parse(&format!("a = <{src}>"))
            .unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        odin_to_json(&value).expect_err("the interval should be refused")
    }

    /// Every two-sided interval form of
    /// `LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered
    /// Primitive Types encodes as the canonical `Proper_interval` object, with
    /// the relational operators carried by the two `*_included` flags.
    #[test]
    fn two_sided_interval_forms_encode_their_inclusivity() {
        let cases: &[(&str, bool, bool)] = &[
            ("|0..5|", true, true),
            ("|>0..5|", false, true),
            ("|0..<5|", true, false),
            ("|>0..<5|", false, false),
        ];
        for (src, lower_included, upper_included) in cases {
            assert_eq!(
                interval_json(src),
                serde_json::json!({
                    "_type": "Proper_interval",
                    "lower": 0,
                    "upper": 5,
                    "lower_unbounded": false,
                    "upper_unbounded": false,
                    "lower_included": lower_included,
                    "upper_included": upper_included,
                }),
                "{src}"
            );
        }
    }

    /// A one-sided form (`|>=N|`, `|<N|`) and the `infinity`/`*` endpoint
    /// markers leave the open side unbounded, with no bound member and the
    /// `*_included` flag false — the `Lower_included_valid`/
    /// `Upper_included_valid` invariants of
    /// `BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`.
    #[test]
    fn one_sided_and_unbounded_interval_forms_omit_the_open_bound() {
        assert_eq!(
            interval_json("|>=1939-02-01|"),
            serde_json::json!({
                "_type": "Proper_interval",
                "lower": "1939-02-01",
                "lower_unbounded": false,
                "upper_unbounded": true,
                "lower_included": true,
                "upper_included": false,
            })
        );
        assert_eq!(
            interval_json("|<10.5|"),
            serde_json::json!({
                "_type": "Proper_interval",
                "upper": 10.5,
                "lower_unbounded": true,
                "upper_unbounded": false,
                "lower_included": false,
                "upper_included": false,
            })
        );
        assert_eq!(
            interval_json("|0..infinity|"),
            serde_json::json!({
                "_type": "Proper_interval",
                "lower": 0,
                "lower_unbounded": false,
                "upper_unbounded": true,
                "lower_included": true,
                "upper_included": false,
            })
        );
        assert_eq!(
            interval_json("|*..*|"),
            serde_json::json!({
                "_type": "Proper_interval",
                "lower_unbounded": true,
                "upper_unbounded": true,
                "lower_included": false,
                "upper_included": false,
            })
        );
    }

    /// A degenerate closed interval denotes a single value, so it carries the
    /// `Point_interval` tag — the predicate
    /// [`crate::aom::interval::point_value_i32`] adjudicates for the
    /// constraint model (both sides bounded, both bounds included, both bounds
    /// equal).
    #[test]
    fn a_degenerate_closed_interval_is_a_point_interval() {
        let point = serde_json::json!({
            "_type": "Point_interval",
            "lower": 5,
            "upper": 5,
            "lower_unbounded": false,
            "upper_unbounded": false,
            "lower_included": true,
            "upper_included": true,
        });
        assert_eq!(interval_json("|5|"), point);
        assert_eq!(interval_json("|5..5|"), point);
        // An excluded bound is not a single value, degenerate or not.
        assert_eq!(
            interval_json("|>5..5|")["_type"],
            serde_json::json!("Proper_interval")
        );
    }

    /// The `|N +/- M|` form reduces to its two bounds, which is the meaning
    /// both chapters give it (`LANG/docs/odin/master07-leaf_data.adoc`
    /// §Intervals of Ordered Primitive Types: `|5.0 +/-0.5|` is `4.5 ±5.5`;
    /// `AM/docs/ADL1.4/master04-dadl.adoc` writes the same example "4.5 -
    /// 5.5").
    #[test]
    fn a_plus_minus_interval_reduces_to_its_numeric_bounds() {
        assert_eq!(
            interval_json("|5.0 +/-0.5|"),
            serde_json::json!({
                "_type": "Proper_interval",
                "lower": 4.5,
                "upper": 5.5,
                "lower_unbounded": false,
                "upper_unbounded": false,
                "lower_included": true,
                "upper_included": true,
            })
        );
        assert_eq!(interval_json("|5 +/-2|")["lower"], serde_json::json!(3));
        assert_eq!(interval_json("|5 +/-2|")["upper"], serde_json::json!(7));
    }

    /// A `|N +/- M|` interval over temporal endpoints has no reduction without
    /// type context, so it is a typed refusal naming the limitation, never a
    /// guessed bound pair.
    #[test]
    fn a_non_numeric_plus_minus_interval_is_refused() {
        assert_eq!(
            interval_error("|2020-01-01 +/-P1D|"),
            OdinJsonError::PlusMinusNotNumeric {
                centre: "Date",
                delta: "Duration",
            }
        );
    }

    /// A `|N +/- M|` interval whose bounds overflow the `Integer` range is
    /// refused rather than wrapped.
    #[test]
    fn an_overflowing_plus_minus_interval_is_refused() {
        assert_eq!(
            interval_error(&format!("|{} +/-1|", i64::MAX)),
            OdinJsonError::PlusMinusOutOfRange
        );
    }
}
