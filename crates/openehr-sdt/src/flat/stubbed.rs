// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM-mandatory structural attributes a node's folded descendants get from
//! the structural stubs the compactor re-homed onto it.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use openehr_rm::v1_2::paths::PathSegment;
use serde_json::{Map, Value};

use crate::flat::rmpath;
use crate::flat::webtemplate::model::WebTemplateNode;

/// Fills an RM-mandatory structural attribute of `node` or of a descendant the
/// compactor folded into it (an EVENT's `data`, an ACTIVITY's `description`)
/// with the node its template constrains, where the build left it out.
///
/// AOM 1.4 `master04-constraint_model_package.adoc` §`Valid_value`: a filler
/// must carry the constrained `archetype_node_id`, so the RM-default `at0001`
/// placeholder `complete_tree` writes is left for an unconstrained attribute.
pub(crate) fn fill_stubbed_descendants(obj: &mut Map<String, Value>, node: &WebTemplateNode) {
    for stub in &node.structural_stubs {
        let Some(owner) = stub
            .path
            .strip_suffix(stub.attr.as_str())
            .and_then(|p| p.strip_suffix('/'))
        else {
            continue;
        };
        let segments = rmpath::relative(&node.aql_path, owner);
        let mut owners = Vec::new();
        objects_at_mut(obj, &segments, &mut owners);
        for o in owners {
            let Some(ty) = o.get("_type").and_then(Value::as_str) else {
                continue;
            };
            let rm_mandatory = match stub.attr.as_str() {
                "data" => matches!(ty, "POINT_EVENT" | "INTERVAL_EVENT" | "EVENT"),
                "description" => ty == "ACTIVITY",
                _ => false,
            };
            if rm_mandatory && !o.contains_key(&stub.attr) {
                o.insert(
                    stub.attr.clone(),
                    crate::flat::build::structural_from(stub, "Tree"),
                );
            }
        }
    }
}

/// Every object `segments` reach from `obj`, each step keeping the children
/// whose node id and name the segment's predicate matches.
fn objects_at_mut<'a>(
    obj: &'a mut Map<String, Value>,
    segments: &[PathSegment],
    out: &mut Vec<&'a mut Map<String, Value>>,
) {
    let Some((segment, rest)) = segments.split_first() else {
        out.push(obj);
        return;
    };
    let children: Vec<&mut Value> = match obj.get_mut(&segment.attribute) {
        Some(Value::Array(items)) => items.iter_mut().collect(),
        Some(single) => vec![single],
        None => Vec::new(),
    };
    for child in children {
        if segment.predicate.matches(child)
            && let Value::Object(m) = child
        {
            objects_at_mut(m, rest, out);
        }
    }
}
