// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `553|incomplete|` presence relaxation, expressed as two pure
//! model-driven predicates over a canonical-JSON node.
//!
//! RM common `master06-change_control_package.adoc` §Incomplete Content
//! (NOTE): "In the `incomplete` state, a limited form of invalidity is
//! allowed: mandatory attributes may be absent. Concretely, single-valued
//! attributes may have null values and container attributes may be empty,
//! even though they may have minimum existence and cardinality respectively
//! of one. All other validity requirements must be satisfied. In other words,
//! in an `incomplete` commit, data may be missing, but it may not be wrong."
//!
//! That sentence splits the structural judgement of a node in two, and this
//! module realizes exactly that split:
//!
//! * [`mandatory_data_present`] answers "is anything MISSING?" — the half the
//!   `incomplete` state relaxes. A node it answers `false` for is one whose
//!   *typed construction* would fail on presence alone, so the caller does not
//!   drive that tier for it (the generated RM types make existence and
//!   cardinality lower bounds structural — a mandatory attribute is a plain
//!   field and a `1..*` container is a
//!   [`NonEmptyVec`](openehr_base::containers::NonEmptyVec) — which is
//!   precisely the refusal §Incomplete Content lifts).
//! * [`contradicts_rm_type`] answers "is anything WRONG?" — the half that
//!   stays at full strength: a JSON kind the declared type cannot hold, or a
//!   slot payload whose `_type` is not a descendant of the declared class.
//!
//! Both are **shallow by design on the wrongness side and deep on the presence
//! side**, because the callers differ: the presence predicate must mirror what
//! a typed decode of the whole node would touch (single-valued nested nodes
//! and container members are decoded with it), while the wrongness predicate is
//! invoked once per node by a walker that visits every child in turn.
//!
//! Neither predicate is used by the strict (`532|complete|`) path, which is
//! byte-for-byte unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::Value;

use crate::v1_2::model::{Container, RmAttribute, class};

/// The `POINT_EVENT`/`INTERVAL_EVENT` `data` slot, whose generic parameter the
/// validation dispatcher erases to `serde_json::Value` — so any non-absent
/// payload decodes. Mirrors the same exemption in the fast path.
fn generic_any_slot(ty: &str, attr: &str) -> bool {
    attr == "data" && matches!(ty, "POINT_EVENT" | "INTERVAL_EVENT")
}

/// The effective RM type of a nested node: its own wire `_type`, else the
/// declared attribute type when that names a CONCRETE class (canonical JSON
/// requires `_type` only on polymorphic slots).
fn effective_type<'a>(node: &'a Value, declared: &'a str) -> Option<&'a str> {
    if let Some(tag) = node.get("_type").and_then(Value::as_str) {
        return Some(tag);
    }
    class(declared)
        .filter(|c| !c.is_abstract)
        .map(|c| c.name)
        .or(None)
}

/// Whether `value` carries every attribute the RM model declares MANDATORY.
///
/// No container with a cardinality lower bound of 1 may be empty either, and
/// the check recurses through the nested nodes a typed decode of `value` would
/// itself decode.
///
/// `true` means "nothing is missing", so the strict tiers apply to this node
/// unchanged. `false` means the node is missing data the RM declares
/// mandatory — exactly the state RM common master06 §Incomplete Content admits
/// for a `553|incomplete|` commit ("mandatory attributes may be absent …
/// container attributes may be empty, even though they may have minimum
/// existence and cardinality respectively of one").
///
/// The predicate is deliberately biased toward `false`: it recurses into every
/// nested object and every container member, which is at least as much as any
/// decode touches. An over-eager `false` only routes the node onto the
/// presence-tolerant path (which still judges wrongness); a false `true` would
/// refuse a commit the spec admits, and is what this bias rules out.
///
/// A class the model does not know is not judged (`true`) — an unrecognised
/// `_type` is not a presence claim this layer can adjudicate.
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

/// Whether `value` positively CONTRADICTS its declared RM type `ty`.
///
/// A contradiction is a member whose JSON kind the declared type cannot hold, a
/// container member that is not an array, or a slot payload whose `_type` is not
/// a concrete descendant of the declared class.
///
/// This is the "may not be wrong" half of RM common master06 §Incomplete
/// Content, and it ignores presence entirely: an absent or `null` member is
/// never a contradiction here (that is [`mandatory_data_present`]'s question).
///
/// Only the node's OWN members are judged — a walker visits each nested node
/// with its own effective type — so the check is one level deep by design.
///
/// `false` is also the answer for anything this layer cannot adjudicate (an
/// unknown class, an undeclared member, a declared type outside the model);
/// the undeclared-member refusal and the terminology, pattern and archetype
/// passes are separate layers that keep running at full strength.
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

/// Whether one member value contradicts its declared spec type. Mirrors the
/// primitive acceptance of the canonical-JSON readers (`String` → JSON string,
/// `Integer` → an integral number in `i32` range, …); a declared class demands
/// an object whose `_type`, when present, names a concrete descendant.
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

    /// `CLUSTER.items` is `1..*`
    /// (`RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
    /// §Attributes), so an EMPTY `items` is missing data — the shape master06
    /// §Incomplete Content names verbatim ("container attributes may be empty,
    /// even though they may have minimum … cardinality … of one") — and is
    /// not wrong.
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

    /// Missing data NESTED inside a present node is still missing: the typed
    /// decode of the outer node would fail on it, so the outer node must route
    /// onto the presence-tolerant path too.
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

    /// The twin of the two above: WRONG data is wrong whether or not anything
    /// is missing — a JSON kind the declared type cannot hold, and a slot
    /// payload whose `_type` is not a descendant of the declared class.
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
