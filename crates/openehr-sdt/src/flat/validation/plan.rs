// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pre-parsed archetype-conformance walk plan.
//!
//! The archetype-conformance walk ([`super::validate_archetype_conformance`])
//! is guided by the constraint paths a [`WebTemplateNode`] carries, which are
//! template-static: [`prepare_walk`] parses them once at build time into the
//! cached [`WebTemplateNode::walk`]; a node without a cached plan gets an
//! identical [`NodeWalk`] built per visit, so there is one code path. No
//! openEHR spec governs the WebTemplate model or this plan: our own design.

use openehr_rm::v1_2::paths::PathSegment;

use super::SiblingNameIndex;
use crate::flat::rmpath;
use crate::flat::webtemplate::model::WebTemplateNode;

/// A pre-parsed archetype-conformance walk plan for one [`WebTemplateNode`].
///
/// Every `Vec` is index-aligned with the corresponding node constraint vector so
/// the walk can zip them; `None` marks a constraint path that is not an
/// extension of the node's own `aqlPath`.
#[derive(Debug, Clone)]
pub(crate) struct NodeWalk {
    /// Child sibling groups, in first-seen order (grouping the node's `children`
    /// by `aqlPath`).
    pub(crate) child_groups: Vec<ChildGroup>,
    /// Name-based sibling routing index over the child groups.
    pub(crate) child_names: SiblingNameIndex,
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
    /// Name-based sibling routing index over the slot groups.
    pub(crate) slot_names: SiblingNameIndex,
    /// Per-step id-only name fallback over every template path under this node
    /// ([`rmpath::NameFallback`]), so a step whose name conjunct a renamed
    /// instance does not match still reaches it when the name is redundant.
    pub(crate) name_fallback: rmpath::NameFallback,
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

        Self {
            child_groups,
            child_names,
            existence: node.existence.iter().map(|e| rel(&e.path)).collect(),
            cardinalities: node.card_all.iter().map(|c| rel(&c.path)).collect(),
            closed: node
                .closed_attributes
                .iter()
                .map(|c| rel(&c.path))
                .collect(),
            slot_groups,
            slot_names,
            name_fallback,
        }
    }
}

/// Build the name-based sibling routing index from a set of already-parsed
/// relative sibling paths (one per distinct child/slot `aqlPath`).
///
/// Per identity `(attribute, archetype_node_id)`, count the distinct sibling
/// paths carrying it and collect the explicit `name/value` constraints; keep only
/// identities carried by more than one sibling. A path with no predicate-bearing
/// segment, or no `archetype_node_id`, is skipped.
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

/// Populate the archetype-conformance walk plan on `node` and, recursively, every
/// descendant — called once by [`crate::flat::webtemplate::builder::build_web_template`] on the
/// finalized tree so the validation walk never re-parses a template-static
/// constraint path.
pub(crate) fn prepare_walk(node: &mut WebTemplateNode) {
    let plan = NodeWalk::build(node);
    node.walk = Some(Box::new(plan));
    for child in node
        .children
        .iter_mut()
        .chain(node.alternatives.iter_mut())
        .chain(node.null_constraints.iter_mut())
    {
        prepare_walk(child);
    }
}
