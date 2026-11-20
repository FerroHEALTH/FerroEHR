// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a settings class keeps of the settings: the members the decision
//! reads, every coded value by its code phrase alone, and each readable
//! interval by its bounds. No openEHR spec governs the classes: our own
//! design (#5588).

use serde_json::Value;

use super::{normal_coded, retain, set};
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
