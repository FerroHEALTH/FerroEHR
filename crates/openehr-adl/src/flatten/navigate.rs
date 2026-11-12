// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Differential-path navigation over a flat definition, with proxy expansion.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use super::anchor_matches;
use crate::aom::access::object_node_id;
use crate::paths::{PathSegment, parse_path};

/// Navigate `root` to the object node addressed by `path`, expanding any proxy
/// (`C_COMPLEX_OBJECT_PROXY`) encountered on the way to an inline copy of its
/// target (`master09.05` §Internal Reference (Proxy Object) Redefinition — a
/// proxy overridden through its path is expanded inline). Returns a mutable
/// reference to the target complex object, or `None` if the path does not
/// resolve.
pub(super) fn resolve_object_mut<'a>(
    root: &'a mut CComplexObject,
    path: &str,
) -> Option<&'a mut CComplexObjectData> {
    // Resolve a fresh snapshot of proxy targets before mutating (proxy expansion
    // reads the current tree).
    let snapshot = root.clone();
    match root {
        CComplexObject::CComplexObject(d) => resolve_data_mut(d, &snapshot, path),
        // The flattener only navigates plain complex objects.
        CComplexObject::CArchetypeRoot(_) => None,
    }
}

/// Navigate from `start` to the object node addressed by `path`, expanding any
/// proxy on the way against `snapshot` (the tree `start` belongs to, read
/// before mutation), as [`resolve_object_mut`] does from the root.
#[expect(
    clippy::indexing_slicing,
    reason = "`attr_pos` is an `Iterator::position` over `current.attributes` and `child_pos` a `pick_child_pos` result over that attribute's `children`, so both are in bounds by construction; a `get_mut` rewrite would split the reborrow chain this `&mut` navigation loop depends on"
)]
pub(super) fn resolve_data_mut<'a>(
    start: &'a mut CComplexObjectData,
    snapshot: &CComplexObject,
    path: &str,
) -> Option<&'a mut CComplexObjectData> {
    let segments = parse_path(path);
    let mut current = start;
    for seg in &segments {
        let attr_pos = current
            .attributes
            .iter()
            .flatten()
            .position(|a| a.rm_attribute_name == seg.attribute)?;
        let attributes = current.attributes.as_mut()?;
        let child_pos = pick_child_pos(&attributes[attr_pos], seg)?;
        // Proxy expansion: if the picked child is a proxy, replace it inline
        // with a copy of its target structure resolved in the snapshot.
        let children = attributes[attr_pos].children.as_mut()?;
        if let CObject::CComplexObjectProxy(proxy) = &children[child_pos] {
            children[child_pos] = expand_proxy(snapshot, proxy)?;
        }
        current = match &mut children[child_pos] {
            CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d,
            _ => return None,
        };
    }
    Some(current)
}

/// Expand a proxy node to an inline `C_COMPLEX_OBJECT` copy of its target
/// (resolved against `tree`), preserving the proxy's node id / occurrences.
fn expand_proxy(
    tree: &CComplexObject,
    proxy: &openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy,
) -> Option<CObject> {
    let target = resolve_object_ref(tree, &proxy.target_path)?;
    let mut out = target.clone();
    out.node_id.clone_from(&proxy.node_id);
    if proxy.occurrences.is_some() {
        out.occurrences.clone_from(&proxy.occurrences);
    }
    out.sibling_order = None;
    Some(CObject::CComplexObject(CComplexObject::CComplexObject(out)))
}

/// Read-only path resolution to a complex object (used for proxy-target lookup).
fn resolve_object_ref<'a>(root: &'a CComplexObject, path: &str) -> Option<&'a CComplexObjectData> {
    let segments = parse_path(path);
    let mut current: &CComplexObjectData = match root {
        CComplexObject::CComplexObject(d) => d,
        CComplexObject::CArchetypeRoot(_) => return None,
    };
    for seg in &segments {
        let attr = current
            .attributes
            .iter()
            .flatten()
            .find(|a| a.rm_attribute_name == seg.attribute)?;
        let child = pick_child_ref(attr, seg)?;
        current = match child {
            CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d,
            _ => return None,
        };
    }
    Some(current)
}

fn pick_child_pos(attr: &CAttribute, seg: &PathSegment) -> Option<usize> {
    match &seg.node_id {
        Some(nid) => attr
            .children
            .iter()
            .flatten()
            .position(|c| object_node_id(c) == nid || anchor_matches(object_node_id(c), nid)),
        None if attr.children.as_ref().map_or(0, Vec::len) == 1 => Some(0),
        None => None,
    }
}

fn pick_child_ref<'a>(attr: &'a CAttribute, seg: &PathSegment) -> Option<&'a CObject> {
    pick_child_pos(attr, seg).and_then(|i| attr.children.as_ref()?.get(i))
}
