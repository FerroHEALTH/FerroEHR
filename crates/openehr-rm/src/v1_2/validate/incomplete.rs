// @generated-from-template templates/openehr-rm/validate/incomplete.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The `553|incomplete|` relaxation as two predicates: "In the `incomplete`
//! state … mandatory attributes may be absent … data may be missing, but it
//! may not be wrong" (RM `master06-change_control_package.adoc` §Incomplete
//! Content).
//!
//! [`mandatory_data_present`] answers whether anything is missing, deep;
//! [`contradicts_rm_type`] whether anything is wrong, one level. The strict
//! path uses neither.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::Value;

use crate::v1_2::model::{Container, RmAttribute, class};

/// The generic `data` slot of `POINT_EVENT` and `INTERVAL_EVENT`.
fn generic_any_slot(ty: &str, attr: &str) -> bool {
    attr == "data" && matches!(ty, "POINT_EVENT" | "INTERVAL_EVENT")
}

/// The effective type of a nested node: its `_type`, else a concrete declared
/// type.
fn effective_type<'a>(node: &'a Value, declared: &'a str) -> Option<&'a str> {
    if let Some(tag) = node.get("_type").and_then(Value::as_str) {
        return Some(tag);
    }
    class(declared)
        .filter(|c| !c.is_abstract)
        .map(|c| c.name)
        .or(None)
}

/// Whether `value` carries every mandatory attribute and non-empty `1..*`
/// container, recursively.
///
/// Biased toward `false`, which only routes the node to the
/// presence-tolerant path; an unknown class is `true`.
#[must_use]
pub fn mandatory_data_present(ty: &str, value: &Value) -> bool {
    let Some(spec) = class(ty) else { return true };
    for attr in spec.attributes {
        // `List<Octet>` renders as an inline base64 string, not an array.
        if attr.declared_type == "Octet" {
            continue;
        }
        // No `Hash` attribute is judged here (none is modelled by the
        // structural checkers either).
        if attr.container == Container::Hash {
            continue;
        }
        let Some(member) = value.get(attr.name).filter(|v| !v.is_null()) else {
            if attr.is_mandatory {
                return false;
            }
            continue;
        };
        let present = match attr.container {
            Container::None => {
                generic_any_slot(ty, attr.name) || single_member_present(member, attr)
            }
            _ => list_members_present(member, attr),
        };
        if !present {
            return false;
        }
    }
    true
}

/// Whether a single-valued member carries the mandatory data of its own
/// declared type.
fn single_member_present(member: &Value, attr: &RmAttribute) -> bool {
    if !member.is_object() {
        return true;
    }
    let Some(child) = effective_type(member, attr.declared_type) else {
        return true;
    };
    mandatory_data_present(child, member)
}

/// Whether a container member satisfies its declared lower bound and each of
/// its object members carries their own mandatory data.
fn list_members_present(member: &Value, attr: &RmAttribute) -> bool {
    let Some(items) = member.as_array() else {
        return true;
    };
    if items.is_empty() && attr.cardinality.is_some_and(|c| c.lower >= 1) {
        return false;
    }
    items.iter().all(|item| single_member_present(item, attr))
}

/// Whether one of `value`'s own members contradicts its declared type: a
/// wrong JSON kind, or a `_type` that is not a concrete descendant. Absence
/// is never a contradiction; anything unknown is `false`.
#[must_use]
pub fn contradicts_rm_type(ty: &str, value: &Value) -> bool {
    let Some(spec) = class(ty) else { return false };
    let Some(members) = value.as_object() else {
        return false;
    };
    for (key, member) in members {
        if key == "_type" || member.is_null() {
            continue;
        }
        let Some(attr) = spec.attributes.iter().find(|a| a.name == *key) else {
            continue;
        };
        if attr.declared_type == "Octet" || generic_any_slot(ty, attr.name) {
            continue;
        }
        match attr.container {
            Container::None => {
                if value_contradicts(member, attr.declared_type) {
                    return true;
                }
            }
            Container::List | Container::Set => {
                let Some(items) = member.as_array() else {
                    // A `Vec` field never decodes from a non-array.
                    return true;
                };
                if items
                    .iter()
                    .any(|item| value_contradicts(item, attr.declared_type))
                {
                    return true;
                }
            }
            Container::Hash => {}
        }
    }
    false
}

/// Whether one member contradicts its declared spec type, by the readers'
/// primitive acceptance.
fn value_contradicts(member: &Value, declared: &str) -> bool {
    match declared {
        "String" => !member.is_string(),
        "Boolean" => !member.is_boolean(),
        "Real" | "Double" => !member.is_number(),
        "Integer" => member.as_i64().is_none_or(|n| i32::try_from(n).is_err()),
        "Integer64" => member.as_i64().is_none(),
        "Character" => member.as_str().is_none_or(|s| s.chars().count() != 1),
        _ => {
            let Some(slot) = class(declared) else {
                return false;
            };
            let Some(obj) = member.as_object() else {
                return true;
            };
            match obj.get("_type") {
                Some(Value::String(tag)) => !slot.descendants.contains(&tag.as_str()),
                // A non-string `_type` fails the readers' `Option<String>` read.
                Some(_) => true,
                // An untagged payload is legal only under a concrete slot.
                None => slot.is_abstract,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{contradicts_rm_type, mandatory_data_present};

    /// A complete CLUSTER: nothing missing, nothing wrong — the strict tiers
    /// apply to it unchanged even on a `553|incomplete|` commit.
    #[test]
    fn a_complete_node_reports_neither_missing_nor_wrong() {
        let cluster = json!({
            "_type": "CLUSTER",
            "name": { "_type": "DV_TEXT", "value": "c" },
            "archetype_node_id": "at0001",
            "items": [{
                "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": "e" },
                "archetype_node_id": "at0002",
                "value": { "_type": "DV_TEXT", "value": "v" }
            }]
        });
        assert!(mandatory_data_present("CLUSTER", &cluster));
        assert!(!contradicts_rm_type("CLUSTER", &cluster));
    }

    /// An empty `CLUSTER.items` is missing, not wrong.
    #[test]
    fn an_empty_one_or_more_container_is_missing_not_wrong() {
        let cluster = json!({
            "_type": "CLUSTER",
            "name": { "_type": "DV_TEXT", "value": "c" },
            "archetype_node_id": "at0001",
            "items": []
        });
        assert!(!mandatory_data_present("CLUSTER", &cluster));
        assert!(!contradicts_rm_type("CLUSTER", &cluster));
    }

    /// An absent mandatory single-valued attribute is missing, not wrong
    /// ("single-valued attributes may have null values").
    #[test]
    fn an_absent_mandatory_attribute_is_missing_not_wrong() {
        let element = json!({
            "_type": "ELEMENT",
            "archetype_node_id": "at0002",
            "value": { "_type": "DV_TEXT", "value": "v" }
        });
        assert!(!mandatory_data_present("ELEMENT", &element));
        assert!(!contradicts_rm_type("ELEMENT", &element));
    }

    /// Missing data nested in a present node is missing.
    #[test]
    fn missing_data_inside_a_nested_node_is_seen_from_the_parent() {
        let cluster = json!({
            "_type": "CLUSTER",
            "name": { "_type": "DV_TEXT", "value": "c" },
            "archetype_node_id": "at0001",
            "items": [{
                "_type": "ELEMENT",
                "archetype_node_id": "at0002",
                "value": { "_type": "DV_TEXT", "value": "v" }
            }]
        });
        assert!(!mandatory_data_present("CLUSTER", &cluster));
    }

    /// Wrong data is wrong whether or not anything is missing.
    #[test]
    fn wrong_kinds_and_foreign_slot_types_contradict() {
        // DV_QUANTITY.magnitude is Real: a string cannot hold it.
        assert!(contradicts_rm_type(
            "DV_QUANTITY",
            &json!({ "_type": "DV_QUANTITY", "magnitude": "many", "units": "mg" })
        ));
        // LOCATABLE.name is DV_TEXT: a DV_QUANTITY is not one of its descendants.
        assert!(contradicts_rm_type(
            "ELEMENT",
            &json!({
                "_type": "ELEMENT",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_QUANTITY", "magnitude": 1.0, "units": "mg" }
            })
        ));
        // A container member that is not an array.
        assert!(contradicts_rm_type(
            "CLUSTER",
            &json!({ "_type": "CLUSTER", "archetype_node_id": "at0001", "items": 7 })
        ));
        // The accepting twin of each: the well-typed forms.
        assert!(!contradicts_rm_type(
            "DV_QUANTITY",
            &json!({ "_type": "DV_QUANTITY", "magnitude": 1.5, "units": "mg" })
        ));
        assert!(!contradicts_rm_type(
            "ELEMENT",
            &json!({
                "_type": "ELEMENT",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "e" }
            })
        ));
    }

    /// A class outside the model is not judged by either predicate.
    #[test]
    fn an_unknown_class_is_not_judged() {
        let node = json!({ "_type": "NOT_AN_RM_CLASS", "x": 1 });
        assert!(mandatory_data_present("NOT_AN_RM_CLASS", &node));
        assert!(!contradicts_rm_type("NOT_AN_RM_CLASS", &node));
    }
}
