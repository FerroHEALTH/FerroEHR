// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The example of a `DV_INTERVAL<DV_QUANTITY>`: both limits in one unit the
//! constraints on both admit, with one magnitude inside every range, so the
//! interval meets BASE `Interval` `Limits_comparable` and `Limits_consistent`.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::{Map, Value, json};

use super::{input_with_suffix, pick_decimal, put};
use crate::flat::webtemplate::model::{WebTemplateNode, WebTemplateRange};

/// The node whose example fills `child`: a `DV_INTERVAL` part the template
/// leaves open takes its constrained sibling's example, so the limits agree
/// (RM `data_types` §DV_INTERVAL `Limits_consistent`).
pub(super) fn part_source<'a>(
    node: &'a WebTemplateNode,
    child: &'a WebTemplateNode,
) -> &'a WebTemplateNode {
    let open = child.implicit && node.rm_type.starts_with("DV_INTERVAL<");
    let sibling = || {
        node.children
            .iter()
            .find(|s| !s.implicit && s.rm_type == child.rm_type)
    };
    open.then(sibling).flatten().unwrap_or(child)
}

/// Emits the limit `child` of the interval `node` as the shared limit both
/// limits take, reporting whether it did.
pub(super) fn emit_shared_limit(
    node: &WebTemplateNode,
    child: &WebTemplateNode,
    prefix: &str,
    out: &mut Map<String, Value>,
) -> bool {
    let Some((magnitude, unit)) = shared_limit(node).filter(|_| child.rm_type == "DV_QUANTITY")
    else {
        return false;
    };
    put(out, prefix, "magnitude", json!(magnitude));
    put(out, prefix, "unit", json!(unit));
    true
}

/// The `(magnitude, unit)` both limits of the interval `node` take, or `None`
/// when `node` is no `DV_INTERVAL<DV_QUANTITY>` or its limits share no unit.
fn shared_limit(node: &WebTemplateNode) -> Option<(f64, String)> {
    if !node.rm_type.starts_with("DV_INTERVAL<DV_QUANTITY") {
        return None;
    }
    let limits: Vec<&WebTemplateNode> = node
        .children
        .iter()
        .filter(|c| c.rm_type == "DV_QUANTITY")
        .collect();
    let constrained: Vec<&WebTemplateNode> = limits
        .iter()
        .copied()
        .filter(|l| !units_of(l).is_empty())
        .collect();
    let first = constrained.first()?;
    let unit = units_of(first)
        .into_iter()
        .find(|u| constrained.iter().all(|l| units_of(l).contains(u)))?;
    let ranges: Vec<&WebTemplateRange> = constrained
        .iter()
        .filter_map(|l| range_for(l, &unit))
        .collect();
    Some((pick_decimal(intersect(&ranges).as_ref()), unit))
}

/// The units a limit's constraint lists.
fn units_of(limit: &WebTemplateNode) -> Vec<String> {
    input_with_suffix(limit, "unit")
        .map(|i| i.list.iter().map(|cv| cv.value.clone()).collect())
        .unwrap_or_default()
}

/// The magnitude range a limit's constraint gives `unit`.
fn range_for<'a>(limit: &'a WebTemplateNode, unit: &str) -> Option<&'a WebTemplateRange> {
    input_with_suffix(limit, "unit")?
        .list
        .iter()
        .find(|cv| cv.value == unit)
        .and_then(|cv| cv.validation.as_ref())
        .and_then(|v| v.range.as_ref())
}

/// The range every one of `ranges` admits: the greatest lower bound and the
/// least upper bound, with the operator of the bound kept.
fn intersect(ranges: &[&WebTemplateRange]) -> Option<WebTemplateRange> {
    let bound = |r: &&WebTemplateRange, upper: bool| {
        let (value, op) = if upper {
            (&r.max, &r.max_op)
        } else {
            (&r.min, &r.min_op)
        };
        value
            .as_ref()
            .and_then(Value::as_f64)
            .map(|v| (v, op.clone()))
    };
    let pick = |upper: bool| {
        ranges
            .iter()
            .filter_map(|r| bound(r, upper))
            .reduce(|a, b| match (upper, a.0 < b.0) {
                (true, true) | (false, false) => a,
                _ => b,
            })
    };
    let (min, max) = (pick(false), pick(true));
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op: min.as_ref().and_then(|m| m.1.clone()),
        min: min.map(|m| Value::from(m.0)),
        max_op: max.as_ref().and_then(|m| m.1.clone()),
        max: max.map(|m| Value::from(m.0)),
    })
}
