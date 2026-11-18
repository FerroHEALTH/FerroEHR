// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The per-node step of the RM-invariant walk: the fields a node's checks
//! read, the descent into one attribute, and the declared-slot-type rule.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::fmt::Write as _;

use openehr_rm::v1_2::model::declared_concrete_type;
use serde_json::Value;

use super::{LowerBounds, ValidationKind, ValidationMessage, norm_path, push, rm_invariant_pass};

/// The node fields every per-node check reads, collected in ONE projection
/// pass so none of them pays a hashed map lookup.
///
/// This runs for every node of every commit; the only remaining per-node gets
/// are gated behind a matching `_type`.
pub(super) struct NodeFields<'a> {
    pub(super) ty: Option<&'a str>,
    pub(super) node_id: Option<&'a str>,
    pub(super) has_archetype_details: bool,
    pub(super) details_archetype_id: Option<&'a str>,
}

impl<'a> NodeFields<'a> {
    pub(super) fn of(obj: &'a serde_json::Map<String, Value>) -> Self {
        let mut fields = NodeFields {
            ty: None,
            node_id: None,
            has_archetype_details: false,
            details_archetype_id: None,
        };
        for (k, val) in obj {
            match k.as_str() {
                "_type" => fields.ty = val.as_str(),
                "archetype_node_id" => fields.node_id = val.as_str(),
                "archetype_details" => {
                    fields.has_archetype_details = !val.is_null();
                    fields.details_archetype_id = val
                        .get("archetype_id")
                        .and_then(|a| a.get("value"))
                        .and_then(Value::as_str);
                }
                _ => {}
            }
        }
        fields
    }
}

/// Descends into one attribute of a node, checking each member's declared slot
/// type on the way down.
pub(super) fn descend_attribute(
    out: &mut Vec<ValidationMessage>,
    k: &str,
    val: &Value,
    path: &mut String,
    ty: Option<&str>,
    bounds: LowerBounds,
) {
    let child_declared = ty.and_then(|t| declared_concrete_type(t, k));
    match val {
        Value::Array(a) => {
            for (i, item) in a.iter().enumerate() {
                let base = path.len();
                let _ = write!(path, "/{k}[{i}]");
                if item.is_object() {
                    check_slot_type(out, ty, k, item, path);
                    rm_invariant_pass(out, item, path, child_declared, bounds);
                } else if let Some(iv) = ty.and_then(|parent| {
                    openehr_rm::v1_2::validate::check_slot_member_is_object(parent, k)
                }) {
                    // A scalar member of a class-typed list slot is the same
                    // positive contradiction a foreign `_type` is — its
                    // declared type is an RM class, which no JSON scalar can
                    // be.
                    push(out, norm_path(path), iv.message, ValidationKind::Invariant);
                }
                path.truncate(base);
            }
        }
        Value::Object(_) => {
            let base = path.len();
            let _ = write!(path, "/{k}");
            check_slot_type(out, ty, k, val, path);
            rm_invariant_pass(out, val, path, child_declared, bounds);
            path.truncate(base);
        }
        _ => {}
    }
}

/// The declared-slot-type conformance rule (the WRONGNESS half of slot typing,
/// never relaxed — RM common master06 §Incomplete Content): a TAGGED child
/// must claim the slot's declared type or a subtype.
///
/// One model-driven rule for single slots and list members alike; the
/// shallow-pruned typed dispatch cannot see it (each pruned member
/// re-dispatches on its own `_type`), so the walk owns it.
pub(super) fn check_slot_type(
    out: &mut Vec<ValidationMessage>,
    parent_ty: Option<&str>,
    attribute: &str,
    item: &Value,
    path: &str,
) {
    if let (Some(parent), Some(wire)) = (parent_ty, item.get("_type").and_then(Value::as_str))
        && let Some(iv) =
            openehr_rm::v1_2::validate::check_declared_slot_type(parent, attribute, wire)
    {
        push(out, norm_path(path), iv.message, ValidationKind::Invariant);
    }
}
