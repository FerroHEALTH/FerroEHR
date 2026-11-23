// @generated-from-template templates/openehr-rm/validate/fast.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The allocation-free fast path of the RM class-invariant check
//! ([`super::try_fast_validate`]), declining to [`super::typed_dispatch`].
//!
//! No openEHR spec governs this module: our own design. A node is checked
//! against the generated model ([`crate::v1_1::model`]) and its invariants run
//! through the same cores the typed impls call; a node it cannot vouch for
//! falls back, and it never rejects on its own. It accepts conservatively what
//! the canonical-JSON readers accept, mirrors `prune_child_nodes` in shallow
//! mode, and accepts any payload in a generic `data` slot
//! ([`generic_any_slot`]).

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::{Map, Value};

use crate::v1_1::model::{Container, RmAttribute, RmClass};
use crate::v1_1::validate::generated;
use crate::v1_1::validate::{
    valid_iso8601_date, valid_iso8601_date_time, valid_iso8601_duration, valid_iso8601_time,
};
use openehr_base::validate::{InvariantViolation, Validate};

/// Validates `value` of type `ty` on the fast path, returning whether it
/// handled the node; nothing is appended on `false`.
pub(super) fn try_validate(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) -> bool {
    let Value::Object(obj) = value else {
        return false;
    };
    // Must match the table in `super::typed_dispatch::dispatch_typed`.
    let shallow = match ty {
        // `run_shallow` classes (structural containers, scalar-only invariants).
        "CLUSTER" | "POINT_EVENT" | "INTERVAL_EVENT" | "COMPOSITION" | "EVENT_CONTEXT"
        | "ACTIVITY" | "OBSERVATION" | "INSTRUCTION" | "ACTION" | "EVALUATION" | "ADMIN_ENTRY"
        | "SECTION" | "FOLDER" => true,
        // `run` classes with a fast evaluator below.
        "CODE_PHRASE" | "DV_TEXT" | "DV_CODED_TEXT" | "DV_URI" | "DV_EHR_URI" | "DV_IDENTIFIER"
        | "TERM_MAPPING" | "DV_QUANTITY" | "DV_COUNT" | "DV_ORDINAL" | "DV_SCALE"
        | "DV_PROPORTION" | "DV_DURATION" | "DV_DATE" | "DV_TIME" | "DV_DATE_TIME"
        | "DV_PARSABLE" | "ELEMENT" | "HISTORY" | "ARCHETYPED" | "PARTY_IDENTIFIED"
        | "PARTY_RELATED" | "TERMINOLOGY_ID" | "ARCHETYPE_ID" => false,
        // Everything else (rare / complex classes) keeps the typed path.
        _ => return false,
    };
    // Every class in the mode match above is in the fast set, so resolve the
    // model spec directly (no second membership match on the hot path).
    let Some(spec) = crate::v1_1::model::class(ty) else {
        return false;
    };
    if !node_conforms(obj, spec, shallow) {
        return false;
    }
    run_invariants(ty, obj, out)
}

/// The classes this checker models; any other makes the node fall back.
fn fast_spec(name: &str) -> Option<&'static RmClass> {
    let listed = matches!(
        name,
        // identification (BASE)
        "TERMINOLOGY_ID"
            | "ARCHETYPE_ID"
            | "TEMPLATE_ID"
            | "HIER_OBJECT_ID"
            | "OBJECT_VERSION_ID"
            | "GENERIC_ID"
            | "PARTY_REF"
            // text / coded
            | "CODE_PHRASE"
            | "DV_TEXT"
            | "DV_CODED_TEXT"
            | "TERM_MAPPING"
            // uri / basic / encapsulated
            | "DV_URI"
            | "DV_EHR_URI"
            | "DV_IDENTIFIER"
            | "DV_BOOLEAN"
            | "DV_PARSABLE"
            // ordered / quantified
            | "DV_QUANTITY"
            | "DV_COUNT"
            | "DV_ORDINAL"
            | "DV_SCALE"
            | "DV_PROPORTION"
            | "DV_DURATION"
            | "DV_DATE"
            | "DV_TIME"
            | "DV_DATE_TIME"
            // common
            | "ARCHETYPED"
            | "LINK"
            | "PARTY_IDENTIFIED"
            | "PARTY_RELATED"
            | "PARTY_SELF"
            // data structures
            | "ELEMENT"
            | "CLUSTER"
            | "ITEM_TREE"
            | "ITEM_LIST"
            | "ITEM_SINGLE"
            | "ITEM_TABLE"
            | "HISTORY"
            | "POINT_EVENT"
            | "INTERVAL_EVENT"
            // composition
            | "COMPOSITION"
            | "EVENT_CONTEXT"
            | "SECTION"
            | "OBSERVATION"
            | "EVALUATION"
            | "INSTRUCTION"
            | "ACTION"
            | "ADMIN_ENTRY"
            | "ACTIVITY"
            | "ISM_TRANSITION"
            | "FOLDER"
    );
    if listed {
        crate::v1_1::model::class(name)
    } else {
        None
    }
}

/// The generic `data` slot of `POINT_EVENT` and `INTERVAL_EVENT`, which
/// accepts any present payload.
fn generic_any_slot(class: &str, attr: &str) -> bool {
    attr == "data" && matches!(class, "POINT_EVENT" | "INTERVAL_EVENT")
}

/// Whether the typed deserialize of `class` verifiably accepts `obj`; `false`
/// means "cannot vouch".
fn node_conforms(obj: &Map<String, Value>, class: &'static RmClass, shallow: bool) -> bool {
    let mut mandatory_seen = 0usize;
    for (key, v) in obj {
        if key == "_type" {
            continue; // validated by the caller's slot/tag dispatch
        }
        // Unknown wire keys are ignored by the derive's deserialize.
        let Some(attr) = class.attributes.iter().find(|a| a.name == key) else {
            continue;
        };
        match attribute_conforms(v, attr, class, shallow) {
            AttrCheck::Refused => return false,
            // A mandatory attribute the wire leaves absent is `missing field`
            // to the typed reader.
            AttrCheck::Absent if attr.is_mandatory => return false,
            AttrCheck::Conforms if attr.is_mandatory => mandatory_seen += 1,
            AttrCheck::Absent | AttrCheck::Conforms => {}
        }
    }
    // Every mandatory attribute must have been seen.
    mandatory_seen == class.attributes.iter().filter(|a| a.is_mandatory).count()
}

/// What the typed reader would make of one attribute's wire value.
enum AttrCheck {
    /// JSON `null`, which the reader's `Option` shadow treats as absent.
    Absent,
    /// The value verifiably deserializes as the attribute's declared shape.
    Conforms,
    /// The typed reader would refuse the value, so this checker cannot vouch.
    Refused,
}

/// Checks one attribute's wire value against its declared container shape.
fn attribute_conforms(
    v: &Value,
    attr: &'static RmAttribute,
    class: &'static RmClass,
    shallow: bool,
) -> AttrCheck {
    match attr.container {
        Container::None => single_conforms(v, attr, class, shallow),
        Container::List | Container::Set => list_conforms(v, attr, shallow),
        // No `Hash` attribute is modelled here.
        Container::Hash => AttrCheck::Refused,
    }
}

/// Checks a single-valued attribute's wire value.
fn single_conforms(
    v: &Value,
    attr: &'static RmAttribute,
    class: &'static RmClass,
    shallow: bool,
) -> AttrCheck {
    if v.is_null() {
        return AttrCheck::Absent;
    }
    if generic_any_slot(class.name, attr.name) {
        return AttrCheck::Conforms;
    }
    if value_conforms(v, attr.declared_type, shallow) {
        AttrCheck::Conforms
    } else {
        AttrCheck::Refused
    }
}

/// Checks a container attribute's wire value; an empty `1..*` array is not
/// vouched for.
fn list_conforms(v: &Value, attr: &'static RmAttribute, shallow: bool) -> AttrCheck {
    // `Vec` never deserializes from a non-array (incl. `null`).
    let Value::Array(items) = v else {
        return AttrCheck::Refused;
    };
    let lower_bound_one = attr.cardinality.is_some_and(|c| c.lower >= 1) || attr.nonempty;
    if lower_bound_one && items.is_empty() {
        return AttrCheck::Refused;
    }
    let members_conform = if shallow {
        shallow_members_conform(items, attr)
    } else {
        items
            .iter()
            .all(|item| value_conforms(item, attr.declared_type, false))
    };
    if members_conform {
        AttrCheck::Conforms
    } else {
        AttrCheck::Refused
    }
}

/// Checks the first object member of a pruned array, as `prune_child_nodes`
/// keeps it; a non-empty all-scalar array is not vouched for.
fn shallow_members_conform(items: &[Value], attr: &'static RmAttribute) -> bool {
    let Some(first) = items.first() else {
        return true;
    };
    items.iter().any(Value::is_object) && value_conforms(first, attr.declared_type, true)
}

/// Whether a single value verifiably deserializes as the declared spec type.
fn value_conforms(v: &Value, declared: &str, shallow: bool) -> bool {
    match declared {
        // BASE 1.3.0 `Hrid` lowers to `String`; its grammar is the holding
        // class's invariant (`ARCHETYPE_ID.Value_valid`), checked below.
        "String" | "Hrid" => v.is_string(),
        "Boolean" => v.is_boolean(),
        // Any JSON number deserializes as `f64`.
        "Real" | "Double" => v.is_number(),
        // serde rejects floats for integer targets; range-check the rest.
        "Integer" => v.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
        "Integer64" => v.as_i64().is_some(),
        // serde `char`: a one-character string.
        "Character" => v.as_str().is_some_and(|s| s.chars().count() == 1),
        _ => class_slot_conforms(v, declared, shallow),
    }
}

/// Whether an object in a slot declared `declared` deserializes: its `_type`
/// names a concrete descendant, or it has none and the slot is concrete.
fn class_slot_conforms(v: &Value, declared: &str, shallow: bool) -> bool {
    let Value::Object(obj) = v else {
        return false;
    };
    let Some(slot) = crate::v1_1::model::class(declared) else {
        return false;
    };
    // Canonical JSON emits `_type` as the first key; peek there before paying
    // a hashed lookup (performance: this runs per nested slot of every node).
    let tag = match obj.iter().next() {
        Some((k, v)) if k == "_type" => Some(v),
        _ => obj.get("_type"),
    };
    let target = match tag {
        Some(Value::String(t)) => {
            if !slot.descendants.contains(&t.as_str()) {
                return false;
            }
            t.as_str()
        }
        // A non-string `_type` fails the shadow's `Option<String>` read.
        Some(_) => return false,
        None => {
            if slot.is_abstract {
                return false;
            }
            declared
        }
    };
    let Some(spec) = fast_spec(target) else {
        return false;
    };
    node_conforms(obj, spec, shallow)
}

// ── invariant evaluation on the conformed node ───────────────────────────────

/// Linear-scan field access, faster than hashing on a node of few keys.
fn field<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.iter().find_map(|(k, v)| (k == key).then_some(v))
}

/// The attribute value as `&str` (`None` for absent or `null` — the typed
/// `Option<String>` reading).
fn str_of<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    field(obj, key).and_then(Value::as_str)
}

/// The attribute value as `f64` (`None` for absent/`null`).
fn f64_of(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    field(obj, key).and_then(Value::as_f64)
}

/// The attribute value as `bool` (`None` for absent/`null`).
fn bool_of(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    field(obj, key).and_then(Value::as_bool)
}

/// Whether the attribute is present and non-`null` (the typed
/// `Option<T>::is_some` reading).
fn present(obj: &Map<String, Value>, key: &str) -> bool {
    field(obj, key).is_some_and(|v| !v.is_null())
}

/// Runs the class invariants of a conforming node through the shared cores;
/// `false` for a declined case such as a periodic `HISTORY`.
#[expect(
    clippy::too_many_lines,
    reason = "a flat per-RM-class dispatch table; splitting it would scatter the mirror of the typed invariant table"
)]
fn run_invariants(ty: &str, obj: &Map<String, Value>, out: &mut Vec<InvariantViolation>) -> bool {
    match ty {
        "CODE_PHRASE" => {
            let Some(code_string) = str_of(obj, "code_string") else {
                return false;
            };
            generated::code_phrase_core(code_string, out);
        }
        "DV_TEXT" | "DV_CODED_TEXT" => {
            let Some(value) = str_of(obj, "value") else {
                return false;
            };
            generated::dv_text_core(ty, value, str_of(obj, "formatting"), out);
        }
        "DV_URI" => {
            let Some(value) = str_of(obj, "value") else {
                return false;
            };
            crate::v1_1::data_types::uri::dv_uri_impl::push_dv_uri_invariants(value, out);
        }
        "DV_EHR_URI" => {
            let Some(value) = str_of(obj, "value") else {
                return false;
            };
            crate::v1_1::data_types::uri::dv_ehr_uri_impl::push_dv_ehr_uri_invariants(value, out);
        }
        "DV_IDENTIFIER" => {
            let Some(id) = str_of(obj, "id") else {
                return false;
            };
            generated::dv_identifier_core(id, out);
        }
        "TERM_MAPPING" => {
            let Some(code) = str_of(obj, "match").and_then(|s| s.chars().next()) else {
                return false;
            };
            generated::term_mapping_core(code, out);
        }
        "DV_PARSABLE" => {
            let Some(formalism) = str_of(obj, "formalism") else {
                return false;
            };
            generated::dv_parsable_core(formalism, out);
        }
        // `normal_range` is a DV_INTERVAL, never vouched for, so the consistency
        // invariant cannot fire here.
        "DV_QUANTITY" | "DV_COUNT" | "DV_DURATION" => {
            if ty == "DV_DURATION" {
                let Some(value) = str_of(obj, "value") else {
                    return false;
                };
                generated::temporal_value_core(ty, valid_iso8601_duration(value), out);
            }
            generated::dv_amount_core(
                ty,
                f64_of(obj, "accuracy"),
                bool_of(obj, "accuracy_is_percent"),
                str_of(obj, "magnitude_status"),
                out,
            );
        }
        "DV_DATE" | "DV_TIME" | "DV_DATE_TIME" => {
            let Some(value) = str_of(obj, "value") else {
                return false;
            };
            let valid = match ty {
                "DV_DATE" => valid_iso8601_date(value),
                "DV_TIME" => valid_iso8601_time(value),
                _ => valid_iso8601_date_time(value),
            };
            generated::temporal_value_core(ty, valid, out);
            generated::magnitude_status_core(ty, str_of(obj, "magnitude_status"), out);
        }
        // Only the DV_ORDERED consistency invariant, which cannot fire
        // without a (never-vouched) normal_range.
        "DV_ORDINAL" | "DV_SCALE" => {}
        "DV_PROPORTION" => {
            let (Some(numerator), Some(denominator)) =
                (f64_of(obj, "numerator"), f64_of(obj, "denominator"))
            else {
                return false;
            };
            let Some(kind) = field(obj, "type")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
            else {
                return false;
            };
            let precision = field(obj, "precision")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok());
            generated::dv_proportion_core(numerator, denominator, kind, precision, out);
            generated::dv_amount_core(
                ty,
                f64_of(obj, "accuracy"),
                bool_of(obj, "accuracy_is_percent"),
                str_of(obj, "magnitude_status"),
                out,
            );
        }
        "ELEMENT" => {
            let Some(node_id) = str_of(obj, "archetype_node_id") else {
                return false;
            };
            crate::v1_1::data_structures::representation::element_impl::push_element_invariants(
                present(obj, "value"),
                present(obj, "null_flavour"),
                present(obj, "null_reason"),
                node_id,
                out,
            );
        }
        // LOCATABLE containers whose only class invariant is the inherited
        // Archetype_node_id_valid.
        "CLUSTER" | "SECTION" | "FOLDER" | "POINT_EVENT" | "INTERVAL_EVENT" => {
            let Some(node_id) = str_of(obj, "archetype_node_id") else {
                return false;
            };
            generated::archetype_node_id_core(ty, node_id, out);
        }
        "HISTORY" => {
            // Period_consistency needs the typed event-offset arithmetic —
            // decline periodic histories (rare) to the typed path.
            if present(obj, "period") {
                return false;
            }
            let Some(node_id) = str_of(obj, "archetype_node_id") else {
                return false;
            };
            let events_empty = field(obj, "events")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty);
            generated::history_basic_core(events_empty, present(obj, "summary"), node_id, out);
        }
        "COMPOSITION" => {
            let Some(node_id) = str_of(obj, "archetype_node_id") else {
                return false;
            };
            generated::composition_core(present(obj, "archetype_details"), node_id, out);
        }
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY" => {
            let Some(node_id) = str_of(obj, "archetype_node_id") else {
                return false;
            };
            generated::entry_root_core(ty, present(obj, "archetype_details"), node_id, out);
        }
        "ACTIVITY" => {
            let (Some(action_archetype_id), Some(node_id)) = (
                str_of(obj, "action_archetype_id"),
                str_of(obj, "archetype_node_id"),
            ) else {
                return false;
            };
            generated::activity_core(action_archetype_id, node_id, out);
        }
        "EVENT_CONTEXT" => {
            generated::event_context_core(str_of(obj, "location"), out);
        }
        "ARCHETYPED" => {
            let Some(rm_version) = str_of(obj, "rm_version") else {
                return false;
            };
            generated::archetyped_core(rm_version, out);
        }
        "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            let has_identifiers = field(obj, "identifiers")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
            generated::party_identified_core(
                ty,
                str_of(obj, "name"),
                has_identifiers,
                present(obj, "external_ref"),
                out,
            );
        }
        // The identifier cores live in `openehr-base`.
        "TERMINOLOGY_ID" => {
            let Some(value) = str_of(obj, "value") else {
                return false;
            };
            openehr_base::v1_2::prelude::TerminologyId {
                value: value.to_owned(),
            }
            .validate_invariants(out);
        }
        "ARCHETYPE_ID" => {
            let Some(value) = str_of(obj, "value") else {
                return false;
            };
            openehr_base::v1_2::prelude::ArchetypeId {
                value: value.to_owned(),
            }
            .validate_invariants(out);
        }
        _ => return false,
    }
    true
}
