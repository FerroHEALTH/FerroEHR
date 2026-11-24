// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM-mandatory structural attributes a node's folded descendants get from
//! the structural stubs the compactor re-homed onto it.

use openehr_rm::v1_2::access::RmObject;
use openehr_rm::v1_2::paths::PathSegment;

use crate::flat::error::FlatError;
use crate::flat::instance::matches;
use std::borrow::Cow;

use crate::flat::typed::{Build, o, view};
use crate::flat::validation::plan::stub_segments;
use crate::flat::webtemplate::model::WebTemplateNode;

/// Fills an RM-mandatory structural attribute of `node` or a folded descendant
/// with the node its template constrains (AOM 1.4 §`Valid_value`).
pub(crate) fn fill_stubbed_descendants(
    obj: &mut dyn RmObject,
    node: &WebTemplateNode,
) -> Result<(), FlatError> {
    for (index, stub) in node.structural_stubs.iter().enumerate() {
        let cached = node
            .walk
            .as_ref()
            .and_then(|plan| plan.stub_segments_of(node, index, &stub.path));
        let segments = match cached {
            Some(segments) => Some(Cow::Borrowed(segments)),
            None => stub_segments(&node.aql_path, &stub.path, &stub.attr).map(Cow::Owned),
        };
        let Some(segments) = segments else {
            continue;
        };
        let mut owners = Vec::new();
        objects_at_mut(&mut *obj, &segments, &mut owners);
        for o_ in owners {
            let ty = o_.rm_type();
            let rm_mandatory = match stub.attr.as_str() {
                "data" => matches!(ty, "POINT_EVENT" | "INTERVAL_EVENT" | "EVENT"),
                "description" => ty == "ACTIVITY",
                _ => false,
            };
            if rm_mandatory && !o_.is_set(&stub.attr) {
                o_.put(
                    &stub.attr,
                    o(crate::flat::build::structural_from(stub, "Tree")?),
                )?;
            }
        }
    }
    Ok(())
}

/// Collects the objects `segments` reach from `obj`, each step through the
/// members (or the single value) of its attribute that the step's predicate
/// matches (BASE `master11-paths` §"Predicate Expressions").
fn objects_at_mut<'a>(
    obj: &'a mut dyn RmObject,
    segments: &[PathSegment],
    out: &mut Vec<&'a mut dyn RmObject>,
) {
    let Some((segment, rest)) = segments.split_first() else {
        out.push(obj);
        return;
    };
    let children: Vec<&'a mut dyn RmObject> = if obj.items(&segment.attribute).is_some() {
        obj.items_mut(&segment.attribute)
            .map(Iterator::collect)
            .unwrap_or_default()
    } else {
        obj.object_mut(&segment.attribute).into_iter().collect()
    };
    for child in children {
        if matches(&segment.predicate, view(&*child)) {
            objects_at_mut(child, rest, out);
        }
    }
}
