// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM-instance terminology pass: a [`ValidationKind::Terminology`] message
//! per violation, over the binding table of
//! [`openehr_rm::v1_2::validate::terminology`] (TERM 3.1.0, [`openehr_term::bundle`]).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use openehr_rm::v1_2::model::declared_concrete_type;
use openehr_rm::v1_2::validate::terminology::{Slot, slot_is_violated, slots_for};
use serde_json::Value;

use super::{ValidationKind, ValidationMessage, norm_path, push};

/// Pass 2: recurse the instance, checking every coded slot its owning RM type
/// binds.
///
/// `path` and `declared` work as in [`super::rm_invariant_pass`].
pub(crate) fn terminology_pass(
    out: &mut Vec<ValidationMessage>,
    v: &Value,
    path: &mut String,
    declared: Option<&str>,
) {
    use std::fmt::Write as _;
    let Some(obj) = v.as_object() else { return };
    let this_type = obj
        .get("_type")
        .and_then(Value::as_str)
        .or(declared)
        .unwrap_or("");

    // Coded slots fixed by the owning RM type (the shared binding table).
    for slot in slots_for(this_type) {
        if let Some(node) = obj.get(slot.field) {
            let base = path.len();
            let _ = write!(path, "/{}", slot.field);
            check_code(out, slot, node, path);
            path.truncate(base);
        }
    }

    for (k, val) in obj {
        if k.starts_with('_') {
            continue;
        }
        let child_declared = declared_concrete_type(this_type, k);
        match val {
            Value::Array(a) => {
                for (i, item) in a.iter().enumerate() {
                    if item.is_object() {
                        let base = path.len();
                        let _ = write!(path, "/{k}[{i}]");
                        terminology_pass(out, item, path, child_declared);
                        path.truncate(base);
                    }
                }
            }
            Value::Object(_) => {
                let base = path.len();
                let _ = write!(path, "/{k}");
                terminology_pass(out, val, path, child_declared);
                path.truncate(base);
            }
            _ => {}
        }
    }
}

/// Validate a coded node against its slot binding, recording a
/// [`ValidationKind::Terminology`] violation naming the vocabulary label.
fn check_code(out: &mut Vec<ValidationMessage>, slot: &Slot, node: &Value, path: &str) {
    if !slot_is_violated(slot, node) {
        return;
    }
    // `slot_is_violated` returned true, so the node carries an in-scope code;
    // re-read it for the message. (The bare `defining_code`/`code_string`
    // read cannot fail here.)
    let code = node
        .get("defining_code")
        .unwrap_or(node)
        .get("code_string")
        .and_then(Value::as_str)
        .unwrap_or("");
    push(
        out,
        norm_path(path),
        format!(
            "code '{code}' is not a valid {} (openEHR terminology)",
            slot.binding.label()
        ),
        ValidationKind::Terminology,
    );
}
