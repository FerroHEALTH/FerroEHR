// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Generic ODIN rendering shared by every ODIN-shaped section: optional string
//! attributes, keyed string maps and lists, the `_default` pseudo-attribute
//! carrying a canonical-JSON value, and the ODIN leaf literals (quoted strings,
//! scalars, `TERM_CODE_REF`) — `LANG/docs/odin/master03` +
//! `master07-leaf_data`.

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use std::collections::BTreeMap;

use openehr_base::prelude::TerminologyCode;

use crate::print::Printer;

impl Printer {
    // ── ODIN helpers ────────────────────────────────────────────────────────
    pub(super) fn opt_string(&mut self, depth: usize, key: &str, v: Option<&str>) {
        if let Some(v) = v {
            self.line(depth, &format!("{key} = <{}>", quoted(v)));
        }
    }

    /// Emit a `C_DEFINED_OBJECT.default_value` as the `_default` pseudo-attribute
    /// (`master06-default_values.adoc` §Syntax): `_default = (RM_TYPE) < … >`
    /// with the canonical-JSON intermediate rendered as ODIN — the inverse of
    /// the cADL parser's `_default` handling (`odin_to_json`). Scalar and
    /// object shapes round-trip through print → parse exactly; a JSON array
    /// of objects re-parses as a keyed object (see the [`Self::odin_json_entry`]
    /// NOTE) — the ODIN text is the durable form either way.
    pub(super) fn default_value(&mut self, v: &serde_json::Value, depth: usize) {
        if let serde_json::Value::Object(m) = v
            && let Some(literal) = interval_literal(m)
        {
            self.line(depth, &format!("_default = <{literal}>"));
            return;
        }
        match v {
            serde_json::Value::Object(m) => {
                let head = match m.get("_type").and_then(serde_json::Value::as_str) {
                    Some(t) => format!("_default = ({t}) <"),
                    None => "_default = <".to_owned(),
                };
                self.line(depth, &head);
                for (k, val) in m {
                    if k == "_type" {
                        continue;
                    }
                    self.odin_json_entry(k, val, depth + 1);
                }
                self.line(depth, ">");
            }
            other => self.line(depth, &format!("_default = <{}>", odin_scalar(other))),
        }
    }

    /// One ODIN attribute line (or block) for a canonical-JSON member.
    ///
    /// NOTE: a JSON array of objects has no positional ODIN form — ODIN
    /// containers are keyed lists — so it renders as `["1"] = <…>` entries;
    /// re-parsing yields a `"1"`-keyed object rather than an array. The ODIN
    /// text is the durable ADL2 form; the JSON intermediate carries no
    /// spec-mandated shape (no openEHR spec governs it — our own design,
    /// matching the parser's `odin_to_json`).
    fn odin_json_entry(&mut self, key: &str, v: &serde_json::Value, depth: usize) {
        if let serde_json::Value::Object(m) = v
            && let Some(literal) = interval_literal(m)
        {
            self.line(depth, &format!("{key} = <{literal}>"));
            return;
        }
        match v {
            serde_json::Value::Null => {}
            serde_json::Value::Object(m) => {
                let head = match m.get("_type").and_then(serde_json::Value::as_str) {
                    Some(t) => format!("{key} = ({t}) <"),
                    None => format!("{key} = <"),
                };
                self.line(depth, &head);
                for (k, val) in m {
                    if k == "_type" {
                        continue;
                    }
                    self.odin_json_entry(k, val, depth + 1);
                }
                self.line(depth, ">");
            }
            serde_json::Value::Array(items) if items.iter().all(is_json_scalar) => {
                let joined = items.iter().map(odin_scalar).collect::<Vec<_>>().join(", ");
                self.line(depth, &format!("{key} = <{joined}>"));
            }
            serde_json::Value::Array(items) => {
                self.line(depth, &format!("{key} = <"));
                for (i, item) in items.iter().enumerate() {
                    self.odin_json_entry(
                        &format!("[{}]", quoted(&(i + 1).to_string())),
                        item,
                        depth + 1,
                    );
                }
                self.line(depth, ">");
            }
            scalar => self.line(depth, &format!("{key} = <{}>", odin_scalar(scalar))),
        }
    }

    pub(super) fn odin_string_map(
        &mut self,
        depth: usize,
        key: &str,
        m: &BTreeMap<String, String>,
    ) {
        if m.is_empty() {
            return;
        }
        self.line(depth, &format!("{key} = <"));
        for (k, v) in m {
            self.line(depth + 1, &format!("[{}] = <{}>", quoted(k), quoted(v)));
        }
        self.line(depth, ">");
    }

    pub(super) fn odin_string_list(&mut self, depth: usize, key: &str, list: &[String]) {
        if list.is_empty() {
            return;
        }
        let joined = list
            .iter()
            .map(|s| quoted(s))
            .collect::<Vec<_>>()
            .join(", ");
        self.line(depth, &format!("{key} = <{joined}>"));
    }
}

/// The ODIN interval literal a canonical-JSON `Interval<T>` object denotes, or
/// `None` when the object is not one.
///
/// The inverse of the interval arm of the cADL parser's `_default` handling
/// (`crate::odin::odin_to_json`): the accepted shape is exactly the one that
/// encoder produces — `_type` `Point_interval`/`Proper_interval`, the two
/// bounds present iff their `*_unbounded` flag is false, the four boundary
/// flags, and nothing else — and the point/proper tag must agree with the
/// bounds. Any other object (a hand-written `(Proper_interval) <…>` block, an
/// interval-shaped RM value) falls through to the generic block rendering, so
/// print → parse stays exact in both directions.
///
/// Syntax: `LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered
/// Primitive Types — the relational operators precede the bound they qualify
/// (`|>N..<M|`, `|>=N|`), an omitted side is the `*` unbounded marker, and a
/// single value is the bare `|N|` form.
fn interval_literal(m: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let is_point = match m.get("_type").and_then(serde_json::Value::as_str)? {
        "Point_interval" => true,
        "Proper_interval" => false,
        _ => return None,
    };
    let lower_unbounded = m.get("lower_unbounded")?.as_bool()?;
    let upper_unbounded = m.get("upper_unbounded")?.as_bool()?;
    let lower_included = m.get("lower_included")?.as_bool()?;
    let upper_included = m.get("upper_included")?.as_bool()?;
    let lower = m.get("lower");
    let upper = m.get("upper");
    // Exactly the encoder's field set, and bounds present iff bounded.
    let expected_len = 5 + usize::from(lower.is_some()) + usize::from(upper.is_some());
    if m.len() != expected_len
        || lower.is_some() == lower_unbounded
        || upper.is_some() == upper_unbounded
        || (lower_unbounded && lower_included)
        || (upper_unbounded && upper_included)
    {
        return None;
    }
    if lower.is_some_and(|v| !is_json_scalar(v)) || upper.is_some_and(|v| !is_json_scalar(v)) {
        return None;
    }
    let point =
        !lower_unbounded && !upper_unbounded && lower_included && upper_included && lower == upper;
    if point != is_point {
        return None;
    }
    let literal = match (lower, upper) {
        (Some(l), Some(_)) if point => format!("|{}|", odin_scalar(l)),
        (Some(l), Some(u)) => format!(
            "|{}{}..{}{}|",
            if lower_included { "" } else { ">" },
            odin_scalar(l),
            if upper_included { "" } else { "<" },
            odin_scalar(u)
        ),
        (Some(l), None) => format!(
            "|{}{}|",
            if lower_included { ">=" } else { ">" },
            odin_scalar(l)
        ),
        (None, Some(u)) => format!(
            "|{}{}|",
            if upper_included { "<=" } else { "<" },
            odin_scalar(u)
        ),
        // Unbounded on both sides: the `*` marker is the only spelling that
        // states an absent endpoint explicitly.
        (None, None) => "|*..*|".to_owned(),
    };
    Some(literal)
}

/// Whether a canonical-JSON value renders as one ODIN scalar token.
fn is_json_scalar(v: &serde_json::Value) -> bool {
    matches!(
        v,
        serde_json::Value::String(_) | serde_json::Value::Number(_) | serde_json::Value::Bool(_)
    )
}

/// Render a canonical-JSON scalar as its ODIN literal (the inverse of the
/// cADL parser's `odin_to_json` scalar arms).
fn odin_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => quoted(s),
        serde_json::Value::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
        serde_json::Value::Number(n) => n.to_string(),
        // Null/containers never reach here (`is_json_scalar` gates the list
        // form; containers take the block form).
        other => other.to_string(),
    }
}

/// A double-quoted, `master03`-escaped string literal.
pub(super) fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// `[terminology::code]` (with optional `(version)`), the ODIN `TERM_CODE_REF`
/// form (`master07-leaf_data`).
pub(super) fn term_code_str(t: &TerminologyCode) -> String {
    let id = match &t.terminology_version {
        Some(v) => format!("{}({v})", t.terminology_id),
        None => t.terminology_id.clone(),
    };
    format!("[{id}::{}]", t.code_string)
}
