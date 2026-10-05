// @generated-from-template templates/openehr-rm/validate/fast.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The allocation-free fast path of the RM class-invariant check (hand-written),
//! exposed via [`super::try_fast_validate`]. The authoritative tier it declines
//! to is the sibling [`super::typed_dispatch`]; the wire-boundary entry point
//! that runs the two in order lives in
//! `openehr_its::wire_validate::validate_rm_value`.
//!
//! No openEHR spec governs this module — it is our own performance design; the
//! *semantics* it realizes are exactly those of the typed dispatch in
//! [`super::typed_dispatch`] (the RM class invariants of the `*_impl.rs`
//! siblings plus the structural type-conformance rejection of a typed
//! deserialize).
//!
//! # Design: vouch-or-fall-back
//!
//! The typed dispatcher deserializes every `_type` node into its concrete RM
//! type to run a handful of scalar checks — ~1.5k `serde_json::from_value` runs
//! per populated commit. This module removes that cost for the common case:
//!
//! 1. structural conformance is checked directly against the live
//!    `&serde_json::Value` node, driven by the generated static RM model
//!    ([`crate::v1_1::model`]), so the field tables cannot drift from the
//!    generated types;
//! 2. a conforming node has its class invariants evaluated straight off the
//!    JSON map through the same invariant cores the typed `Validate` impls
//!    call, so the violation messages are byte-identical by construction;
//! 3. a node this checker cannot verifiably vouch for falls back to the
//!    authoritative typed path. The fast path never emits a rejection of its
//!    own, so a vouching bug can only degrade to the slow-correct path, never
//!    to a different wire result.
//!
//! # The conformance rules mirrored from the deserialize layer
//!
//! The vouch check replicates, conservatively, what the native canonical-JSON
//! codec's `FromJson` impls (and their `_type`-dispatched slot enums) accept:
//!
//! - mandatory single-valued attribute: present and non-`null` (the derive's
//!   shadow treats a `null` as absent → `missing field`);
//! - optional attribute: absent or `null` is fine;
//! - `Vec` attribute: absent → empty; present must be an array (`null` is a
//!   deserialize error);
//! - `_type` on a slot payload: must name a concrete descendant of the
//!   declared class; absent is only allowed when the declared class is itself
//!   concrete (the generated enums' untagged-default arm) — abstract slots
//!   require it;
//! - primitives: `String`→JSON string, `Boolean`→bool, `Real`→any number,
//!   `Integer`/`Integer64`→integral number in `i32`/`i64` range (floats
//!   rejected, as serde does), `Character`→one-char string;
//! - anything not modelled here (`Hash` attributes, classes outside
//!   [`fast_spec`], the `Interval` default-able bound flags) → fall back.
//!
//! **Shallow mode** mirrors the typed dispatcher's `prune_child_nodes` (in
//! [`super::typed_dispatch`]) for the structural container classes the typed
//! path checks via `run_shallow`: a child
//! *collection* is vouched without descending iff it is empty or contains at
//! least one object (exactly the arrays the prune empties before the typed
//! deserialize), while single-valued nested nodes are still checked (their
//! presence/type is a constraint the shallow deserialize enforces).
//!
//! Generic-parameter erasure: the dispatcher checks `HISTORY` as
//! `History<Value>` and the events as `POINT_EVENT`/`INTERVAL_EVENT` with
//! `data: Value` — so `data` on those two classes accepts anything non-absent
//! ([`generic_any_slot`]), mirroring the monomorphized `Value` payload.

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

/// Validate `value` (whose `_type` is `ty`) on the fast path. Returns `true`
/// when the node was fully handled (invariants appended to `out`); `false`
/// means the caller must run the typed dispatch instead. Nothing is appended
/// to `out` unless `true` is returned.
pub(super) fn try_validate(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) -> bool {
    let Value::Object(obj) = value else {
        return false;
    };
    // Dispatch mode: which classes have a fast invariant evaluator, and
    // whether the typed path would deserialize them shallowly (`run_shallow`)
    // or in full (`run`). Must stay in lockstep with the typed dispatch table
    // in `super::typed_dispatch` (`dispatch_typed`).
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

/// The structural specs this checker models, resolved from the generated
/// static RM model. A class outside this set (or absent from the model) makes
/// the containing node fall back to the typed path. The set covers the
/// composition-content hot path; notable *exclusions* (deliberate — their
/// typed acceptance has semantics this checker does not replicate):
/// `DV_INTERVAL` (default-able bound flags + ordered-limit invariants),
/// `REFERENCE_RANGE`, `DV_MULTIMEDIA` (octet payload), `FEEDER_AUDIT`,
/// `OBJECT_REF`/`LOCATABLE_REF`, `INSTRUCTION_DETAILS`, `PARTICIPATION`,
/// the time-specification family, `DV_STATE`, `DV_PARAGRAPH`,
/// `GENERIC_ENTRY`.
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

/// `HISTORY` is dispatched as `History<Value>` and its events as
/// `POINT_EVENT`/`INTERVAL_EVENT` with `data: Value` (generic parameter
/// monomorphized away), so `data` on the event classes accepts any non-absent,
/// non-`null` payload.
fn generic_any_slot(class: &str, attr: &str) -> bool {
    attr == "data" && matches!(class, "POINT_EVENT" | "INTERVAL_EVENT")
}

/// Whether the typed deserialize of `class` verifiably accepts `obj`. `false`
/// means "cannot vouch" (not "invalid") — the caller falls back to the typed
/// path, which decides authoritatively.
///
/// Performance: iterates the node's entries once and matches attribute names by
/// static-string compare instead of one hashed `obj.get` per attribute — this
/// runs for every `_type` node of every commit, and the map hashing was the
/// measured residual cost. Mandatory-presence is closed out by counting the
/// mandatory single-valued attributes seen against the class's total.
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
    // Absent attributes: an OPTIONAL container defaults to `None` and an
    // optional single attribute to `None`, but a missing MANDATORY attribute
    // fails the typed deserialize — whether it is single-valued or a container
    // (a mandatory container is a plain `Vec`/`NonEmptyVec` field the reader
    // requires). Every mandatory attribute must therefore have been seen.
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

/// Checks a container attribute's wire value.
///
/// A `1..*` container emits as `NonEmptyVec<T>`, whose constructor refuses an
/// empty list — so an empty array does NOT deserialize and this checker must
/// not vouch for it.
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

/// Checks the structural witness of a pruned array.
///
/// This mirrors `prune_child_nodes`: the prune keeps the FIRST member of any
/// object array as the witness, which the typed decode inspects — so this
/// checker inspects it too. An empty array on a non-`NonEmptyVec` field
/// trivially deserializes; a non-empty all-scalar array is kept verbatim and
/// typed-checked, so it is not vouched for here.
fn shallow_members_conform(items: &[Value], attr: &'static RmAttribute) -> bool {
    let Some(first) = items.first() else {
        return true;
    };
    items.iter().any(Value::is_object) && value_conforms(first, attr.declared_type, true)
}

/// Whether a single value verifiably deserializes as the declared spec type.
fn value_conforms(v: &Value, declared: &str, shallow: bool) -> bool {
    match declared {
        "String" => v.is_string(),
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

/// Whether an object in a slot declared as spec class `declared` verifiably
/// deserializes: the payload `_type` (validated by every generated
/// `Deserialize`) selects a concrete descendant; a `_type`-less payload is
/// only accepted by a slot whose declared class is itself concrete (the
/// untagged-default arm of the generated enums — an abstract slot errors).
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

/// Linear-scan field access: an RM node has ≤ ~10 keys, so a sequential
/// static-string compare beats the map's hashed lookup on this hot path
/// (performance: same reasoning as the entry iteration in [`node_conforms`]).
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

/// Run the class invariants for the (structurally conformed) node, appending
/// via the same `pub(crate)` cores the typed `Validate` impls call — the
/// violation text has a single source. Returns `false` (nothing appended)
/// for the rare sub-cases the fast path declines (e.g. a periodic `HISTORY`,
/// whose `Period_consistency` needs the typed event/offset arithmetic).
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
        // DV_AMOUNT descendants. `normal_range` (the only input of the
        // DV_ORDERED consistency invariant that could fire) is a DV_INTERVAL,
        // which `node_conforms` never vouches for — so on this path the
        // consistency check is vacuously satisfied, exactly as the typed
        // `push_normal_range_consistency` no-ops without a range.
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
        // Single-`value` identifier classes: the invariant cores live in
        // `openehr-base`; constructing the (one-string) typed value keeps the
        // violation text single-sourced there at the cost of one allocation.
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
