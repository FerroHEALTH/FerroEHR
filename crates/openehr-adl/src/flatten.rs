// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Specialisation flattening — overlay a differential child archetype onto its
//! flat parent to produce the flat form.
//!
//! Oracle: `docs/specs/openehr/AM/docs/AOM2/master08-validation.adoc`
//! §Flattening (the overlay-semantics enumeration) and
//! `docs/specs/openehr/AM/docs/ADL2/master09.02`–`master09.10` (the concrete
//! redefinition + section-merge rules). master08 §Flattening explicitly defers
//! the concrete algorithm to the ADL-Workbench reference compiler
//! ("provide a reasonable template"), so the *ordering* of the overlay concerns
//! within a single step is our own design; the individual rules below are each
//! spec-cited.
//!
//! One flatten step ([`flatten`]) overlays the child differential onto an
//! already-flat parent, handling: differential paths (incl. overridden codes)
//! and path congruence (`master09.02` §Path Congruence); node overlay by
//! congruent id with cloning vs in-place replacement (`master09.05` §Single and
//! Multiple Specialisation — the `clone_not_needed` predicate); sibling-order
//! anchors (`master09.04` §Ordering of Sibling Nodes — the moving-anchor run
//! model); deletions (`occurrences {0}` / `existence {0}`); proxy inline
//! expansion on override (`master09.05` §Internal Reference (Proxy Object)
//! Redefinition); and the section-level merges (`master09.06`–`master09.10`).
//! Multi-level lineage is flattened top-down and memoised by [`flat_form`].
//!
//! NOTE: plain flattening's treatment of prohibited nodes is spec-silent (the
//! normative removal/inlining rules are OPT-only —
//! `docs/specs/openehr/AM/docs/OPT2/master03-opt_raw.adoc` §Flattening;
//! `master08` §Flattening defers to the Workbench): a prohibited
//! (`occurrences {0}`) node is KEPT stripped of its sub-structure, and
//! un-overridden proxies / open slots are retained — our own design/extension
//! for a lossless plain flat form.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_base::prelude::MultiplicityInterval;
use openehr_lang::nesting::MAX_NESTING_DEPTH;

use crate::aom::access::{
    child_occurrences, complex_attributes, complex_node_id, object_node_id, sibling_order,
    strip_sibling_order,
};
use crate::aom::nesting::check_definition_nesting;
use crate::artefact::{ArchetypeRepository, view};
use crate::paths::{PathSegment, parse_path};
use crate::validate::conformance::tuple_member_names;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

/// A failure while flattening a specialised archetype against its parent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FlattenError {
    /// The archetype's declared specialisation parent is not in the repository.
    #[error("specialisation parent {0:?} was not found in the repository")]
    ParentNotFound(String),
    /// The specialisation lineage contains a cycle.
    #[error("specialisation lineage of {0:?} is cyclic")]
    CyclicLineage(String),
    /// A differential path in the child does not resolve in the flat parent.
    #[error("differential path {0:?} does not resolve in the flat parent")]
    UnresolvedDifferentialPath(String),
    /// The specialisation lineage is longer than the engine bound
    /// ([`openehr_lang::nesting::MAX_NESTING_DEPTH`] parents) — an
    /// implementation limit, not a spec rule.
    #[error("specialisation lineage exceeds the limit of {limit} levels")]
    LineageTooDeep {
        /// The bound that was crossed.
        limit: usize,
    },
    /// The flat form composes objects deeper than the engine bound
    /// ([`openehr_lang::nesting::MAX_NESTING_DEPTH`]): each artefact parses
    /// within it, but the differential paths place nodes below the parent's
    /// own depth — an implementation limit, not a spec rule.
    #[error("the flat form nests deeper than the limit of {limit} levels")]
    NestingTooDeep {
        /// The bound that was crossed.
        limit: usize,
    },
}

/// Flatten `child` (a differential specialised archetype) against its already-
/// flattened parent `flat_parent`, producing the flat form.
///
/// `repo` supplies proxy-target and supplier lookups. The result carries
/// `is_differential = false` (a flat form). `flat_parent` must be the true flat
/// form of the parent — for a specialised parent obtain it via [`flat_form`].
///
/// # Errors
/// [`FlattenError::UnresolvedDifferentialPath`] if a child differential path
/// cannot be located in the flat parent structure;
/// [`FlattenError::NestingTooDeep`] if the composed flat definition nests
/// past the engine bound.
pub fn flatten(
    child: &Archetype,
    flat_parent: &Archetype,
    _repo: &ArchetypeRepository,
) -> Result<Archetype, FlattenError> {
    let cv = view(child);
    let pv = view(flat_parent);
    let child_level = cv.specialisation_level();

    // Definition: start from a clone of the flat parent, restamp the root node
    // id to the child's specialised root, then overlay the child differential.
    let mut flat_def = pv.definition.clone();
    set_complex_node_id(&mut flat_def, complex_node_id(cv.definition).to_owned());
    overlay_root(&mut flat_def, cv.definition, child_level)?;
    check_definition_nesting(&flat_def)
        .map_err(|e| FlattenError::NestingTooDeep { limit: e.limit })?;

    // Terminology (master09.09 term_definitions accumulate / value_sets replace;
    // master09.10 bindings override).
    let flat_term = merge_terminology(pv.terminology, cv.terminology);

    Ok(rebuild_flat(child, flat_def, flat_term))
}

/// The flat form of `archetype`, resolving and flattening its full
/// specialisation lineage top-down (`master08` §Flattening: "process each
/// parent in order from the top").
///
/// A non-specialised archetype is its own flat form (`master09.02`
/// §Differential and Flat Forms). Results are memoised per repo lookup key
/// for the duration of the call graph.
///
/// # Errors
/// [`FlattenError::ParentNotFound`] if a lineage parent is absent from `repo`,
/// [`FlattenError::CyclicLineage`] on a lineage cycle,
/// [`FlattenError::LineageTooDeep`] past the engine bound on lineage length,
/// or a differential-path or nesting error from [`flatten`].
pub fn flat_form(
    archetype: &Archetype,
    repo: &ArchetypeRepository,
) -> Result<Archetype, FlattenError> {
    let mut memo = BTreeMap::new();
    flat_form_memo(archetype, repo, &mut memo, &mut Vec::new())
}

fn flat_form_memo(
    archetype: &Archetype,
    repo: &ArchetypeRepository,
    memo: &mut BTreeMap<String, Archetype>,
    stack: &mut Vec<String>,
) -> Result<Archetype, FlattenError> {
    let v = view(archetype);
    let Some(parent_id) = v.parent_archetype_id else {
        // Level-0: the flat form is the differential form as-is, but stamped
        // `is_differential = false` so downstream consumers see a flat artefact.
        return Ok(mark_flat(archetype));
    };
    let key = parent_id.to_owned();
    if stack.contains(&key) {
        return Err(FlattenError::CyclicLineage(key));
    }
    if stack.len() >= MAX_NESTING_DEPTH {
        return Err(FlattenError::LineageTooDeep {
            limit: MAX_NESTING_DEPTH,
        });
    }
    if let Some(cached) = memo.get(&key) {
        let cached = cached.clone();
        return flatten(archetype, &cached, repo);
    }
    let Some(parent) = repo.get(parent_id) else {
        return Err(FlattenError::ParentNotFound(parent_id.to_owned()));
    };
    stack.push(key.clone());
    let flat_parent = flat_form_memo(parent, repo, memo, stack)?;
    stack.pop();
    memo.insert(key, flat_parent.clone());
    flatten(archetype, &flat_parent, repo)
}

// ── overlay ───────────────────────────────────────────────────────────────

/// Overlay the child root's attributes onto the flat definition root, honouring
/// differential paths (`master09.02` §Specialisation Paths).
fn overlay_root(
    flat_def: &mut CComplexObject,
    child_def: &CComplexObject,
    level: usize,
) -> Result<(), FlattenError> {
    // Root occurrences / rm-type overrides (rare, but valid).
    if let (CComplexObject::CComplexObject(fd), CComplexObject::CComplexObject(cd)) =
        (&mut *flat_def, child_def)
        && cd.occurrences.is_some()
    {
        fd.occurrences.clone_from(&cd.occurrences);
    }
    for child_attr in complex_attributes(child_def) {
        if let Some(diff) = child_attr.differential_path.as_deref() {
            let target = resolve_object_mut(flat_def, diff)
                .ok_or_else(|| FlattenError::UnresolvedDifferentialPath(diff.to_owned()))?;
            overlay_attribute(target.attributes.get_or_insert_default(), child_attr, level);
        } else {
            let attrs = complex_attributes_mut(flat_def);
            overlay_attribute(attrs, child_attr, level);
        }
    }
    Ok(())
}

/// Overlay a single child `C_ATTRIBUTE` onto the base attribute list. Matches by
/// `rm_attribute_name`; a new attribute is added, an existing one has its
/// existence/cardinality overrides applied and its children overlaid.
#[expect(
    clippy::indexing_slicing,
    reason = "`pos` is the `Iterator::position` of the matching attribute in this very `base_attrs` vector — the let-else below returns when there is none, and nothing removes from the vector before the indexed accesses, so every index is in bounds by construction"
)]
fn overlay_attribute(base_attrs: &mut Vec<CAttribute>, child_attr: &CAttribute, level: usize) {
    let Some(pos) = base_attrs
        .iter()
        .position(|a| a.rm_attribute_name == child_attr.rm_attribute_name)
    else {
        // ADD: a brand-new attribute — its children are new nodes.
        let mut added = child_attr.clone();
        added.differential_path = None;
        for c in added.children.iter_mut().flatten() {
            strip_sibling_order(c);
        }
        base_attrs.push(added);
        return;
    };
    // Existence override; `existence {0}` = attribute deletion (kept visible in
    // plain flat, stripped of children — see the module NOTE).
    if let Some(ex) = child_attr.existence.as_ref() {
        base_attrs[pos].existence = Some(ex.clone());
    }
    if let Some(card) = child_attr.cardinality.as_ref() {
        base_attrs[pos].cardinality = Some(card.clone());
    }
    base_attrs[pos].is_multiple = base_attrs[pos].is_multiple || child_attr.is_multiple;

    if child_attr
        .existence
        .as_ref()
        .is_some_and(MultiplicityInterval::is_prohibited)
    {
        base_attrs[pos].children = None;
        return;
    }
    overlay_children(&mut base_attrs[pos], child_attr, level);
}

/// The overlay classification of one child object node against the flat parent.
enum Overlaid {
    /// Redefines the base node at `base_idx` (congruent id), overlaid form.
    Redef { base_idx: usize, node: CObject },
    /// A brand-new (extension) node.
    New { node: CObject },
}

/// Overlay the children of one container/single attribute — the cloning +
/// sibling-order core.
///
/// Classifies each child node in source order as a redefinition of a
/// congruent base node or a new extension, carrying its EXPLICIT sibling
/// marker.
///
/// The parser attaches a marker only to the node immediately after the marker
/// keyword; per `master09.04` L249 a marker anchors every following node until
/// the next marker, so the run range is everything from the first
/// explicitly-marked node onward.
#[expect(
    clippy::indexing_slicing,
    reason = "`base_idx` is a `find_congruent_idx` result over `base_nodes`, so it is in bounds by construction"
)]
fn classify_children(
    base_nodes: &[CObject],
    child_attr: &CAttribute,
) -> Vec<(Overlaid, Option<SiblingOrder>)> {
    let mut classified: Vec<(Overlaid, Option<SiblingOrder>)> = Vec::new();
    for child in child_attr.children.iter().flatten() {
        let marker = sibling_order(child).cloned();
        let overlaid = if let Some(base_idx) = find_congruent_idx(base_nodes, object_node_id(child))
        {
            let node = overlay_node(&base_nodes[base_idx], child);
            Overlaid::Redef { base_idx, node }
        } else {
            let mut node = child.clone();
            strip_sibling_order(&mut node);
            Overlaid::New { node }
        };
        classified.push((overlaid, marker));
    }
    classified
}

/// Phase 1: the working list with every unmarked node at its default position
/// — redefinitions in place or cloned, extensions appended
/// (`master09.04` L272).
///
/// The cloning decision uses the WHOLE redef set, but only the default
/// (pre-marker) redefs are placed here: run-range redefs are positioned by
/// their run in phase 2, so a base node whose only redefinition is in the run
/// range is dropped here for the run to place. Where cloning is needed the
/// parent node survives and the default clone-redefs follow it
/// (`master09.05` §Single and Multiple Specialisation).
#[expect(
    clippy::indexing_slicing,
    reason = "`split` is a `position(..).unwrap_or(len)` over `classified`, so `classified[..split]` is valid"
)]
fn place_default_nodes(
    base_attr: &CAttribute,
    base_nodes: &[CObject],
    classified: &[(Overlaid, Option<SiblingOrder>)],
    split: usize,
) -> Vec<CObject> {
    let mut work: Vec<CObject> = Vec::new();
    for (base_idx, base_node) in base_nodes.iter().enumerate() {
        let all_redefs = redef_nodes_for(classified, base_idx);
        if all_redefs.is_empty() {
            work.push(base_node.clone());
            continue;
        }
        let default_redefs = redef_nodes_for(&classified[..split], base_idx);
        if !clone_not_needed(base_node, base_attr, &all_redefs) {
            work.push(base_node.clone());
        }
        for node in default_redefs {
            work.push(node.clone());
        }
    }
    for (o, _) in &classified[..split] {
        if let Overlaid::New { node } = o {
            work.push(node.clone());
        }
    }
    work
}

/// Two phases (`master09.04`/`master09.05`):
/// 1. Build the working list with in-place redefinitions and cloning applied,
///    ignoring sibling markers (unmarked nodes placed at their default position:
///    redefinitions in place, extensions at the end).
/// 2. Insert the marked nodes at their anchors — a `before`/`after` marker
///    anchors the run of nodes following it until the next marker
///    (`master09.04` L249), resolved with dependency ordering so an anchor that
///    is itself placed by a later run is available first.
#[expect(
    clippy::indexing_slicing,
    reason = "every index here is in bounds by construction: `split` is a `position(..).unwrap_or(len)` so `classified[split..]` is valid, and `runs[i]` is guarded by the enclosing `i < runs.len()`"
)]
fn overlay_children(base_attr: &mut CAttribute, child_attr: &CAttribute, _level: usize) {
    let base_nodes = base_attr.children.clone();

    // Classify each child node in source order, carrying its EXPLICIT sibling
    // marker. The parser attaches a marker only to the node immediately after
    // the marker keyword; per `master09.04` L249 a marker anchors every following
    // node until the next marker, so the run range is everything from the first
    // explicitly-marked node onward.
    let classified = classify_children(base_nodes.as_deref().unwrap_or_default(), child_attr);
    let split = classified
        .iter()
        .position(|(_, m)| m.is_some())
        .unwrap_or(classified.len());
    let mut work = place_default_nodes(
        base_attr,
        base_nodes.as_deref().unwrap_or_default(),
        &classified,
        split,
    );

    // Phase 2: place the run-range nodes by their runs (dependency-ordered).
    let mut runs = build_runs(&classified[split..]);
    let mut progress = true;
    while !runs.is_empty() && progress {
        progress = false;
        let mut i = 0;
        while i < runs.len() {
            if let Some(at) = resolve_anchor(&work, &runs[i].anchor) {
                let run = runs.remove(i);
                insert_run(&mut work, at, run.nodes);
                progress = true;
            } else {
                i += 1;
            }
        }
    }
    // Any run whose anchor never resolved (dangling — VSSM is the phase-2
    // diagnostic) is appended at the end so no nodes are lost.
    for run in runs {
        let end = work.len();
        insert_run(&mut work, InsertAt::Index(end), run.nodes);
    }

    base_attr.children = openehr_base::containers::present(work);
}

/// One anchored run of marked sibling nodes.
struct Run {
    anchor: SiblingOrder,
    nodes: Vec<CObject>,
}

/// The overlaid redefinition nodes among `items` that redefine base node
/// `base_idx`, in source order.
fn redef_nodes_for(items: &[(Overlaid, Option<SiblingOrder>)], base_idx: usize) -> Vec<&CObject> {
    items
        .iter()
        .filter_map(|(o, _)| match o {
            Overlaid::Redef { base_idx: bi, node } if *bi == base_idx => Some(node),
            _ => None,
        })
        .collect()
}

/// Build the runs from the run-range items (`master09.04` L249): an explicit
/// marker starts a run; each following un-marked node joins the current run
/// (the parser attaches the marker only to the first node of the run).
fn build_runs(run_items: &[(Overlaid, Option<SiblingOrder>)]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (o, m) in run_items {
        let node = match o {
            Overlaid::Redef { node, .. } | Overlaid::New { node } => node.clone(),
        };
        if let Some(marker) = m {
            runs.push(Run {
                anchor: marker.clone(),
                nodes: vec![node],
            });
        } else if let Some(last) = runs.last_mut() {
            last.nodes.push(node);
        }
        // A run-range node with no marker and no active run cannot occur — the
        // range begins at the first explicitly-marked node.
    }
    runs
}

/// Where to splice a run into the working list.
#[derive(Clone, Copy)]
enum InsertAt {
    Index(usize),
}

/// Resolve a sibling-order anchor to an insertion point in `work`
/// (`master09.04` L268-279): `before [X]` inserts before the first node
/// conforming to X, `after [X]` after the last such node.
fn resolve_anchor(work: &[CObject], anchor: &SiblingOrder) -> Option<InsertAt> {
    let matches: Vec<usize> = work
        .iter()
        .enumerate()
        .filter(|(_, n)| anchor_matches(object_node_id(n), &anchor.sibling_node_id))
        .map(|(i, _)| i)
        .collect();
    let idx = if anchor.is_before {
        *matches.first()?
    } else {
        *matches.last()?
    };
    Some(if anchor.is_before {
        InsertAt::Index(idx)
    } else {
        InsertAt::Index(idx + 1)
    })
}

/// A node id matches a sibling-order anchor id if it is identical or conforms to
/// it (the anchor id may have been redefined away — `master09.04` L274-279:
/// resolve to a currently-available conforming sibling).
fn anchor_matches(node_id: &str, anchor_id: &str) -> bool {
    node_id == anchor_id || AdlCodeDefinitionsData::codes_conformant(node_id, anchor_id)
}

fn insert_run(work: &mut Vec<CObject>, at: InsertAt, nodes: Vec<CObject>) {
    let InsertAt::Index(mut idx) = at;
    idx = idx.min(work.len());
    for node in nodes {
        work.insert(idx, node);
        idx += 1;
    }
}

/// The AOM2 `clone_not_needed` predicate (`master09.05` §Single and Multiple
/// Specialisation, L316-323):
///
/// ```text
/// clone not needed = max effective_occurrences of object node in parent = 1 OR
///     object node in child is sole child of its parent, and has max occurrences = 1
/// ```
///
/// When cloning is NOT needed the parent node is replaced in place; otherwise the
/// parent node survives and each child overlays a clone of it.
fn clone_not_needed(base_node: &CObject, base_attr: &CAttribute, redefs: &[&CObject]) -> bool {
    // A same-id redefinition (occurrences/prohibition/type change on the parent
    // node itself, not a split into new specialised siblings) is always in place —
    // the parent cannot be kept alongside a clone under the identical node id.
    if redefs
        .iter()
        .any(|n| object_node_id(n) == object_node_id(base_node))
    {
        return true;
    }
    // Case 1: max effective occurrences of the parent node == 1.
    if max_effective_occurrences(base_node, base_attr) == Some(1) {
        return true;
    }
    // Case 2: the child node is the sole redefinition of this parent and has an
    // explicit max occurrences of 1.
    if let [only] = redefs
        && child_occurrences(only).is_some_and(|o| !o.upper_unbounded && o.upper == Some(1))
    {
        return true;
    }
    false
}

/// The maximum effective occurrences of an object node under `attr`
/// (`master04.5` occurrences-inferencing rules): its explicit occurrences upper,
/// else the owning attribute's cardinality upper for a container, else 1 for a
/// single-valued attribute. `None` = unbounded (`*`).
fn max_effective_occurrences(node: &CObject, attr: &CAttribute) -> Option<i32> {
    if let Some(occ) = child_occurrences(node) {
        return if occ.upper_unbounded { None } else { occ.upper };
    }
    if attr.is_multiple {
        return attr
            .cardinality
            .as_ref()
            .filter(|c| !c.interval.upper_unbounded)
            .and_then(|c| c.interval.upper);
    }
    Some(1)
}

/// Overlay one child node onto its congruent base node (`master09.05`). For two
/// `C_COMPLEX_OBJECT`s the base is cloned and the child's id / type / occurrences
/// / attributes are overlaid; a prohibited (`occurrences {0}`) child keeps the
/// node visible but strips its sub-structure (plain flat — see the module NOTE).
/// For any other combination the child fully specifies the node and replaces it.
fn overlay_node(base: &CObject, child: &CObject) -> CObject {
    if let (
        CObject::CComplexObject(CComplexObject::CComplexObject(bp)),
        CObject::CComplexObject(CComplexObject::CComplexObject(cp)),
    ) = (base, child)
    {
        let mut out = bp.clone();
        out.node_id.clone_from(&cp.node_id);
        if !cp.rm_type_name.is_empty() {
            out.rm_type_name.clone_from(&cp.rm_type_name);
        }
        if cp.occurrences.is_some() {
            out.occurrences.clone_from(&cp.occurrences);
        }
        out.sibling_order = None;
        if out
            .occurrences
            .as_ref()
            .is_some_and(MultiplicityInterval::is_prohibited)
        {
            out.attributes = None;
            out.attribute_tuples = None;
            return CObject::CComplexObject(CComplexObject::CComplexObject(out));
        }
        for ca in cp.attributes.iter().flatten() {
            overlay_attribute(out.attributes.get_or_insert_default(), ca, 0);
        }
        overlay_attribute_tuples(&mut out, cp.attribute_tuples.as_deref().unwrap_or_default());
        return CObject::CComplexObject(CComplexObject::CComplexObject(out));
    }
    let mut node = child.clone();
    strip_sibling_order(&mut node);
    node
}

/// Overlays a child node's `C_ATTRIBUTE_TUPLE` set onto the base node's, keyed
/// by member-attribute group.
///
/// A tuple's identity is the attribute group it co-constrains: the conformance
/// functions compare a child tuple only against the parent tuple over the same
/// group (`AOM2/master04.5` §Conformance semantics: `C_SECOND_ORDER` —
/// "'corresponding' means a node found at the same or a congruent path"), and
/// `ADL2/master09.05` §Tuple Redefinition narrows a group by restating that
/// group's whole row list. So a child tuple REPLACES the base tuple over the
/// same group, a base tuple over a group the child does not restate is
/// RETAINED, and a child tuple over a group the base does not carry is APPENDED.
fn overlay_attribute_tuples(out: &mut CComplexObjectData, child: &[CAttributeTuple]) {
    // NOTE: no openEHR spec governs tuple overlay — `AOM2/master08` §Flattening
    // enumerates no second-order case; group-keyed merge is our own design, so
    // flattening never silently drops an inherited constraint.
    if child.is_empty() {
        return;
    }
    let mut merged: Vec<CAttributeTuple> = out
        .attribute_tuples
        .iter()
        .flatten()
        .map(|base| {
            child
                .iter()
                .find(|c| tuple_group_key(c) == tuple_group_key(base))
                .unwrap_or(base)
                .clone()
        })
        .collect();
    for added in child {
        if !merged
            .iter()
            .any(|m| tuple_group_key(m) == tuple_group_key(added))
        {
            merged.push(added.clone());
        }
    }
    out.attribute_tuples = openehr_base::containers::present(merged);
}

/// The order-insensitive member-attribute group a `C_ATTRIBUTE_TUPLE`
/// co-constrains — its overlay identity.
///
/// Sorted, because the conformance functions match two tuples on their member
/// SET and map positions between them (`AOM2/master04.5` §Conformance
/// semantics: `C_ATTRIBUTE_TUPLE`), not on declaration order.
fn tuple_group_key(tuple: &CAttributeTuple) -> Vec<&str> {
    let mut names = tuple_member_names(tuple);
    names.sort_unstable();
    names
}

// ── differential-path navigation + proxy expansion ─────────────────────────

/// Navigate `root` to the object node addressed by `path`, expanding any proxy
/// (`C_COMPLEX_OBJECT_PROXY`) encountered on the way to an inline copy of its
/// target (`master09.05` §Internal Reference (Proxy Object) Redefinition — a
/// proxy overridden through its path is expanded inline). Returns a mutable
/// reference to the target complex object, or `None` if the path does not
/// resolve.
#[expect(
    clippy::indexing_slicing,
    reason = "`attr_pos` is an `Iterator::position` over `current.attributes` and `child_pos` a `pick_child_pos` result over that attribute's `children`, so both are in bounds by construction; a `get_mut` rewrite would split the reborrow chain this `&mut` navigation loop depends on"
)]
fn resolve_object_mut<'a>(
    root: &'a mut CComplexObject,
    path: &str,
) -> Option<&'a mut CComplexObjectData> {
    let segments = parse_path(path);
    // Resolve a fresh snapshot of proxy targets before mutating (proxy expansion
    // reads the current tree).
    let snapshot = root.clone();
    let mut current: &mut CComplexObjectData = match root {
        CComplexObject::CComplexObject(d) => d,
        CComplexObject::CArchetypeRoot(r) => {
            // Treat an archetype root as a complex object for navigation by
            // reborrowing its attributes through a data view is not directly
            // possible; the flattener only navigates plain complex objects.
            let _ = r;
            return None;
        }
    };
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
            children[child_pos] = expand_proxy(&snapshot, proxy)?;
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

// ── terminology merge ──────────────────────────────────────────────────────

/// Merge parent + child terminology for the flat form (`master09.09`,
/// `master09.10`): `term_definitions` accumulate per language (child overrides /
/// adds; the parent's terms always survive) restricted to the languages present
/// in the flat parent (`master09.07` — languages intersect, child-only languages
/// discarded); `value_sets` replace by id; `term_bindings` override by
/// (terminology, code).
fn merge_terminology(
    parent: &ArchetypeTerminology,
    child: &ArchetypeTerminology,
) -> ArchetypeTerminology {
    let mut flat = parent.clone();
    flat.is_differential = false;
    flat.concept_code.clone_from(&child.concept_code);
    flat.original_language.clone_from(&child.original_language);

    // term_definitions accumulate, only for languages already in the flat parent.
    for (lang, cmap) in &child.term_definitions {
        if let Some(pmap) = flat.term_definitions.get_mut(lang) {
            for (code, term) in cmap {
                pmap.insert(code.clone(), term.clone());
            }
        }
    }

    // value_sets: union by id, child replaces the same id.
    if let Some(cvs) = &child.value_sets {
        let out = flat.value_sets.get_or_insert_with(BTreeMap::new);
        for (id, vs) in cvs {
            out.insert(id.clone(), vs.clone());
        }
    }

    // term_bindings: override by (terminology, code).
    if let Some(cbind) = &child.term_bindings {
        let out = flat.term_bindings.get_or_insert_with(BTreeMap::new);
        for (term_id, codes) in cbind {
            let inner = out.entry(term_id.clone()).or_default();
            for (code, uri) in codes {
                inner.insert(code.clone(), uri.clone());
            }
        }
    }

    flat
}

// ── archetype (re)construction ─────────────────────────────────────────────

/// Rebuild the flat archetype: clone `child`'s metadata, install the flattened
/// `definition` + `terminology`, append the child's rules to the parent's (rules
/// combine, `master09.06`), and mark it non-differential. The child's
/// `description` is kept verbatim (it replaces the parent's, `master09.08`).
fn rebuild_flat(
    child: &Archetype,
    definition: CComplexObject,
    terminology: ArchetypeTerminology,
) -> Archetype {
    match child {
        Archetype::AuthoredArchetype(inner) => {
            Archetype::AuthoredArchetype(Box::new(match inner.as_ref() {
                AuthoredArchetype::AuthoredArchetype(d) => {
                    let mut out = d.clone();
                    out.definition = definition;
                    out.terminology = Box::new(terminology);
                    out.is_differential = false;
                    AuthoredArchetype::AuthoredArchetype(out)
                }
                AuthoredArchetype::Template(t) => {
                    let mut out = t.clone();
                    out.definition = definition;
                    out.terminology = terminology;
                    out.is_differential = false;
                    AuthoredArchetype::Template(out)
                }
                AuthoredArchetype::OperationalTemplate(o) => {
                    let mut out = o.clone();
                    out.definition = definition;
                    out.terminology = terminology;
                    out.is_differential = false;
                    AuthoredArchetype::OperationalTemplate(out)
                }
            }))
        }
        Archetype::TemplateOverlay(t) => {
            let mut out = t.as_ref().clone();
            out.definition = definition;
            out.terminology = Box::new(terminology);
            out.is_differential = false;
            Archetype::TemplateOverlay(Box::new(out))
        }
    }
}

/// A level-0 archetype's flat form: itself with `is_differential = false`.
fn mark_flat(archetype: &Archetype) -> Archetype {
    let def = view(archetype).definition.clone();
    let term = view(archetype).terminology.clone();
    rebuild_flat(archetype, def, term)
}

// ── small helpers ──────────────────────────────────────────────────────────

/// The mutable attribute list of a `C_COMPLEX_OBJECT`, materialised.
///
/// `C_COMPLEX_OBJECT.attributes` is `0..1`, so it emits as `Option<Vec<…>>`;
/// every caller here is about to overlay an attribute INTO the list, so the
/// absent state is materialised on the way in and the returned list is never
/// left empty.
fn complex_attributes_mut(cco: &mut CComplexObject) -> &mut Vec<CAttribute> {
    match cco {
        CComplexObject::CComplexObject(d) => d.attributes.get_or_insert_default(),
        CComplexObject::CArchetypeRoot(r) => r.attributes.get_or_insert_default(),
    }
}

fn set_complex_node_id(cco: &mut CComplexObject, id: String) {
    match cco {
        CComplexObject::CComplexObject(d) => d.node_id = id,
        CComplexObject::CArchetypeRoot(r) => r.node_id = id,
    }
}

/// The index of the base node congruent to `child_id` (same id or a parent of
/// it, `master09.02` §Path Congruence), preferring the deepest/most-specific
/// match. Primitive leaves (which carry no real node id) are matched by the same-
/// type fallback elsewhere, not here.
fn find_congruent_idx(base: &[CObject], child_id: &str) -> Option<usize> {
    if child_id.is_empty() {
        return None;
    }
    // Exact id first, then a specialisation (child_id conforms to base id).
    if let Some(i) = base.iter().position(|b| object_node_id(b) == child_id) {
        return Some(i);
    }
    base.iter().position(|b| {
        !object_node_id(b).is_empty()
            && AdlCodeDefinitionsData::codes_conformant(child_id, object_node_id(b))
    })
}
