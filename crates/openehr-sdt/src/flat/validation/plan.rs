// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype-conformance walk plan: the template-static constraint paths
//! parsed once by [`prepare_walk`]. No openEHR spec governs it: our own design.

use openehr_rm::v1_2::paths::PathSegment;

use super::SiblingNameIndex;
use super::compiled::{Compiled, Compiler};
use crate::flat::rmpath;
use crate::flat::webtemplate::model::WebTemplateNode;

/// A pre-parsed walk plan for one [`WebTemplateNode`], each `Vec` index-aligned
/// with its constraint vector; `None` marks a path not under the node's own.
#[derive(Debug, Clone)]
pub(crate) struct NodeWalk {
    /// Child sibling groups, in first-seen order (grouping the node's `children`
    /// by `aqlPath`).
    pub(crate) child_groups: Vec<ChildGroup>,
    /// Parsed relative segments per `existence` constraint (index-aligned with
    /// [`WebTemplateNode::existence`]).
    pub(crate) existence: Vec<Option<Vec<PathSegment>>>,
    /// Parsed relative segments per `card_all` constraint (index-aligned with
    /// [`WebTemplateNode::card_all`]).
    pub(crate) cardinalities: Vec<Option<Vec<PathSegment>>>,
    /// Parsed relative segments per `closed_attributes` constraint (index-aligned
    /// with [`WebTemplateNode::closed_attributes`]).
    pub(crate) closed: Vec<Option<Vec<PathSegment>>>,
    /// Slot groups (grouped by slot path, first-seen order).
    pub(crate) slot_groups: Vec<SlotGroup>,
    /// Per-step id-only name fallback over every template path under this node
    /// ([`rmpath::NameFallback`]), so a step whose name conjunct a renamed
    /// instance does not match still reaches it when the name is redundant.
    pub(crate) name_fallback: rmpath::NameFallback,
    /// Each child's path relative to this node ([`rmpath::relative`]),
    /// index-aligned with [`WebTemplateNode::children`], for the FLAT build
    /// and flatten.
    pub(crate) child_segments: Vec<Vec<PathSegment>>,
    /// Per child, the template's record of each structural wrapper its
    /// relative path passes ([`crate::flat::build::wrapper_types`]); empty
    /// when the plan was built without the template's slots.
    pub(crate) child_wrappers: Vec<Vec<crate::flat::build::WrapperFacts>>,
    /// Per child, the [`path_fingerprint`] its cached path and wrappers were
    /// computed from, so a tree changed after preparation is read afresh.
    pub(crate) child_fingerprints: Vec<u64>,
    /// The [`slots_fingerprint`] of the template the wrappers were computed
    /// from.
    pub(crate) slots_fingerprint: u64,
    /// The constraint paths compiled for the conformance walk.
    pub(crate) compiled: Compiled,
}

impl NodeWalk {
    /// The cached path from `parent` (this plan's node) to its child `index`,
    /// when it was computed from the paths the two nodes carry now.
    pub(crate) fn segments_of(
        &self,
        parent: &WebTemplateNode,
        index: usize,
        child: &WebTemplateNode,
    ) -> Option<&[PathSegment]> {
        let fresh = self.child_fingerprints.get(index).copied()
            == Some(path_fingerprint(&parent.aql_path, &child.aql_path));
        fresh
            .then(|| self.child_segments.get(index).map(Vec::as_slice))
            .flatten()
    }

    /// The cached wrapper facts of child `index`, when computed from the
    /// paths the nodes carry now and from slots of `slots_fingerprint`.
    pub(crate) fn wrappers_of(
        &self,
        parent: &WebTemplateNode,
        index: usize,
        child: &WebTemplateNode,
        slots_fingerprint: u64,
    ) -> Option<&[crate::flat::build::WrapperFacts]> {
        (self.slots_fingerprint == slots_fingerprint
            && self.segments_of(parent, index, child).is_some())
        .then(|| self.child_wrappers.get(index).map(Vec::as_slice))
        .flatten()
    }
}

/// A fingerprint of a parent and child `aqlPath` pair.
pub(crate) fn path_fingerprint(parent: &str, child: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    parent.hash(&mut hasher);
    child.hash(&mut hasher);
    hasher.finish()
}

/// A fingerprint of every hoisted-wrapper slot of the tree under `node`, in
/// tree order.
pub(crate) fn slots_fingerprint(node: &WebTemplateNode) -> u64 {
    use std::hash::{Hash, Hasher};
    fn feed(node: &WebTemplateNode, hasher: &mut std::hash::DefaultHasher) {
        for slot in &node.slots {
            slot.path.hash(hasher);
            slot.rm_type.hash(hasher);
            slot.name.hash(hasher);
        }
        for child in &node.children {
            feed(child, hasher);
        }
    }
    let mut hasher = std::hash::DefaultHasher::new();
    feed(node, &mut hasher);
    hasher.finish()
}

/// A sibling group of `WebTemplate` children sharing one `aqlPath` (a single
/// node, or a set of polymorphic type alternatives).
#[derive(Debug, Clone)]
pub(crate) struct ChildGroup {
    /// Indices into [`WebTemplateNode::children`] of the group's members.
    pub(crate) members: Vec<usize>,
    /// The group's `aqlPath` parsed relative to the parent node's `aqlPath`; an
    /// empty vec means the child path is not a strict extension of the parent's
    /// (or equals it) — the walk skips such a group.
    pub(crate) segments: Vec<PathSegment>,
}

/// A group of hoisted-wrapper slot constraints sharing one absolute path.
#[derive(Debug, Clone)]
pub(crate) struct SlotGroup {
    /// The allowed RM types (first-seen order — the `join(", ")` message order).
    pub(crate) allowed: Vec<String>,
    /// The absolute slot path (the violation path the walk pushes).
    pub(crate) path: String,
    /// The slot path parsed relative to the node's `aqlPath` (empty = not an
    /// extension; the walk skips it).
    pub(crate) segments: Vec<PathSegment>,
}

impl NodeWalk {
    /// Build the walk plan for `node` from its (final, post-compaction) constraint
    /// vectors and children. Parses each constraint path once.
    #[expect(
        clippy::indexing_slicing,
        reason = "`gi` is either an index already recorded in the `child_idx`/`slot_idx` map or `next == len` immediately after the matching `push`, so both `child_groups[gi]` and `slot_groups[gi]` are in bounds by construction"
    )]
    pub(crate) fn build(node: &WebTemplateNode) -> Self {
        let aql = node.aql_path.as_str();

        // Child groups: iterate children in order, grouping by `aqlPath`.
        let mut child_groups: Vec<ChildGroup> = Vec::new();
        let mut child_idx: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for (i, child) in node.children.iter().enumerate() {
            let key = child.aql_path.as_str();
            let next = child_groups.len();
            let gi = *child_idx.entry(key).or_insert(next);
            if gi == next {
                let segments = key.strip_prefix(aql).map(rmpath::parse).unwrap_or_default();
                child_groups.push(ChildGroup {
                    members: Vec::new(),
                    segments,
                });
            }
            child_groups[gi].members.push(i);
        }
        let child_names =
            sibling_index_from_segments(child_groups.iter().map(|g| g.segments.as_slice()));

        // Slot groups: grouped by absolute slot path, first-seen order.
        let mut slot_groups: Vec<SlotGroup> = Vec::new();
        let mut slot_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for slot in &node.slots {
            let key = slot.path.as_str();
            let next = slot_groups.len();
            let gi = *slot_idx.entry(key).or_insert(next);
            if gi == next {
                let segments = key.strip_prefix(aql).map(rmpath::parse).unwrap_or_default();
                slot_groups.push(SlotGroup {
                    allowed: Vec::new(),
                    path: slot.path.clone(),
                    segments,
                });
            }
            slot_groups[gi].allowed.push(slot.rm_type.clone());
        }
        let slot_names =
            sibling_index_from_segments(slot_groups.iter().map(|g| g.segments.as_slice()));
        let name_fallback = rmpath::name_fallback_from_segments(
            child_groups
                .iter()
                .map(|g| g.segments.as_slice())
                .chain(slot_groups.iter().map(|g| g.segments.as_slice())),
        );

        let rel = |p: &str| p.strip_prefix(aql).map(rmpath::parse);
        let existence: Vec<_> = node.existence.iter().map(|e| rel(&e.path)).collect();
        let cardinalities: Vec<_> = node.card_all.iter().map(|c| rel(&c.path)).collect();
        let closed: Vec<_> = node
            .closed_attributes
            .iter()
            .map(|c| rel(&c.path))
            .collect();

        let mut paths = Compiler::new(&name_fallback);
        for group in &child_groups {
            let first = group.members.first().and_then(|&i| node.children.get(i));
            let first_type = first.map_or("", |c| c.rm_type.as_str());
            paths.group(&group.segments, &child_names, first_type);
        }
        for group in &slot_groups {
            paths.slot(&group.segments, &slot_names);
        }
        let (on_existence, on_cardinalities, on_closed) = (
            paths.constraints(&existence),
            paths.constraints(&cardinalities),
            paths.constraints(&closed),
        );
        let compiled = Compiled {
            existence: on_existence,
            cardinalities: on_cardinalities,
            closed: on_closed,
            ..paths.finish()
        };

        Self {
            child_groups,
            existence,
            cardinalities,
            closed,
            slot_groups,
            name_fallback,
            child_segments: node
                .children
                .iter()
                .map(|c| rmpath::relative(aql, &c.aql_path))
                .collect(),
            child_wrappers: Vec::new(),
            child_fingerprints: node
                .children
                .iter()
                .map(|c| path_fingerprint(aql, &c.aql_path))
                .collect(),
            slots_fingerprint: 0,
            compiled,
        }
    }
}

/// Build the name-based sibling routing index from parsed relative sibling
/// paths: identities carried by more than one sibling, with their explicit
/// `name/value` constraints.
fn sibling_index_from_segments<'a>(
    groups: impl Iterator<Item = &'a [PathSegment]>,
) -> SiblingNameIndex {
    let mut acc: std::collections::HashMap<(String, String), (u32, Vec<String>)> =
        std::collections::HashMap::new();
    for segs in groups {
        let Some(id_seg) = segs.iter().rfind(|s| !s.predicate.is_empty()) else {
            continue;
        };
        let Some(id) = &id_seg.predicate.archetype_node_id else {
            continue;
        };
        let entry = acc
            .entry((id_seg.attribute.clone(), id.clone()))
            .or_default();
        entry.0 += 1;
        if let Some(name) = &id_seg.predicate.name_value
            && !entry.1.contains(name)
        {
            entry.1.push(name.clone());
        }
    }
    acc.into_iter()
        .filter(|(_, (count, _))| *count > 1)
        .map(|(k, (_, names))| (k, names))
        .collect()
}

/// Populate the walk plan on `node` and every descendant.
pub(crate) fn prepare_walk(node: &mut WebTemplateNode) {
    let mut slots = Vec::new();
    crate::flat::build::collect_slot_types(node, &mut slots);
    let fingerprint = slots_fingerprint(node);
    prepare(node, &slots, fingerprint);
}

/// [`prepare_walk`] under the template's hoisted wrappers `slots`, whose
/// [`slots_fingerprint`] is `fingerprint`.
fn prepare(node: &mut WebTemplateNode, slots: &[crate::flat::build::SlotType], fingerprint: u64) {
    let mut plan = NodeWalk::build(node);
    plan.slots_fingerprint = fingerprint;
    plan.child_wrappers = node
        .children
        .iter()
        .zip(&plan.child_segments)
        .map(|(child, rel)| crate::flat::build::wrapper_types(&child.aql_path, rel.len(), slots))
        .collect();
    node.walk = Some(Box::new(plan));
    for child in node
        .children
        .iter_mut()
        .chain(node.alternatives.iter_mut())
        .chain(node.null_constraints.iter_mut())
    {
        prepare(child, slots, fingerprint);
    }
}
