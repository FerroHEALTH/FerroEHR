// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a settings class keeps of the settings: only the members the
//! decision reads, each coded value by its code phrase alone. Our own design
//! (#5588).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification), and its class is the same open object"
)]

use openehr_its::json::to_canonical_value;
use openehr_rm::prelude::{DvDateTime, PartyProxy, PartySelf};
use serde_json::Value;

use super::{INSTANT, retain, set};
use crate::service::ehr::access_projection::{Validity, validity};

/// Keeps of the settings `value` only what the decision reads.
pub(super) fn normalize_settings(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(default) = object.get_mut("default_access") {
        *default = normal_coded(default);
    }
    for key in ["directives", "exclusions"] {
        if let Some(Value::Array(directives)) = object.get_mut(key) {
            directives.iter_mut().for_each(normalize_directive);
        }
    }
    if let Some(marking) = object.get_mut("marking") {
        normalize_marking(marking);
    }
}

/// Keeps of a `CONFIDENTIALITY_MARKING` only its level: the decision never
/// reads who set it, when or why, so EHRs of one level share a class.
pub(super) fn normalize_marking(marking: &mut Value) {
    let level = marking.get("confidentiality").map(normal_coded);
    retain(marking, &["_type"]);
    if let (Some(object), Some(level)) = (marking.as_object_mut(), level) {
        object.insert("confidentiality".to_owned(), level);
        let set_by = PartyProxy::PartySelf(PartySelf { external_ref: None });
        let set_at = DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: INSTANT.to_owned(),
        };
        object.insert("set_by".to_owned(), to_canonical_value(&set_by));
        object.insert("set_at".to_owned(), to_canonical_value(&set_at));
    }
}

/// The members of a directive the decision reads.
const DIRECTIVE: [&str; 10] = [
    "_type",
    "kind",
    "effect",
    "applies_to",
    "operations",
    "purposes",
    "data",
    "time_validity",
    "emergency_override",
    "exceptions",
];

/// Keeps of the directive `value` only what the decision reads.
fn normalize_directive(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.retain(|key, _| DIRECTIVE.contains(&key.as_str()) || key == "name");
    if let Some(name) = object.get_mut("name") {
        retain(name, &["_type", "value"]);
        set(name, "_type", "DV_TEXT");
        set(name, "value", "directive");
    }
    object.insert(
        "archetype_node_id".to_owned(),
        Value::String("at0000".to_owned()),
    );
    for key in ["kind", "effect"] {
        if let Some(coded) = object.get_mut(key) {
            *coded = normal_coded(coded);
        }
    }
    for key in ["operations", "purposes"] {
        if let Some(Value::Array(codes)) = object.get_mut(key) {
            for c in codes.iter_mut() {
                *c = normal_coded(c);
            }
        }
    }
    if let Some(validity) = object.get_mut("time_validity") {
        normalize_interval(validity);
    }
    if let Some(Value::Array(scopes)) = object.get_mut("applies_to") {
        for scope in scopes.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["role", "relationship", "caller_kind"] {
                if let Some(coded) = scope.get_mut(key) {
                    *coded = normal_coded(coded);
                }
            }
        }
    }
    if let Some(Value::Array(scopes)) = object.get_mut("data") {
        for scope in scopes.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["categories", "sensitivities"] {
                if let Some(Value::Array(codes)) = scope.get_mut(key) {
                    for c in codes.iter_mut() {
                        *c = normal_coded(c);
                    }
                }
            }
            if let Some(recorded) = scope.get_mut("recorded") {
                normalize_interval(recorded);
            }
        }
    }
    if let Some(Value::Array(exceptions)) = object.get_mut("exceptions") {
        exceptions.iter_mut().for_each(normalize_directive);
    }
}

/// Keeps of a readable `DV_INTERVAL<DV_DATE_TIME>` only its bounds and their
/// flags, dropping a bound the interval marks unbounded; an unreadable one
/// stays as it is, so the class holds what the decision makes of it.
fn normalize_interval(value: &mut Value) {
    let readable = serde_json::from_value(value.clone())
        .is_ok_and(|interval| matches!(validity(&interval), Validity::Within { .. }));
    let Some(object) = value.as_object_mut().filter(|_| readable) else {
        return;
    };
    for (bound, unbounded) in [("lower", "lower_unbounded"), ("upper", "upper_unbounded")] {
        let open = object
            .get(unbounded)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if open {
            object.remove(bound);
        } else if let Some(at) = object.get_mut(bound) {
            retain(at, &["_type", "value"]);
        }
    }
    object.retain(|key, _| {
        [
            "_type",
            "lower",
            "upper",
            "lower_unbounded",
            "upper_unbounded",
            "lower_included",
            "upper_included",
        ]
        .contains(&key.as_str())
    });
}

/// A `DV_CODED_TEXT` by its code phrase alone, its value the code.
pub(super) fn normal_coded(coded: &Value) -> Value {
    let mut coded = coded.clone();
    retain(&mut coded, &["_type", "value", "defining_code"]);
    let code = coded
        .pointer("/defining_code/code_string")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = coded.as_object_mut() {
        object.insert("value".to_owned(), code);
    }
    if let Some(phrase) = coded.get_mut("defining_code") {
        retain(phrase, &["_type", "terminology_id", "code_string"]);
        if let Some(terminology) = phrase.get_mut("terminology_id") {
            retain(terminology, &["_type", "value"]);
        }
    }
    coded
}
