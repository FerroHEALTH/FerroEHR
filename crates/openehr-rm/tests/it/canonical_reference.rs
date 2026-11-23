// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    reason = "a reference helper outside #[test] fns, which the clippy.toml allow-*-in-tests \
              scoping does not reach, also compiled into the bench"
)]

//! The RFC 8785 writer as it stood before the fast paths of #5682, kept as the
//! reference the current writer is held byte-identical to and timed against
//! (`tests/it/canonical_fast_path.rs`, `benches/canonical_form.rs`).
//!
//! It scans strings by character, escapes a control character through
//! `write!`, writes every number through `ryu-js`, and sorts names by their
//! UTF-16 code units. Only finite numbers reach it, so it has no error path.

use std::fmt::Write as _;

use serde_json::Value;

/// The canonical form of `value` by the reference writer these tests compare against.
pub(crate) fn canonical(value: &Value) -> String {
    let mut out = String::new();
    value_to(value, &mut out);
    out
}

/// Writes `value` to `out`.
fn value_to(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => {
            let double = number.as_f64().unwrap_or(f64::NAN);
            assert!(double.is_finite(), "the reference takes finite numbers");
            out.push_str(ryu_js::Buffer::new().format_finite(double));
        }
        Value::String(text) => string_to(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                value_to(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut members: Vec<(&String, &Value)> = map.iter().collect();
            members.sort_by(|left, right| left.0.encode_utf16().cmp(right.0.encode_utf16()));
            out.push('{');
            for (index, (name, member)) in members.into_iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                string_to(name, out);
                out.push(':');
                value_to(member, out);
            }
            out.push('}');
        }
    }
}

/// Whether the character `c` takes an escape.
fn escaped(c: char) -> bool {
    c == '"' || c == '\\' || c < ' '
}

/// Writes `text` to `out` as a JSON string, one character run at a time.
fn string_to(text: &str, out: &mut String) {
    out.push('"');
    for run in text.split_inclusive(escaped) {
        let mut chars = run.chars();
        match chars.next_back() {
            Some(last) if escaped(last) => {
                out.push_str(chars.as_str());
                match last {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\u{8}' => out.push_str("\\b"),
                    '\t' => out.push_str("\\t"),
                    '\n' => out.push_str("\\n"),
                    '\u{c}' => out.push_str("\\f"),
                    '\r' => out.push_str("\\r"),
                    control => {
                        write!(out, "\\u{:04x}", u32::from(control))
                            .expect("writing to a String cannot fail");
                    }
                }
            }
            _ => out.push_str(run),
        }
    }
    out.push('"');
}
