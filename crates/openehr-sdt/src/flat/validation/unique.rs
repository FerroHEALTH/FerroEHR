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

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::flat::instance::Instance;

/// The positions of the first two members that are equal by value, if any.
pub(super) fn first_duplicate<'i, I: Instance<'i>>(members: &[I]) -> Option<(usize, usize)> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, member) in members.iter().enumerate() {
        let mut key = String::new();
        comparable(*member, true, &mut key);
        if let Some(&first) = seen.get(&key) {
            return Some((first, index));
        }
        seen.insert(key, index);
    }
    None
}

/// A text that is equal for two values exactly when they are equal by value:
/// attributes in name order, `uid` left out at any depth and `name` on the
/// member itself, numbers by their value.
fn comparable<'i, I: Instance<'i>>(value: I, member: bool, out: &mut String) {
    if !value.is_object() {
        match value.as_f64() {
            Some(f) if f.abs() < 9.0e15 => {
                let _ = write!(out, "{f:?}");
            }
            _ => out.push_str(&value.render()),
        }
        return;
    }
    out.push('{');
    if let Some(t) = value.rm_type() {
        let _ = write!(out, "_type={t:?};");
    }
    let mut keys: Vec<&str> = value
        .keys()
        .into_iter()
        .filter(|k| *k != "uid" && !(member && *k == "name"))
        .collect();
    keys.sort_unstable();
    for key in keys {
        let _ = write!(out, "{key:?}=");
        if let Some(items) = value.members(key) {
            out.push('[');
            for item in items {
                comparable(item, false, out);
                out.push(',');
            }
            out.push(']');
        } else if let Some(v) = value.attr(key) {
            comparable(v, false, out);
        }
        out.push(';');
    }
    out.push('}');
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
