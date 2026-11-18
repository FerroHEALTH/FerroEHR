// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One `RESULT_SET` cell, rendered once as JSON text, and the renderings of
//! the typed SQL values a cell is read from.
//!
//! A cell is written exactly as PostgreSQL's `to_jsonb` of the same value
//! read back through `serde_json` used to serve it: a text as a JSON string,
//! a number as the `serde_json` number of its decimal text, a `jsonb` column
//! with its keys in jsonb order, and a `timestamptz` in the XSD form
//! `to_jsonb` writes in the session's time zone. No openEHR spec governs the
//! rendering mechanics: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result cells are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use std::fmt::Write as _;

use serde_json::Value;

use super::fragment::{write_jsonb, write_string};
use super::ir::TypedLit;

/// One rendered `RESULT_SET` cell: valid JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell(Box<str>);

impl Cell {
    /// The JSON `null` cell.
    #[must_use]
    pub fn null() -> Self {
        Self("null".into())
    }

    /// The cell of `value`, written in its own key order.
    #[must_use]
    pub fn of_value(value: &Value) -> Self {
        // NOTE: serde_json::to_string of a Value cannot fail, so the null is unreachable.
        Self(
            serde_json::to_string(value)
                .unwrap_or_else(|_| "null".to_owned())
                .into(),
        )
    }

    /// The cell of `value`, its object keys in jsonb order ([`write_jsonb`]).
    #[must_use]
    pub fn of_jsonb(value: &Value) -> Self {
        let mut out = String::new();
        write_jsonb(value, &mut out);
        Self(out.into())
    }

    /// The cell of `json`, JSON text the caller wrote.
    pub(crate) fn of_text(json: String) -> Self {
        Self(json.into())
    }

    /// The JSON string cell of `text`.
    #[must_use]
    pub fn of_str(text: &str) -> Self {
        let mut out = String::with_capacity(text.len() + 2);
        write_string(text, &mut out);
        Self(out.into())
    }

    /// The cell's JSON text.
    #[must_use]
    pub fn json(&self) -> &str {
        &self.0
    }

    /// The cell's text when it is a JSON string.
    #[must_use]
    pub fn as_string(&self) -> Option<String> {
        self.0
            .starts_with('"')
            .then(|| serde_json::from_str(&self.0).ok())
            .flatten()
    }

    /// The cell parsed back into a [`Value`].
    #[must_use]
    pub fn to_value(&self) -> Value {
        // NOTE: a cell holds valid JSON by construction, so the null is unreachable.
        serde_json::from_str(&self.0).unwrap_or(Value::Null)
    }
}

/// The cell of an AQL literal, as `to_jsonb` of its bound value wrote it.
#[must_use]
pub fn literal(lit: &TypedLit) -> Cell {
    match lit {
        TypedLit::Integer(i) => Cell::of_value(&Value::from(*i)),
        TypedLit::Real(r) => float(*r),
        TypedLit::Boolean(b) => Cell::of_value(&Value::from(*b)),
        TypedLit::String(s) | TypedLit::Temporal(s) => Cell::of_str(s),
        TypedLit::Null => Cell::null(),
    }
}

/// The cell of a `float8`: `to_jsonb` stores the shortest decimal of the
/// double as a jsonb number, which `serde_json` then reads; a non-finite
/// double is the string `float8out` writes.
#[must_use]
pub fn float(r: f64) -> Cell {
    if r.is_nan() {
        return Cell::of_str("NaN");
    }
    if r.is_infinite() {
        return Cell::of_str(if r > 0.0 { "Infinity" } else { "-Infinity" });
    }
    numeric(&r.to_string())
}

/// The cell of a `numeric` written as `text`: a JSON number when the text is
/// one, the string otherwise (`NaN`, `Infinity`), as `to_jsonb` reads it.
#[must_use]
pub fn numeric(text: &str) -> Cell {
    match serde_json::from_str::<Value>(text) {
        Ok(number @ Value::Number(_)) => Cell::of_value(&number),
        _ => Cell::of_str(text),
    }
}

/// Writes the typed value of an `ELEMENT` row from its columns.
///
/// The text is the object `crate::storage::slim::rebuild_value` builds from
/// the same columns, its keys in jsonb order; `None` when the number is no
/// JSON number.
#[must_use]
pub fn write_typed_value(
    out: &mut String,
    value_type: &str,
    num: Option<&str>,
    text: Option<&str>,
    code: Option<&str>,
    terminology: Option<&str>,
) -> Option<()> {
    let number = || -> Option<serde_json::Number> {
        match serde_json::from_str::<Value>(num?).ok()? {
            Value::Number(number) => Some(number),
            _ => None,
        }
    };
    let string_or_null = |out: &mut String, value: Option<&str>| match value {
        Some(value) => write_string(value, out),
        None => out.push_str("null"),
    };
    let coded = |out: &mut String| {
        out.push_str(r#"{"_type":"DV_CODED_TEXT","value":"#);
        write_string(text.unwrap_or_default(), out);
        out.push_str(r#","defining_code":{"_type":"CODE_PHRASE","code_string":"#);
        write_string(code.unwrap_or_default(), out);
        out.push_str(r#","terminology_id":{"_type":"TERMINOLOGY_ID","value":"#);
        write_string(terminology.unwrap_or_default(), out);
        out.push_str("}}}");
    };
    let kind = |out: &mut String| {
        out.push_str(r#"{"_type":"#);
        write_string(value_type, out);
    };
    match value_type {
        "DV_QUANTITY" => {
            let magnitude = number()?;
            kind(out);
            out.push_str(r#","units":"#);
            string_or_null(out, text);
            let _ = write!(out, r#","magnitude":{magnitude}}}"#);
        }
        "DV_COUNT" => {
            let magnitude = number()?;
            kind(out);
            let _ = write!(out, r#","magnitude":{magnitude}}}"#);
        }
        "DV_CODED_TEXT" => coded(out),
        "DV_ORDINAL" => {
            let value = number()?;
            kind(out);
            let _ = write!(out, r#","value":{value},"symbol":"#);
            coded(out);
            out.push('}');
        }
        "DV_IDENTIFIER" => {
            out.push_str(r#"{"id":"#);
            string_or_null(out, text);
            out.push_str(r#","_type":"#);
            write_string(value_type, out);
            out.push('}');
        }
        "DV_BOOLEAN" => {
            kind(out);
            if text == Some("true") {
                out.push_str(r#","value":true}"#);
            } else {
                out.push_str(r#","value":false}"#);
            }
        }
        _ => {
            kind(out);
            out.push_str(r#","value":"#);
            string_or_null(out, text);
            out.push('}');
        }
    }
    Some(())
}

/// The text `numeric_out` writes for a `numeric` in its binary wire form
/// (PostgreSQL `numeric_send`: digit count, weight, sign and display scale,
/// then base-10000 digits); `None` for a malformed value.
#[must_use]
pub fn numeric_text(bytes: &[u8]) -> Option<String> {
    let word = |i: usize| -> Option<i16> {
        let pair = bytes.get(i * 2..i * 2 + 2)?;
        Some(i16::from_be_bytes([*pair.first()?, *pair.get(1)?]))
    };
    let ndigits = usize::try_from(word(0)?).ok()?;
    let weight = i32::from(word(1)?);
    let sign = u16::from_be_bytes(word(2)?.to_be_bytes());
    let dscale = usize::try_from(word(3)?).ok()?;
    match sign {
        0xC000 => return Some("NaN".to_owned()),
        0xD000 => return Some("Infinity".to_owned()),
        0xF000 => return Some("-Infinity".to_owned()),
        _ => {}
    }
    let digit = |i: i32| -> Option<i16> {
        match usize::try_from(i) {
            Ok(i) if i < ndigits => word(4 + i),
            _ => Some(0),
        }
    };
    let mut out = String::new();
    if sign == 0x4000 {
        out.push('-');
    }
    if weight < 0 {
        out.push('0');
    } else {
        for i in 0..=weight {
            let d = digit(i)?;
            if i == 0 {
                let _ = write!(out, "{d}");
            } else {
                let _ = write!(out, "{d:04}");
            }
        }
    }
    if dscale > 0 {
        out.push('.');
        let start = out.len();
        let mut i = weight + 1;
        while out.len() - start < dscale {
            let _ = write!(out, "{:04}", digit(i)?);
            i += 1;
        }
        out.truncate(start + dscale);
    }
    Some(out)
}

/// The cell of a `timestamptz` as `to_jsonb` writes it.
///
/// `offset_seconds` is the session's offset from UTC at that instant. The
/// form is the XSD date-time (PostgreSQL `JsonEncodeDateTime`,
/// `USE_XSD_DATES`), microseconds with their trailing zeros cut and the
/// offset as `±HH:MM[:SS]`.
#[must_use]
pub fn timestamp(at: jiff::Timestamp, offset_seconds: i32) -> Cell {
    let Ok(offset) = jiff::tz::Offset::from_seconds(offset_seconds) else {
        return Cell::of_str(&at.to_string());
    };
    let civil = offset.to_datetime(at);
    let mut out = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        civil.year(),
        civil.month(),
        civil.day(),
        civil.hour(),
        civil.minute(),
        civil.second()
    );
    let micros = civil.subsec_nanosecond().div_euclid(1000);
    if micros != 0 {
        let digits = format!("{micros:06}");
        let _ = write!(out, ".{}", digits.trim_end_matches('0'));
    }
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let total = offset_seconds.unsigned_abs();
    let _ = write!(
        out,
        "{sign}{:02}:{:02}",
        total.div_euclid(3600),
        total.div_euclid(60) % 60
    );
    if !total.is_multiple_of(60) {
        let _ = write!(out, ":{:02}", total % 60);
    }
    Cell::of_str(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_is_written_as_its_jsonb_number() {
        assert_eq!(float(1.0).json(), "1");
        assert_eq!(float(0.1).json(), "0.1");
        assert_eq!(float(1e20).json(), "1e+20");
        assert_eq!(numeric("120.50").json(), "120.5");
        assert_eq!(numeric("NaN").json(), "\"NaN\"");
    }

    #[test]
    fn a_typed_value_is_written_as_its_rebuilt_object() {
        type Case<'a> = (
            &'a str,
            Option<&'a str>,
            Option<&'a str>,
            Option<&'a str>,
            Option<&'a str>,
        );
        let cases: [Case<'_>; 8] = [
            ("DV_QUANTITY", Some("120.50"), Some("mm[Hg]"), None, None),
            ("DV_QUANTITY", Some("7"), None, None, None),
            ("DV_COUNT", Some("3"), None, None, None),
            (
                "DV_CODED_TEXT",
                None,
                Some("Fasting \"x\""),
                Some("at0001"),
                Some("local"),
            ),
            (
                "DV_ORDINAL",
                Some("2"),
                Some("Moderate"),
                Some("at0012"),
                Some("local"),
            ),
            ("DV_IDENTIFIER", None, Some("1234"), None, None),
            ("DV_BOOLEAN", None, Some("true"), None, None),
            ("DV_TEXT", None, Some("free\ntext"), None, None),
        ];
        for (value_type, num, text, code, terminology) in cases {
            let mut out = String::new();
            write_typed_value(&mut out, value_type, num, text, code, terminology).unwrap();
            let rebuilt = crate::storage::slim::rebuild_value(&crate::storage::slim::TypedValue {
                value_type: value_type.to_owned(),
                num: num.map(str::to_owned),
                text: text.map(str::to_owned),
                code: code.map(str::to_owned),
                terminology: terminology.map(str::to_owned),
            })
            .unwrap();
            assert_eq!(out, Cell::of_jsonb(&rebuilt).json(), "{value_type}");
        }
        let mut out = String::new();
        assert!(write_typed_value(&mut out, "DV_COUNT", Some("x"), None, None, None).is_none());
    }

    #[test]
    fn numeric_binary_is_written_as_numeric_out() {
        // 120.50: ndigits 2, weight 0, sign +, dscale 2, digits [120, 5000].
        let bytes = [0, 2, 0, 0, 0, 0, 0, 2, 0, 120, 0x13, 0x88];
        assert_eq!(numeric_text(&bytes).as_deref(), Some("120.50"));
        // -0.005: ndigits 1, weight -1, sign -, dscale 3, digits [50].
        let bytes = [0, 1, 0xFF, 0xFF, 0x40, 0, 0, 3, 0, 50];
        assert_eq!(numeric_text(&bytes).as_deref(), Some("-0.005"));
        // 10000: ndigits 1, weight 1, digits [1].
        let bytes = [0, 1, 0, 1, 0, 0, 0, 0, 0, 1];
        assert_eq!(numeric_text(&bytes).as_deref(), Some("10000"));
    }

    #[test]
    fn a_timestamp_is_written_in_the_xsd_form() {
        let at: jiff::Timestamp = "2026-11-18T09:30:00.120Z".parse().unwrap();
        assert_eq!(timestamp(at, 0).json(), "\"2026-11-18T09:30:00.12+00:00\"");
        assert_eq!(
            timestamp(at, 3600).json(),
            "\"2026-11-18T10:30:00.12+01:00\""
        );
        let whole: jiff::Timestamp = "2026-11-18T09:30:00Z".parse().unwrap();
        assert_eq!(
            timestamp(whole, -9000).json(),
            "\"2026-11-18T07:00:00-02:30\""
        );
    }
}
