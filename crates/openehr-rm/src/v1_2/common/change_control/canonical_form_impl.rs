// @generated-from-template templates/openehr-rm/common/change_control/canonical_form_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
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
//! one pass (#5220). Every object of a run shares one member vector, a
//! string is scanned by byte and each run with nothing to escape is copied
//! whole, an integer within 2^53 is written as its digits, and two names whose
//! first difference is ASCII are ordered by their bytes (#5682). The `serde_json_canonicalizer` crate is its test oracle.

#![expect(
    clippy::disallowed_types,
    reason = "the canonical form is computed over the assembled canonical-JSON node the versioning \
              service signs (#1694 boundary class)"
)]

use std::cmp::Ordering;

use serde_json::{Number, Value};

/// Returns the RFC 8785 canonical form of `value`.
///
/// # Errors
/// A [`serde_json::Error`] when a number has no finite IEEE 754 double form,
/// which RFC 8785 §3.2.2.3 requires every JSON number to have.
pub fn canonical_json(value: &Value) -> Result<String, serde_json::Error> {
    let mut writer = Writer::default();
    writer.value(value)?;
    Ok(writer.out)
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
    let mut writer = Writer::default();
    writer.members.extend(members);
    writer.object(0)?;
    Ok(writer.out)
}

/// One run of the writer over a value whose strings and values outlive `'a`.
#[derive(Debug, Default)]
struct Writer<'a> {
    /// The canonical form written so far.
    out: String,
    /// The members of every object being written, outermost first: each
    /// object sorts and walks its own tail and leaves the vector as it found
    /// it, so a whole document shares one allocation.
    members: Vec<(&'a str, &'a Value)>,
}

impl<'a> Writer<'a> {
    /// Writes the canonical form of `value`.
    fn value(&mut self, value: &'a Value) -> Result<(), serde_json::Error> {
        match value {
            Value::Null => self.out.push_str("null"),
            Value::Bool(true) => self.out.push_str("true"),
            Value::Bool(false) => self.out.push_str("false"),
            Value::Number(number) => write_number(number, &mut self.out)?,
            Value::String(text) => write_string(text, &mut self.out),
            Value::Array(items) => {
                self.out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        self.out.push(',');
                    }
                    self.value(item)?;
                }
                self.out.push(']');
            }
            Value::Object(map) => {
                let start = self.members.len();
                self.members
                    .extend(map.iter().map(|(name, value)| (name.as_str(), value)));
                self.object(start)?;
            }
        }
        Ok(())
    }

    /// Writes the object whose members are those from `start` on, sorted by
    /// the UTF-16 code units of each name (RFC 8785 §3.2.3), and removes them.
    fn object(&mut self, start: usize) -> Result<(), serde_json::Error> {
        if let Some(members) = self.members.get_mut(start..) {
            members.sort_by(|(a, _), (b, _)| utf16_order(a, b));
        }
        let end = self.members.len();
        self.out.push('{');
        for index in start..end {
            if let Some(&(name, value)) = self.members.get(index) {
                if index > start {
                    self.out.push(',');
                }
                write_string(name, &mut self.out);
                self.out.push(':');
                self.value(value)?;
            }
        }
        self.out.push('}');
        self.members.truncate(start);
        Ok(())
    }
}

/// Orders `a` and `b` by their UTF-16 code units (RFC 8785 §3.2.3).
///
/// UTF-8 byte order is code point order, which UTF-16 order departs from only
/// between a supplementary character and one in U+E000 to U+FFFF. When one
/// name is a prefix of the other, or their first differing bytes are both
/// ASCII, the byte order is the UTF-16 order; any other pair is compared by
/// its code units.
pub(super) fn utf16_order(a: &str, b: &str) -> Ordering {
    match a.bytes().zip(b.bytes()).find(|(x, y)| x != y) {
        None => a.len().cmp(&b.len()),
        Some((x, y)) if x.is_ascii() && y.is_ascii() => x.cmp(&y),
        Some(_) => a.encode_utf16().cmp(b.encode_utf16()),
    }
}

/// 2^53, the magnitude up to which every integer is an exact IEEE 754 double.
const EXACT_INTEGER: u64 = 1 << 53;

/// Writes `number` to `out` as the ECMAScript Number-to-String form of its
/// IEEE 754 double (RFC 8785 §3.2.2.3), every integer included.
pub(super) fn write_number(number: &Number, out: &mut String) -> Result<(), serde_json::Error> {
    // NOTE: ECMA-262 Number::toString writes an integral double below 10^21 as
    // its decimal digits, so an integer within 2^53 is written as its own.
    if let Some(magnitude) = number.as_u64().filter(|n| *n <= EXACT_INTEGER) {
        write_digits(magnitude, out);
        return Ok(());
    }
    if let Some(negative) = number
        .as_i64()
        .filter(|n| n.unsigned_abs() <= EXACT_INTEGER)
    {
        out.push('-');
        write_digits(negative.unsigned_abs(), out);
        return Ok(());
    }
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

/// Writes the decimal digits of `value` to `out`, with no leading zero.
#[expect(
    clippy::expect_used,
    reason = "a remainder of a division by ten is below ten, so it fits a byte"
)]
fn write_digits(mut value: u64, out: &mut String) {
    let mut digits = [b'0'; 20];
    let mut count = 0;
    for slot in digits.iter_mut().rev() {
        let digit = u8::try_from(value % 10).expect("a decimal digit should fit a byte");
        *slot = b'0' + digit;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let skip = digits.len() - count;
    out.extend(digits.iter().skip(skip).map(|digit| char::from(*digit)));
}

/// Whether RFC 8785 §3.2.2.2 writes the byte `byte` as an escape.
///
/// Every escaped character is ASCII, and no byte of a multi-byte UTF-8
/// sequence is, so a byte scan finds each escape at a character boundary.
const fn escaped(byte: u8) -> bool {
    byte == b'"' || byte == b'\\' || byte < b' '
}

/// Writes `text` to `out` as a JSON string (RFC 8785 §3.2.2.2), copying each
/// run with nothing to escape whole.
pub(super) fn write_string(text: &str, out: &mut String) {
    out.reserve(text.len() + 2);
    out.push('"');
    let mut rest = text;
    while let Some(at) = rest.bytes().position(escaped) {
        let (run, byte, tail) = split_escape(rest, at);
        out.push_str(run);
        write_escape(byte, out);
        rest = tail;
    }
    out.push_str(rest);
    out.push('"');
}

/// Splits `text` around the escaped byte at `at`: the run before it, the
/// byte, and the text after it.
#[expect(
    clippy::expect_used,
    reason = "`at` indexes an ASCII byte of `text` (`escaped` holds only ASCII), so `at` and \
              `at + 1` are character boundaries within `text`"
)]
fn split_escape(text: &str, at: usize) -> (&str, u8, &str) {
    let (run, from) = text
        .split_at_checked(at)
        .expect("an escaped byte should start a character");
    let byte = *from
        .as_bytes()
        .first()
        .expect("an escaped byte should be in the text");
    let tail = from
        .get(1..)
        .expect("an escaped byte should be one whole character");
    (run, byte, tail)
}

/// The lower-case hexadecimal digits of a `\u` escape, by value.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Writes the escape of `byte`, which [`escaped`] holds.
fn write_escape(byte: u8, out: &mut String) {
    match byte {
        b'"' => out.push_str("\\\""),
        b'\\' => out.push_str("\\\\"),
        0x08 => out.push_str("\\b"),
        b'\t' => out.push_str("\\t"),
        b'\n' => out.push_str("\\n"),
        0x0c => out.push_str("\\f"),
        b'\r' => out.push_str("\\r"),
        control => {
            out.push_str("\\u00");
            for nibble in [control >> 4, control & 0x0f] {
                if let Some(digit) = HEX.get(usize::from(nibble)) {
                    out.push(char::from(*digit));
                }
            }
        }
    }
}
