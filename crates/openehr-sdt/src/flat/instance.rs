// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One read-only view of an RM instance for the template-driven walks
//! (#6002): the archetype-conformance pass and the path navigation it shares
//! with the Simplified Formats.
//!
//! A committed document is typed RM ([`RmRef`], through the reflective accessor
//! of `openehr_rm::v1_2::access`); a `553|incomplete|` commit cannot be read
//! into strict typed RM and stays a canonical-JSON value (`&Value`). Both
//! implement [`Instance`], so each walk is written once. The path step
//! ([`select_children`]) is BASE `master11-paths` over that view, the same
//! rule `openehr_rm::v1_2::paths::select_children` applies to a JSON value.

#![expect(
    clippy::disallowed_types,
    reason = "the `553|incomplete|` commit keeps its canonical-JSON value (ITS-JSON) — the one \
              Value form of an RM instance the rewrite keeps (#1694, #6002)"
)]
// NOTE: the walks adopt this view in this same rewrite (rmpath, the archetype-conformance pass).
#![expect(
    dead_code,
    reason = "adopted by rmpath and flat::validation in the core rewrite (#6002)"
)]

use openehr_rm::v1_2::access::RmRef;
use openehr_rm::v1_2::paths::{PathSegment, Predicate};
use serde_json::Value;

/// A read-only view of one RM instance node or attribute value.
pub(crate) trait Instance<'a>: Copy {
    /// The node's RM class: the wire `_type` of a JSON value, the concrete
    /// class of a typed one.
    fn rm_type(self) -> Option<&'a str>;
    /// The value of the single-valued attribute `name`; `None` when absent,
    /// null, or a container.
    fn attr(self, name: &str) -> Option<Self>;
    /// The members of the container attribute `name`, in order; `None` when
    /// absent or single-valued.
    fn members(self, name: &str) -> Option<Vec<Self>>;
    /// Whether attribute `name` holds a value.
    fn is_present(self, name: &str) -> bool;
    /// Whether this is an RM object (not a primitive).
    fn is_object(self) -> bool;
    /// This value as a string.
    fn as_str(self) -> Option<&'a str>;
    /// This value as a number.
    fn as_f64(self) -> Option<f64>;
    /// This value as an integer.
    fn as_i64(self) -> Option<i64>;
    /// This value as a boolean.
    fn as_bool(self) -> Option<bool>;

    /// The string at attribute `name`.
    fn str_at(self, name: &str) -> Option<&'a str> {
        self.attr(name).and_then(Instance::as_str)
    }

    /// The objects under attribute `name`: the members of a container, or the
    /// single object.
    fn objects(self, name: &str) -> Vec<Self> {
        match self.members(name) {
            Some(members) => members.into_iter().filter(|m| m.is_object()).collect(),
            None => self
                .attr(name)
                .filter(|v| v.is_object())
                .into_iter()
                .collect(),
        }
    }
}

impl<'a> Instance<'a> for &'a Value {
    fn rm_type(self) -> Option<&'a str> {
        self.get("_type").and_then(Value::as_str)
    }
    fn attr(self, name: &str) -> Option<Self> {
        self.get(name).filter(|v| !v.is_null() && !v.is_array())
    }
    fn members(self, name: &str) -> Option<Vec<Self>> {
        self.get(name)
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
    }
    fn is_present(self, name: &str) -> bool {
        self.get(name).is_some_and(|v| !v.is_null())
    }
    fn is_object(self) -> bool {
        Value::is_object(self)
    }
    fn as_str(self) -> Option<&'a str> {
        Value::as_str(self)
    }
    fn as_f64(self) -> Option<f64> {
        Value::as_f64(self)
    }
    fn as_i64(self) -> Option<i64> {
        Value::as_i64(self)
    }
    fn as_bool(self) -> Option<bool> {
        Value::as_bool(self)
    }
}

impl<'a> Instance<'a> for RmRef<'a> {
    fn rm_type(self) -> Option<&'a str> {
        match self {
            RmRef::Object(o) => Some(o.rm_type()),
            _ => None,
        }
    }
    fn attr(self, name: &str) -> Option<Self> {
        match self {
            RmRef::Object(o) => o.attribute(name),
            _ => None,
        }
    }
    fn members(self, name: &str) -> Option<Vec<Self>> {
        match self {
            RmRef::Object(o) => o
                .items(name)
                .map(|items| items.map(RmRef::Object).collect()),
            _ => None,
        }
    }
    fn is_present(self, name: &str) -> bool {
        match self {
            RmRef::Object(o) => o.is_set(name),
            _ => false,
        }
    }
    fn is_object(self) -> bool {
        matches!(self, RmRef::Object(_))
    }
    fn as_str(self) -> Option<&'a str> {
        match self {
            RmRef::String(s) => Some(s),
            _ => None,
        }
    }
    fn as_f64(self) -> Option<f64> {
        match self {
            RmRef::Real(r) => Some(r),
            #[expect(
                clippy::as_conversions,
                clippy::cast_precision_loss,
                reason = "a JSON number reads an integer as f64 the same way (serde_json `as_f64`)"
            )]
            RmRef::Integer(i) => Some(i as f64),
            _ => None,
        }
    }
    fn as_i64(self) -> Option<i64> {
        match self {
            RmRef::Integer(i) => Some(i),
            _ => None,
        }
    }
    fn as_bool(self) -> Option<bool> {
        match self {
            RmRef::Boolean(b) => Some(b),
            _ => None,
        }
    }
}

/// Whether `node` satisfies the attribute-based conjuncts of `predicate`
/// (`archetype_node_id`, `name/value`, `uid` and the general comparisons),
/// as `Predicate::matches` judges a JSON node.
pub(crate) fn matches<'a, I: Instance<'a>>(predicate: &Predicate, node: I) -> bool {
    if let Some(id) = predicate.archetype_node_id.as_deref()
        && node.str_at("archetype_node_id") != Some(id)
    {
        return false;
    }
    if let Some(name) = predicate.name_value.as_deref()
        && node.attr("name").and_then(|n| n.str_at("value")) != Some(name)
    {
        return false;
    }
    if let Some(uid) = predicate.uid.as_deref() {
        let have = match node.attr("uid") {
            Some(u) if u.is_object() => u.str_at("value"),
            Some(u) => u.as_str(),
            None => None,
        };
        if have != Some(uid) {
            return false;
        }
    }
    predicate.comparisons.iter().all(|c| {
        let mut leaves = Vec::new();
        collect_leaves(node, &c.path, &mut leaves);
        leaves.into_iter().any(|leaf| {
            let leaf = if leaf.is_object() {
                match leaf.attr("value") {
                    Some(v) => v,
                    None => return false,
                }
            } else {
                leaf
            };
            c.satisfied_by_scalar(leaf.as_str(), leaf.as_f64())
        })
    })
}

/// The values a comparison path selects below `node`, through containers.
fn collect_leaves<'a, I: Instance<'a>>(node: I, path: &[String], out: &mut Vec<I>) {
    let Some((first, rest)) = path.split_first() else {
        out.push(node);
        return;
    };
    let next: Vec<I> = match node.members(first) {
        Some(members) => members,
        None => node.attr(first).into_iter().collect(),
    };
    for n in next {
        collect_leaves(n, rest, out);
    }
}

/// One path step from `node`: the members of a container attribute the
/// segment's predicate selects (its position, if any, counted from 1), or
/// the single value when it matches (a single value is at position 1).
pub(crate) fn select_children<'a, I: Instance<'a>>(node: I, segment: &PathSegment) -> Vec<I> {
    let predicate = &segment.predicate;
    if let Some(items) = node.members(&segment.attribute) {
        return match predicate.position {
            Some(pos) => pos
                .checked_sub(1)
                .and_then(|i| items.get(i).copied())
                .filter(|v| matches(predicate, *v))
                .into_iter()
                .collect(),
            None => items
                .into_iter()
                .filter(|v| matches(predicate, *v))
                .collect(),
        };
    }
    let Some(child) = node.attr(&segment.attribute) else {
        return Vec::new();
    };
    if predicate.position.is_some_and(|p| p != 1) || !matches(predicate, child) {
        return Vec::new();
    }
    vec![child]
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_rm::v1_2::access::as_rm_object;
    use openehr_rm::v1_2::paths::RmPath;

    fn composition() -> (Value, openehr_rm::prelude::Composition) {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json"
        ))
        .expect("the IPS fixture should be readable");
        let typed: openehr_rm::prelude::Composition =
            openehr_its::json::from_canonical_json(&text).expect("the IPS fixture should read");
        (openehr_its::json::to_canonical_value(&typed), typed)
    }

    /// Every step the JSON path rule takes, the typed view takes to the same
    /// nodes, on every path the IPS composition holds.
    #[test]
    fn a_step_selects_what_the_json_step_selects() {
        let (value, typed) = composition();
        let paths = [
            "content",
            "content[openEHR-EHR-SECTION.ips_medication.v0]",
            "content/items",
            "content/items/data/events",
            "content/items/data/events/data/items",
            "context/other_context/items",
            "composer",
            "category",
            "content[2]",
        ];
        for p in paths {
            let parsed: RmPath = p.parse().expect("parses");
            let mut json: Vec<&Value> = vec![&value];
            let mut rm: Vec<RmRef<'_>> = vec![RmRef::Object(as_rm_object(&typed))];
            for seg in &parsed.segments {
                json = json
                    .iter()
                    .flat_map(|n| openehr_rm::v1_2::paths::select_children(n, seg))
                    .collect();
                rm = rm.iter().flat_map(|n| select_children(*n, seg)).collect();
                let json_ids: Vec<Option<&str>> = json
                    .iter()
                    .map(|n| n.get("archetype_node_id").and_then(Value::as_str))
                    .collect();
                let rm_ids: Vec<Option<&str>> =
                    rm.iter().map(|n| n.str_at("archetype_node_id")).collect();
                assert_eq!(json_ids, rm_ids, "{p}");
            }
        }
    }
}
