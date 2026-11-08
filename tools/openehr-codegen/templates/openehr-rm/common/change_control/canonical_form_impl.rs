// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! The RFC 8785 JSON Canonicalization Scheme over a JSON value, the
//! serialisation `VERSION.canonical_form()` signs.
//!
//! RFC 8785 §3.2 fixes the form: no whitespace (§3.2.1), strings with only the
//! escapes ECMAScript `JSON.stringify` writes (§3.2.2.2), numbers in the
//! ECMAScript Number-to-String form (§3.2.2.3, written by `ryu-js`), and object
//! members sorted by the UTF-16 code units of their raw names (§3.2.3). The
//! writer runs once over the value into one `String`, sorting each object's
//! members by reference with no per-name allocation, so a large version costs
//! one pass (#5220). The `serde_jcs` crate is its test oracle.

#![expect(
    clippy::disallowed_types,
    reason = "the canonical form is computed over the assembled canonical-JSON node the versioning \
              service signs (#1694 boundary class)"
)]

use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

/// Returns the RFC 8785 canonical form of `value`.
///
/// # Errors
/// A [`serde_json::Error`] when a number has no finite IEEE 754 double form,
/// which RFC 8785 §3.2.2.3 requires every JSON number to have.
pub fn canonical_json(value: &Value) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// Returns the RFC 8785 canonical form of the object whose members are
/// `members`, in any order.
///
/// A caller joins a member by reference this way (a version's `data`) or
/// leaves one out (its `signature`) without copying the object.
///
/// # Errors
/// As [`canonical_json`].
pub fn canonical_object<'a>(
    members: impl IntoIterator<Item = (&'a str, &'a Value)>,
) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    write_members(members.into_iter().collect(), &mut out)?;
    Ok(out)
}

/// Writes the canonical form of `value` to `out`.
fn write_value(value: &Value, out: &mut String) -> Result<(), serde_json::Error> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => write_number(number, out)?,
        Value::String(text) => write_string(text, out)?,
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => write_map(map, out)?,
    }
    Ok(())
}

/// Writes the canonical form of the object `map` to `out`.
fn write_map(map: &Map<String, Value>, out: &mut String) -> Result<(), serde_json::Error> {
    write_members(
        map.iter().map(|(name, value)| (name.as_str(), value)).collect(),
        out,
    )
}

/// Writes the object of `members` to `out`, sorted by the UTF-16 code units
/// of each name (RFC 8785 §3.2.3).
fn write_members(
    mut members: Vec<(&str, &Value)>,
    out: &mut String,
) -> Result<(), serde_json::Error> {
    members.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    out.push('{');
    for (index, (name, value)) in members.into_iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_string(name, out)?;
        out.push(':');
        write_value(value, out)?;
    }
    out.push('}');
    Ok(())
}

/// Writes `number` to `out` as the ECMAScript Number-to-String form of its
/// IEEE 754 double (RFC 8785 §3.2.2.3), every integer included.
fn write_number(number: &Number, out: &mut String) -> Result<(), serde_json::Error> {
    match number.as_f64().filter(|double| double.is_finite()) {
        Some(double) => {
            out.push_str(ryu_js::Buffer::new().format_finite(double));
            Ok(())
        }
        None => Err(serde::ser::Error::custom(
            "a JSON number has no finite IEEE 754 double form (RFC 8785 §3.2.2.3)",
        )),
    }
}

/// Whether RFC 8785 §3.2.2.2 writes `c` as an escape.
fn escaped(c: char) -> bool {
    c == '"' || c == '\\' || c < ' '
}

/// Writes `text` to `out` as a JSON string (RFC 8785 §3.2.2.2).
fn write_string(text: &str, out: &mut String) -> Result<(), serde_json::Error> {
    out.reserve(text.len() + 2);
    out.push('"');
    for run in text.split_inclusive(escaped) {
        let mut chars = run.chars();
        match chars.next_back() {
            Some(last) if escaped(last) => {
                out.push_str(chars.as_str());
                write_escape(last, out)?;
            }
            _ => out.push_str(run),
        }
    }
    out.push('"');
    Ok(())
}

/// Writes the escape of the character `c`, which [`escaped`] holds.
fn write_escape(c: char, out: &mut String) -> Result<(), serde_json::Error> {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\u{8}' => out.push_str("\\b"),
        '\t' => out.push_str("\\t"),
        '\n' => out.push_str("\\n"),
        '\u{c}' => out.push_str("\\f"),
        '\r' => out.push_str("\\r"),
        control => {
            return write!(out, "\\u{:04x}", u32::from(control))
                .map_err(serde::ser::Error::custom);
        }
    }
    Ok(())
}
