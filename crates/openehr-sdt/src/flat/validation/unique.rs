// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `unique` cardinality modifier: no two members of a container may be
//! equal by value (ADL 1.4 `master05-cadl.adoc` and ADL2
//! `master04.3-cadl_complex_types.adoc` §Container attributes, SPECAM-111).
//!
//! The text excludes object identity and "attributes that distinguish sibling
//! objects anyway, such as their names". Two attributes of the RM are of that
//! kind: `LOCATABLE.name` on the member itself, the key of the name-based path
//! predicate that tells siblings apart (BASE `architecture_overview/
//! master11-paths.adoc` §Name-based Predicate), and `LOCATABLE.uid` at any
//! depth, an identifier of the node (RM `common/master03-archetyped_package.adoc`
//! §LOCATABLE). Every other attribute, `archetype_node_id` included, is part of
//! the value. Numbers compare by value, so `1` equals `1.0`.

#![expect(
    clippy::disallowed_types,
    reason = "value equality compares canonical-JSON members as the RM-instance validation \
              reads them (#1694)"
)]

use std::collections::BTreeMap;

use serde_json::Value;

/// The positions of the first two members that are equal by value, if any.
pub(super) fn first_duplicate(members: &[&Value]) -> Option<(usize, usize)> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, member) in members.iter().enumerate() {
        let key = comparable(member, true).to_string();
        if let Some(&first) = seen.get(&key) {
            return Some((first, index));
        }
        seen.insert(key, index);
    }
    None
}

/// `value` with the attributes that never take part in value equality
/// removed, its object members in key order and its numbers normalised.
fn comparable(value: &Value, member: bool) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map
                .keys()
                .filter(|k| k.as_str() != "uid" && !(member && k.as_str() == "name"))
                .collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(key) {
                    out.insert(key.clone(), comparable(v, false));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| comparable(v, false)).collect()),
        Value::Number(n) => number(n),
        other => other.clone(),
    }
}

/// A number in one form per value: every number f64 holds exactly, integers
/// below 2^53 among them, becomes that float, so `1` and `1.0` compare equal.
fn number(n: &serde_json::Number) -> Value {
    match n.as_f64() {
        Some(f) if f.abs() < 9.0e15 => Value::from(f),
        _ => Value::Number(n.clone()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::first_duplicate;

    fn element(name: &str, magnitude: &serde_json::Value) -> serde_json::Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at0004",
               "name": {"_type": "DV_TEXT", "value": name},
               "value": {"_type": "DV_QUANTITY", "magnitude": magnitude, "units": "mm[Hg]"}})
    }

    #[test]
    fn members_differing_only_by_name_and_uid_are_equal() {
        let mut second = element("Systolic #2", &json!(120.0));
        second["uid"] =
            json!({"_type": "HIER_OBJECT_ID", "value": "9a1b0b4e-7a5c-4b1e-9d3c-1f2e3d4c5b6a"});
        let first = element("Systolic", &json!(120));
        assert_eq!(first_duplicate(&[&first, &second]), Some((0, 1)));
    }

    #[test]
    fn a_different_value_or_node_id_keeps_members_apart() {
        let first = element("Systolic", &json!(120));
        let other_value = element("Systolic", &json!(121));
        let mut other_node = element("Systolic", &json!(120));
        other_node["archetype_node_id"] = json!("at0005");
        assert_eq!(first_duplicate(&[&first, &other_value, &other_node]), None);
    }

    #[test]
    fn a_nested_name_is_part_of_the_value() {
        let cluster = |inner: &str| {
            json!({"_type": "CLUSTER", "archetype_node_id": "at0001",
                   "name": {"_type": "DV_TEXT", "value": "Device"},
                   "items": [element(inner, &json!(1))]})
        };
        assert_eq!(first_duplicate(&[&cluster("A"), &cluster("B")]), None);
        assert_eq!(
            first_duplicate(&[&cluster("A"), &cluster("A")]),
            Some((0, 1))
        );
    }
}
