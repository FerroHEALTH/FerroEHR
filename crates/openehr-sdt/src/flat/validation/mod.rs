// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! **Archetype-conformance validation** — the template-driven pass, plus the
//! Simplified-Formats input checks.
//!
//! [`validate_archetype_conformance`] walks a canonical-JSON instance guided by
//! its operational template (as a flattened [`WebTemplate`]), matching by
//! `aql_path` + `archetype_node_id`, and checks type conformance, occurrences,
//! cardinality, and leaf domain constraints (coded lists / numeric ranges /
//! string patterns), collecting *all* violations (not fail-fast). Paths are the
//! archetype `aqlPath` of the constraining node.
//!
//! Everything here needs a template. The template-independent passes live in
//! [`crate::rm_instance`], which also owns the shared [`ValidationMessage`]
//! shape and the composed [`crate::rm_instance::validate_composition`] entry
//! point.
//!
//! Two of the Simplified-Formats input rules
//! (`simplified_formats/master04-basic_concepts.adoc` §Validation) are
//! properties of the FLAT input rather than of the RM tree, so they are checked
//! before conversion: [`validate_flat_other`] for the `|other` open-value-set
//! rules and [`validate_context`] for the mandatory `ctx/language` +
//! `ctx/territory` fields.
//!
//! # Fidelity
//!
//! The instance-validation algorithm is spec-underdetermined: the AOM 1.4
//! constraint model defines a positive-only `Valid_value` cascade over the
//! archetype constraint tree and is silent on unmatched instance nodes. The walk
//! here approximates it over the COMPACTED `WebTemplate` — the wrapper nodes the
//! AOM tree carries are folded into a child's `aqlPath`, so the instance is
//! navigated by the RM-attribute + `[archetype_node_id]` chain that separates a
//! child from its compacted parent, counting occurrences per intermediate
//! container. `C_DV_*` leaf semantics are approximated from the `WebTemplate`
//! `inputs`. A check that cannot be made reliably is skipped rather than
//! over-rejected, so only confident violations are reported.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

mod leaf;
mod subtype;

use indexmap::IndexMap;
use openehr_rm::v1_2::paths::PathSegment;
use serde_json::{Map, Value};

use crate::rm_instance::{ValidationKind, ValidationMessage};

use crate::flat::path;
use crate::flat::rmpath;
use crate::flat::sim::{SimDocument, SimNode, is_present};
use crate::flat::webtemplate::model::{WebTemplate, WebTemplateArchetypeSlot, WebTemplateNode};

/// Validate only the archetype-conformance pass against a resolved
/// [`WebTemplate`] (type conformance, occurrences, cardinality, and leaf
/// domain constraints).
///
/// Callers run [`crate::rm_instance::validate_rm_and_terminology`] separately
/// for the template-independent checks, so this is the additional pass a
/// *declared* template contributes.
#[must_use]
pub fn validate_archetype_conformance(
    composition: &Value,
    wt: &WebTemplate,
) -> Vec<ValidationMessage> {
    let mut v = Validator::default();
    v.walk(composition, &wt.tree);
    v.out
}

/// Archetype-conformance pass for a `553|incomplete|` commit.
///
/// Identical to
/// [`validate_archetype_conformance`] but with existence/occurrences/cardinality
/// **lower** limits treated as zero (RM common master06 §"Incomplete Content":
/// "in an `incomplete` commit, data may be missing, but it may not be wrong …
/// all existence and cardinality lower limits set to zero"). Type conformance,
/// upper limits, and every leaf/value constraint are still enforced — the
/// relaxation tolerates *missing* data, not *wrong* data. The caller must run
/// the RM-invariant + terminology passes at full strictness regardless
/// (they are properties of the instance, not archetype lower bounds).
#[must_use]
pub fn validate_archetype_conformance_incomplete(
    composition: &Value,
    wt: &WebTemplate,
) -> Vec<ValidationMessage> {
    let mut v = Validator {
        relax_lower_bounds: true,
        ..Validator::default()
    };
    v.walk(composition, &wt.tree);
    v.out
}

/// One terminology-server question the template's archetype **constraint
/// bindings** raise about a coded value in this instance.
///
/// A `CONSTRAINT_REF` bound to an external terminology query names a value set
/// the instance's code must belong to (BASE
/// `architecture_overview/master12-terminology.adoc` §"Binding Terminology
/// Value-sets to Archetypes": the ac-code "is bound to queries to one or more
/// external terminologies, whose result would be a … value set from that
/// terminology"). The query lives in a "terminology query server", which this
/// crate has no access to — so the walk collects the questions and a caller
/// that owns a terminology service answers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintBindingCheck {
    /// The archetype `aqlPath` of the constrained node (the path a resulting
    /// violation is keyed by).
    pub path: String,
    /// The archetype constraint code (`ac0004`) the binding is keyed by.
    pub ac_code: String,
    /// The terminology the binding names (`SNOMED-CT`, `LOINC`, …) — the
    /// routing key for choosing a terminology server.
    pub binding_terminology: String,
    /// The bound terminology-query URI identifying the admissible value set.
    pub query_uri: String,
    /// The instance code's own terminology id (`CODE_PHRASE.terminology_id`).
    pub instance_terminology: String,
    /// The instance code (`CODE_PHRASE.code_string`) whose membership is in
    /// question.
    pub instance_code: String,
}

/// Collects the [`ConstraintBindingCheck`]s a template's bindings raise for an
/// instance.
///
/// The checks are gathered by the same archetype-conformance walk
/// [`validate_archetype_conformance`] performs (so a check is raised only for a
/// node the template actually matched). Pure: no violation is decided here —
/// the caller resolves each query against its terminology service.
#[must_use]
pub fn collect_constraint_binding_checks(
    composition: &Value,
    wt: &WebTemplate,
) -> Vec<ConstraintBindingCheck> {
    let mut v = Validator {
        collect_bindings: true,
        ..Validator::default()
    };
    v.walk(composition, &wt.tree);
    v.bindings
}

/// Validate the `|other` open-value-set rules on a **FLAT input** map, before
/// conversion to RM (ITS-REST `simplified_formats/master04-basic_concepts.adoc`
/// §"Open Value-Sets and the `|other` Suffix"):
///
/// * `|other` MUST NOT co-occur with `|code`/`|value`/`|terminology`/
///   `|preferred_term` on the same leaf path;
/// * `|other` MUST be rejected when the leaf's WT constraint is a **closed**
///   coded list (`listOpen: false`).
///
/// This is the validation-**report** form of the same two rules the conversion
/// path enforces as hard rejections: `crate::flat::map::build_leaf` returns
/// [`crate::flat::error::FlatError::OtherSuffixConflict`] /
/// [`crate::flat::error::FlatError::OtherOnClosedValueSet`] when it hits them during a
/// build, so a converter fails fast. This function instead collects them as
/// [`ValidationMessage`]s (validators report, converters reject), for a caller
/// that wants to surface every violation on the raw FLAT input without running a
/// conversion. An empty result means the input satisfies the `|other` rules this
/// check covers.
#[must_use]
pub fn validate_flat_other(doc: &Map<String, Value>, wt: &WebTemplate) -> Vec<ValidationMessage> {
    const EXCLUSIVE: &[&str] = &["code", "value", "terminology", "preferred_term"];
    let mut out = Vec::new();
    // Group each FLAT key by its segment path (`master04 §Field Identifiers`),
    // collecting the datum suffix (`master05` per-type suffix) present on each.
    let mut per_leaf: IndexMap<String, LeafGroup> = IndexMap::new();
    for key in doc.keys() {
        let Ok(fk) = path::FlatKey::parse(key) else {
            continue; // a malformed key is the conversion path's rejection, not this one's
        };
        if fk.is_ctx() {
            continue;
        }
        // The segment portion is everything before the first pipe (the printed
        // path); the first suffix is the leaf's datum suffix (`master05`).
        let path_str = key.split('|').next().unwrap_or(key).to_owned();
        let suffix = fk.suffixes.first().map(|s| s.name.clone());
        let entry = per_leaf.entry(path_str).or_insert_with(|| LeafGroup {
            segments: fk.segments,
            suffixes: Vec::new(),
        });
        if let Some(s) = suffix {
            entry.suffixes.push(s);
        }
    }
    for (path_str, group) in &per_leaf {
        if !group.suffixes.iter().any(|s| s == "other") {
            continue;
        }
        // Mutual exclusion.
        if let Some(conflict) = group
            .suffixes
            .iter()
            .find(|s| EXCLUSIVE.contains(&s.as_str()))
        {
            out.push(ValidationMessage {
                path: path_str.clone(),
                message: format!(
                    "`|other` is mutually exclusive with `|{conflict}` on the same leaf \
                     (master04 §Open Value-Sets)"
                ),
                kind: ValidationKind::CodedValue,
            });
        }
        // Closed-list rejection: resolve the WT node for this path.
        if let Some(node) = find_node_by_segments(wt, &group.segments)
            && node.inputs.iter().any(|i| i.list_open == Some(false))
        {
            out.push(ValidationMessage {
                path: path_str.clone(),
                message: "`|other` is not allowed on a closed value-set (listOpen: false) \
                          (master04 §Open Value-Sets)"
                    .to_owned(),
                kind: ValidationKind::CodedValue,
            });
        }
    }
    out
}

/// One FLAT leaf-path grouping for [`validate_flat_other`]: the parsed segments
/// (for WT resolution) and the datum suffixes present on the path.
struct LeafGroup {
    segments: Vec<path::Segment>,
    suffixes: Vec<String>,
}

/// The `(path, code)` of every FLAT leaf whose `|code` names a code outside
/// its closed web-template value set while no `|value` suffix supplies the
/// text.
///
/// Such a datum cannot be resolved to a valid coded value (master04
/// §Validation: "Terminology bindings are valid"); the closed-set predicate
/// and the terminology-scope bias mirror the archetype pass's
/// `check_code_membership` (confident violations only — an explicit
/// differently-scoped `|terminology` is left to the terminology pass).
#[must_use]
pub fn unresolvable_coded_leaves(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
) -> Vec<(String, String)> {
    struct CodedLeaf {
        segments: Vec<path::Segment>,
        code: Option<String>,
        has_value: bool,
        terminology: Option<String>,
    }
    let mut per_leaf: IndexMap<String, CodedLeaf> = IndexMap::new();
    for (key, value) in doc {
        let Ok(fk) = path::FlatKey::parse(key) else {
            continue; // a malformed key is the conversion path's rejection, not this one's
        };
        if fk.is_ctx() {
            continue;
        }
        let path_str = key.split('|').next().unwrap_or(key).to_owned();
        let suffix = fk.suffixes.first().map(|s| s.name.clone());
        let entry = per_leaf.entry(path_str).or_insert_with(|| CodedLeaf {
            segments: fk.segments,
            code: None,
            has_value: false,
            terminology: None,
        });
        match suffix.as_deref() {
            Some("code") => entry.code = value.as_str().map(str::to_owned),
            Some("value") => entry.has_value = true,
            Some("terminology") => entry.terminology = value.as_str().map(str::to_owned),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (path_str, leaf) in per_leaf {
        let Some(code) = leaf.code else { continue };
        if leaf.has_value {
            continue;
        }
        let Some(node) = find_node_by_segments(wt, &leaf.segments) else {
            continue;
        };
        let miss = node.inputs.iter().any(|input| {
            !input.list.is_empty()
                && input.list_open != Some(true)
                && leaf::terminology_matches(
                    input.terminology.as_deref(),
                    leaf.terminology.as_deref(),
                )
                && !input.list.iter().any(|cv| cv.value == code)
        });
        if miss {
            out.push((path_str, code));
        }
    }
    out
}

/// Validates that the **mandatory context fields** are present.
///
/// The fields are checked on a parsed simplified document (ITS-REST
/// `simplified_formats/master04-basic_concepts.adoc`
/// §Validation: "Mandatory context fields (language, territory) are present";
/// §Context: "Mandatory: language, territory").
///
/// A field counts as present when the `ctx/` node carries it with a non-empty
/// bare value, or — because master05 §COMPOSITION also permits the root-level
/// path spelling (`<root>/language|code`, `<root>/territory|code`) — when a
/// non-`ctx` data root carries a child of that name. Returns one
/// [`ValidationMessage`] (`Required`, keyed `ctx/<field>`) per absent field.
#[must_use]
pub fn validate_context(doc: &SimDocument) -> Vec<ValidationMessage> {
    let mut out = Vec::new();
    let ctx = doc.child("ctx");
    for field in ["language", "territory"] {
        let in_ctx = ctx
            .and_then(|c| c.child(field))
            .and_then(SimNode::bare)
            .is_some_and(is_present);
        // master05 §COMPOSITION permits the root-level path spelling too; a value
        // there satisfies the mandatory-field rule, so it is not falsely flagged.
        let in_root = doc.children.iter().any(|(name, child)| {
            name.as_str() != "ctx"
                && child
                    .occurrences
                    .iter()
                    .any(|occ| occ.child(field).is_some())
        });
        if !in_ctx && !in_root {
            out.push(ValidationMessage {
                path: format!("ctx/{field}"),
                message: format!(
                    "mandatory context field '{field}' is absent (language and territory \
                     are required)"
                ),
                kind: ValidationKind::Required,
            });
        }
    }
    out
}

/// Resolve the [`WebTemplateNode`] a FLAT leaf's parsed `segments` address by
/// descending the WT tree on json-id segments (indices ignored, the leading root
/// id checked). Returns `None` for an unknown path or a `_`-prefixed RM-attribute
/// segment (which addresses no template node).
fn find_node_by_segments<'a>(
    wt: &'a WebTemplate,
    segments: &[path::Segment],
) -> Option<&'a WebTemplateNode> {
    let mut iter = segments.iter();
    // The first segment is the root template id.
    let first = iter.next()?;
    if first.name != wt.tree.id {
        return None;
    }
    let mut node = &wt.tree;
    for seg in iter {
        if seg.is_rm_attribute() {
            return None; // an RM-attribute segment addresses no template node
        }
        node = node
            .children
            .iter()
            .find(|c| c.id == seg.name || c.alt_json_id.as_deref() == Some(seg.name.as_str()))?;
    }
    Some(node)
}

/// Name-based routing for same-archetype-id sibling constraints.
///
/// Maps each identity `(attribute, archetype_node_id)` that the template repeats
/// across more than one distinct sibling path (relative to `parent_aql`) — i.e.
/// the template differentiates same-`archetype_node_id` siblings by their
/// runtime `name` (RM common `master03-archetyped_package.adoc` §"The
/// `LOCATABLE` class" L33-35; AOM 1.4 `master04-constraint_model_package.adoc`
/// §`node_id` L41) — to the set of explicit `name/value` `C_STRING` constraints
/// its *name-qualified* siblings carry.
///
/// A present key marks the identity as name-differentiated, which routing uses
/// two ways ([`select_group_children`]): a name-qualified sibling
/// matches strictly (id + name, no id-only fallback), and the one *unqualified*
/// sibling (whose `name` is unconstrained) admits every same-id instance
/// **except** those bearing a name a name-qualified sibling claims.
pub(crate) type SiblingNameIndex = std::collections::HashMap<(String, String), Vec<String>>;

// ── pre-parsed archetype-conformance walk plan ─────────────────────────────────
//
// The archetype-conformance walk ([`Validator::walk`]) is guided by the
// constraint paths a [`WebTemplateNode`] carries, which are template-static:
// [`prepare_walk`] parses them once at build time into the cached
// [`WebTemplateNode::walk`]; a node without a cached plan gets an identical
// [`NodeWalk`] built per visit, so there is one code path. No openEHR spec
// governs the WebTemplate model or this plan — our own design/extension.

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
    for child in &mut node.children {
        prepare_walk(child);
    }
}

#[derive(Default)]
struct Validator {
    out: Vec<ValidationMessage>,
    /// When set, existence/occurrences/cardinality **lower** limits are treated
    /// as zero (missing/empty is tolerated), realizing the RM common master06
    /// §"Incomplete Content" relaxation for a `553|incomplete|` commit: "in an
    /// `incomplete` commit, data may be missing, but it may not be wrong … all
    /// existence and cardinality lower limits set to zero". Upper limits, type
    /// conformance, and every leaf/value constraint (`RangeError`,
    /// `PatternError`, `CodedValue`, `WrongType`) are still enforced — only the
    /// "not enough / absent" violations are suppressed.
    relax_lower_bounds: bool,
    /// When set, the archetype-conformance walk records the terminology
    /// questions the matched nodes' constraint bindings raise into
    /// [`Self::bindings`] (see [`collect_constraint_binding_checks`]). Off for
    /// the ordinary validation passes, which decide nothing about bindings.
    collect_bindings: bool,
    /// The collected constraint-binding checks (empty unless
    /// [`Self::collect_bindings`]).
    bindings: Vec<ConstraintBindingCheck>,
}

/// Does the generated RM model declare an attribute named `attr` on ANY class?
///
/// Name-only by design: an OPT constrains attributes at several RM levels and
/// the walker does not carry the declaring class here, while the defect this
/// answers — a constraint on an attribute NO RM class declares — is fully
/// decided by the name.
fn rm_declares_attribute(attr: &str) -> bool {
    static DECLARED: std::sync::LazyLock<std::collections::BTreeSet<&str>> =
        std::sync::LazyLock::new(|| {
            openehr_rm::v1_2::model::classes()
                .flat_map(|c| c.attributes.iter().map(|a| a.name))
                .collect()
        });
    DECLARED.contains(attr)
}

/// Why a template constraint cannot be evaluated against any instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnenforceableReason {
    /// The constrained attribute is declared by no RM class, so no conformant
    /// instance can carry it as a stored member.
    ///
    /// Two shapes reach this in the deployed OPT 1.4 corpus: a constraint on a
    /// computed FUNCTION written as if it were stored (`EVENT.offset`,
    /// `DV_PROPORTION.is_integral` — RM
    /// `UML/classes/org.openehr.rm.data_structures.event.adoc` §Functions and
    /// `UML/classes/org.openehr.rm.data_types.dv_proportion.adoc`
    /// §Functions declare them as functions, not attributes), and the US
    /// spelling `null_flavor` for the RM's `null_flavour`.
    AttributeNotInRmModel,
}

/// A template constraint the archetype-conformance walk cannot evaluate.
///
/// Enforcing one would demand an instance member the canonical reader refuses,
/// so the walk skips it. The skip is reported rather than dropped: template
/// content that can never be checked is a property of the template a caller is
/// entitled to see. These are NOT validation failures — a conformant instance
/// is unaffected, and nothing here rejects a commit.
#[derive(Debug, Clone)]
pub struct UnenforceableConstraint {
    /// Absolute archetype path of the constrained attribute.
    pub path: String,
    /// The constrained attribute name.
    pub attribute: String,
    /// Why the walk cannot evaluate it.
    pub reason: UnenforceableReason,
}

/// Reports every existence constraint in `wt` that the conformance walk cannot
/// evaluate.
///
/// A template-level property: the answer depends only on the template, so it is
/// computed once per template rather than per commit. Pair it with
/// [`validate_archetype_conformance`], which skips exactly these constraints.
///
/// # Examples
///
/// ```no_run
/// # use openehr_sdt::flat::validation::unenforceable_existence_constraints;
/// # fn demo(wt: &openehr_sdt::flat::webtemplate::model::WebTemplate) {
/// for skipped in unenforceable_existence_constraints(wt) {
///     eprintln!("unenforceable at {}: {}", skipped.path, skipped.attribute);
/// }
/// # }
/// # let _ = demo;
/// ```
#[must_use]
pub fn unenforceable_existence_constraints(wt: &WebTemplate) -> Vec<UnenforceableConstraint> {
    let mut out = Vec::new();
    collect_unenforceable(&wt.tree, &mut out);
    out
}

/// Walks the node tree accumulating unenforceable existence constraints.
fn collect_unenforceable(node: &WebTemplateNode, out: &mut Vec<UnenforceableConstraint>) {
    for ex in &node.existence {
        let attribute = ex.path.rsplit('/').next().unwrap_or_default();
        if attribute.is_empty() || rm_declares_attribute(attribute) {
            continue;
        }
        out.push(UnenforceableConstraint {
            path: ex.path.clone(),
            attribute: attribute.to_owned(),
            reason: UnenforceableReason::AttributeNotInRmModel,
        });
    }
    for child in &node.children {
        collect_unenforceable(child, out);
    }
}

impl Validator {
    fn push(&mut self, path: impl Into<String>, message: impl Into<String>, kind: ValidationKind) {
        self.out.push(ValidationMessage {
            path: path.into(),
            message: message.into(),
            kind,
        });
    }

    // ── Pass 3: WebTemplate archetype-conformance walk ────────────────────────

    /// Visit an instance node matched to a `WebTemplate` node: check type
    /// conformance, leaf domain constraints, then descend into the `WebTemplate`
    /// children and container cardinalities.
    /// Record the terminology questions this matched node's constraint
    /// bindings raise about the instance's coded value. The `CODE_PHRASE` is
    /// read from the binding's `attr` (`defining_code` for a `DV_CODED_TEXT`);
    /// an empty `attr` means the node itself IS the `CODE_PHRASE`. A node the
    /// instance leaves uncoded raises no question — the binding constrains a
    /// value that is not there.
    fn collect_node_bindings(&mut self, instance: &Value, wt: &WebTemplateNode) {
        for binding in &wt.constraint_bindings {
            let code_phrase = if binding.attr.is_empty() {
                Some(instance)
            } else {
                instance.get(&binding.attr)
            };
            let Some(cp) = code_phrase else { continue };
            let Some(code) = cp.get("code_string").and_then(Value::as_str) else {
                continue;
            };
            if code.is_empty() {
                continue;
            }
            let instance_terminology = cp
                .get("terminology_id")
                .and_then(|t| t.get("value"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            self.bindings.push(ConstraintBindingCheck {
                path: wt.aql_path.clone(),
                ac_code: binding.ac_code.clone(),
                binding_terminology: binding.terminology.clone(),
                query_uri: binding.query_uri.clone(),
                instance_terminology: instance_terminology.to_owned(),
                instance_code: code.to_owned(),
            });
        }
    }

    fn walk(&mut self, instance: &Value, wt: &WebTemplateNode) {
        if let Some(inst_type) = instance.get("_type").and_then(Value::as_str)
            && !subtype::conforms(inst_type, &wt.rm_type)
        {
            self.push(
                &wt.aql_path,
                format!(
                    "expected RM type conforming to {} but found {inst_type}",
                    wt.rm_type
                ),
                ValidationKind::WrongType,
            );
            // A wrong-typed node cannot be meaningfully checked further.
            return;
        }

        if !wt.inputs.is_empty() {
            leaf::check_inputs(self, instance, wt);
        }

        if self.collect_bindings && !wt.constraint_bindings.is_empty() {
            self.collect_node_bindings(instance, wt);
        }

        // The archetype-conformance walk is driven by the node's pre-parsed
        // [`NodeWalk`]: child sibling groups (polymorphic type choices resolved
        // together, never each flagged), the name-routing index, and the parsed
        // constraint paths, all computed ONCE at build time. A hand-built node
        // with no cached plan builds one on the fly (identical result), so every
        // check reads exactly one code path.
        let built;
        let plan: &NodeWalk = if let Some(p) = &wt.walk {
            p
        } else {
            built = NodeWalk::build(wt);
            &built
        };
        for group in &plan.child_groups {
            self.check_group(instance, wt, group, &plan.child_names, &plan.name_fallback);
        }

        self.check_cardinalities(instance, wt, plan);
        self.check_existence(instance, wt, plan);
        self.check_slots(instance, plan);
        self.check_closure(instance, wt, plan);
    }

    /// Closed-archetype walk. Under each constrained
    /// attribute this node records (an attribute with fixed archetype-node
    /// alternatives and/or open `ARCHETYPE_SLOT`s), an instance child bearing an
    /// `archetype_node_id` (i.e. a LOCATABLE — the archetyped-content
    /// discriminator; no RM metadata value carries one) must match a fixed
    /// sibling identity **or** an open slot; any other archetyped child is an
    /// "unexpected node". RM-permitted unconstrained metadata attributes and
    /// wholly-unconstrained attributes are never recorded, so stay open (closed-world capture
    /// rule 2). A rejected node is not descended into (the walk already skips it).
    ///
    /// NOTE: AOM 1.4 `valid_value`
    /// (`AM/docs/AOM1.4/master04-constraint_model_package.adoc` §`Valid_value`
    /// L60-62) is a positive-only cascade, silent on unmatched instance nodes;
    /// closed-world rejection follows the AOM2 direction + de-facto CDR behaviour
    /// and lands only behind the ECC zero-drift gate.
    fn check_closure(&mut self, instance: &Value, wt: &WebTemplateNode, plan: &NodeWalk) {
        for (ca, segs) in wt.closed_attributes.iter().zip(&plan.closed) {
            let Some(segments) = segs else {
                continue;
            };
            let Some((last, intermediate)) = segments.split_last() else {
                continue;
            };
            for container in
                &rmpath::navigate_matched(&[instance], intermediate, &plan.name_fallback)
            {
                let slot_counts = self.match_closed_children(container, ca, &last.attribute);
                self.check_slot_occurrences(ca, &last.attribute, &slot_counts);
            }
        }
    }

    /// Matches every child under one closed attribute against its fixed
    /// sibling alternatives and its slots, recording an "unexpected node" for
    /// each child neither admits, and returns the per-slot filler counts.
    ///
    /// A child with no `archetype_node_id` is not a LOCATABLE (RM metadata /
    /// PATHABLE value) and is not subject to sibling closure.
    ///
    /// NOTE: an unmatched archetype-rooted child is admitted where the
    /// attribute declares no ARCHETYPE_SLOT, OPT 1.4 flattening not
    /// enumerating the slot-fill universe; slots still gate.
    #[expect(
        clippy::indexing_slicing,
        reason = "`counts` is sized `ca.slots.len()` and the index is a `position` over those same slots, so it is in bounds by construction"
    )]
    fn match_closed_children(
        &mut self,
        container: &Value,
        ca: &crate::flat::webtemplate::model::WebTemplateClosedAttribute,
        attribute: &str,
    ) -> Vec<usize> {
        let mut counts = vec![0usize; ca.slots.len()];
        for child in children_under_attr(container, attribute) {
            let Some(nid) = child
                .get("archetype_node_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if ca.allowed_ids.iter().any(|a| a == nid) {
                continue; // Matches a fixed sibling alternative.
            }
            if ca.slots.is_empty() && openehr_rm::v1_2::paths::is_archetype_root_node_id(nid) {
                continue;
            }
            let ct = child.get("_type").and_then(Value::as_str).unwrap_or("");
            match ca.slots.iter().position(|s| slot_admits(s, ct, nid)) {
                Some(i) => counts[i] += 1,
                None => self.push(
                    &ca.path,
                    format!(
                        "unexpected node '{nid}' under '{attribute}': no matching archetype \
                         constraint or slot"
                    ),
                    ValidationKind::Unexpected,
                ),
            }
        }
        counts
    }

    /// Checks each slot's occurrences against the fillers matched to it.
    #[expect(
        clippy::indexing_slicing,
        reason = "`counts` is sized `ca.slots.len()` and the index is the `enumerate` index over those same slots, so it is in bounds by construction"
    )]
    fn check_slot_occurrences(
        &mut self,
        ca: &crate::flat::webtemplate::model::WebTemplateClosedAttribute,
        attribute: &str,
        counts: &[usize],
    ) {
        for (i, slot) in ca.slots.iter().enumerate() {
            let count = i32::try_from(counts[i]).unwrap_or(i32::MAX);
            if counts[i] == 0 {
                if slot.min >= 1 {
                    self.push(
                        &ca.path,
                        format!(
                            "mandatory archetype slot (occurrences {}..) under '{attribute}' \
                             has no filler",
                            slot.min
                        ),
                        ValidationKind::Required,
                    );
                }
            } else if slot.max != -1 && count > slot.max {
                self.push(
                    &ca.path,
                    format!(
                        "too many slot fillers under '{attribute}': found {}, expected at most {}",
                        counts[i], slot.max
                    ),
                    ValidationKind::Occurrences,
                );
            }
        }
    }

    /// Type-conformance check for wrapper constraints the compactor hoisted
    /// away (`ITEM_*` / `HISTORY` / single `EVENT` — AOM 1.4 type conformance;
    /// master16 §`ITEM_STRUCTURE`/§EVENT "Class not allowed"): an instance node
    /// matched at a recorded slot path must conform to one of the RM types the
    /// same-path slot alternatives allow. An abstract recorded type
    /// (`ITEM_STRUCTURE`, `EVENT`) admits every concrete subtype via
    /// [`subtype::conforms`].
    fn check_slots(&mut self, instance: &Value, plan: &NodeWalk) {
        for sg in &plan.slot_groups {
            let Some((last, intermediate)) = sg.segments.split_last() else {
                continue;
            };
            let containers =
                rmpath::navigate_matched(&[instance], intermediate, &plan.name_fallback);
            for container in &containers {
                for node in select_group_children(container, last, &plan.slot_names) {
                    let Some(it) = node.get("_type").and_then(Value::as_str) else {
                        continue;
                    };
                    if !sg.allowed.iter().any(|a| subtype::conforms(it, a)) {
                        self.push(
                            &sg.path,
                            format!(
                                "class {it} not allowed: the slot is constrained to [{}]",
                                sg.allowed.join(", ")
                            ),
                            ValidationKind::WrongType,
                        );
                    }
                }
            }
        }
    }

    /// AOM 1.4 `C_ATTRIBUTE.existence` check: for each mandatory plain
    /// RM attribute constrained on this node, verify the attribute *field* is
    /// present on the matched instance. Existence is distinct from occurrences
    /// (archetype-node-identified children) and cardinality (container
    /// membership) — it governs whether the attribute field is there at all — so
    /// this fills the gap for plain structural attributes the occurrence check
    /// deliberately skips. Only the lower bound (mandatory presence) is enforced;
    /// the upper bound is governed by RM single-valuedness / cardinality.
    fn check_existence(&mut self, instance: &Value, wt: &WebTemplateNode, plan: &NodeWalk) {
        // Existence is a lower-bound (mandatory-presence) constraint — relaxed
        // away for a `553|incomplete|` commit (master06 §"Incomplete Content").
        if self.relax_lower_bounds {
            return;
        }
        for (ex, segs) in wt.existence.iter().zip(&plan.existence) {
            if ex.min < 1 {
                continue;
            }
            let Some(segments) = segs else {
                continue;
            };
            let Some((last, intermediate)) = segments.split_last() else {
                continue;
            };
            // An existence constraint on an attribute no RM class declares cannot
            // be satisfied by a conformant instance — enforcing it would demand a
            // member the canonical reader refuses. Skipped here and REPORTED by
            // `unenforceable_existence_constraints`, which shares this predicate;
            // see `UnenforceableReason::AttributeNotInRmModel` for the shapes and
            // their RM citations.
            if !rm_declares_attribute(&last.attribute) {
                continue;
            }
            // Navigate the intermediate segments to the container node(s).
            let containers =
                rmpath::navigate_matched(&[instance], intermediate, &plan.name_fallback);
            for container in &containers {
                if attr_absent(container, &last.attribute) {
                    self.push(
                        ex.path.clone(),
                        format!(
                            "mandatory attribute '{}' is missing (existence {}..)",
                            last.attribute, ex.min
                        ),
                        ValidationKind::Required,
                    );
                }
            }
        }
    }

    /// Resolve a group of `WebTemplate` children sharing an aql path (a single
    /// node, or a set of polymorphic type alternatives) against the instance:
    /// occurrence-check the group, then recurse into each matched instance,
    /// routing it to the conforming alternative.
    #[expect(
        clippy::indexing_slicing,
        reason = "`group.members` holds indices into `wt_parent.children` recorded by `NodeWalk::build` over that same child vector, `members` is non-empty for any recorded group (so `members[0]` exists), and `identity_idx` is an `rposition`/`len() - 1` over `segments` (returned early when empty), so every index and range here is in bounds by construction"
    )]
    fn check_group(
        &mut self,
        parent: &Value,
        wt_parent: &WebTemplateNode,
        group: &ChildGroup,
        names: &SiblingNameIndex,
        name_fallback: &rmpath::NameFallback,
    ) {
        // Segments were parsed once at build time; an empty slice means the child
        // path is not a strict extension of the parent's (or equals it) — the
        // `strip_prefix … else return` / `segments.is_empty()` early-outs, both
        // preserved here.
        let segments = &group.segments;
        if segments.is_empty() {
            return;
        }
        let members: Vec<&WebTemplateNode> = group
            .members
            .iter()
            .map(|&i| &wt_parent.children[i])
            .collect();
        let first = members[0];
        let identity_idx = identity_index(segments);
        // Only the TERMINAL-identity case reads `first.rm_type`: with a trailing
        // plain attribute the identity node is the archetyped `LOCATABLE`
        // intermediate.
        let raw_id_seg = &segments[identity_idx];
        let trailing = &segments[identity_idx + 1..];
        let identity_is_locatable = !trailing.is_empty() || is_locatable(&first.rm_type);
        let structural_id_seg = structural_identity(raw_id_seg, identity_is_locatable);
        let id_seg: &PathSegment = structural_id_seg.as_ref().unwrap_or(raw_id_seg);

        // Navigate the intermediate segments to the container node(s).
        let containers =
            rmpath::navigate_matched(&[parent], &segments[..identity_idx], name_fallback);

        // Occurrences are an *archetype-node* constraint: only checked when the
        // matched node is identified by an archetype-node predicate (at-code /
        // archetype id) on a `LOCATABLE` node that can bear one. Plain RM
        // structural attributes (`context`, `value`, `ism_transition`, …),
        // non-`LOCATABLE` nodes (`EVENT_CONTEXT`, matched structurally above) and
        // `in_context` nodes are governed by RM cardinality/invariants, not
        // archetype occurrences, so they are not occurrence-checked here.
        let occ_applies = identity_is_locatable
            && id_seg.predicate.archetype_node_id.is_some()
            && !members.iter().any(|c| c.in_context == Some(true));
        let (group_min, group_max) = group_occurrences(&members);

        for container in &containers {
            let matched = select_group_children(container, id_seg, names);
            if occ_applies {
                self.emit_occurrences(&first.aql_path, group_min, group_max, matched.len());
            }
            for node in matched {
                self.walk_group_targets(node, trailing, &members, name_fallback);
            }
        }
    }

    /// Walks every target the trailing segments reach under one matched node.
    ///
    /// A single-member group walks its one template node; a choice group is
    /// dispatched to [`Self::visit_choice`].
    fn walk_group_targets(
        &mut self,
        node: &Value,
        trailing: &[PathSegment],
        members: &[&WebTemplateNode],
        name_fallback: &rmpath::NameFallback,
    ) {
        for target in rmpath::navigate_matched(&[node], trailing, name_fallback) {
            match members {
                [only] => self.walk(target, only),
                _ => self.visit_choice(target, members),
            }
        }
    }

    /// Route a matched instance node to the type-choice alternative it conforms
    /// to; if it conforms to none, report a single `WrongType`.
    #[expect(
        clippy::indexing_slicing,
        reason = "`visit_choice` is only reached from `check_group` with `members.len() > 1`, so `group[0]` exists"
    )]
    fn visit_choice(&mut self, target: &Value, group: &[&WebTemplateNode]) {
        match target.get("_type").and_then(Value::as_str) {
            Some(it) => {
                if let Some(alt) = group.iter().find(|c| subtype::conforms(it, &c.rm_type)) {
                    self.walk(target, alt);
                } else {
                    let expected: Vec<&str> = group.iter().map(|c| c.rm_type.as_str()).collect();
                    self.push(
                        &group[0].aql_path,
                        format!(
                            "value type {it} conforms to none of the permitted types [{}]",
                            expected.join(", ")
                        ),
                        ValidationKind::WrongType,
                    );
                }
            }
            // No `_type`: recurse against the first alternative; the RM-invariant
            // pass covers structural issues.
            None => self.walk(target, group[0]),
        }
    }

    /// Emit occurrence violations for a matched-node `count` against `[min, max]`
    /// (`max == -1` is unbounded).
    // NOTE: evaluation uses the WebTemplate's flattened `(min, max)` integers
    // (`-1` unbounded) — equivalent to `Multiplicity_interval.has(count)` (BASE
    // `…foundation_types.multiplicity_interval.adoc`) for OPT 1.4's closed bounds.
    fn emit_occurrences(&mut self, aql_path: &str, min: i32, max: i32, count: usize) {
        let count_i = i32::try_from(count).unwrap_or(i32::MAX);
        // The lower-bound occurrences checks (absent / too-few) are relaxed away
        // for a `553|incomplete|` commit; the upper bound still applies
        // (master06 §"Incomplete Content": lower limits set to zero).
        if !self.relax_lower_bounds {
            if count == 0 {
                if min >= 1 {
                    self.push(
                        aql_path,
                        format!("mandatory node is missing (occurrences {min}..)"),
                        ValidationKind::Required,
                    );
                }
            } else if count_i < min {
                self.push(
                    aql_path,
                    format!("too few occurrences: found {count}, expected at least {min}"),
                    ValidationKind::Occurrences,
                );
            }
        }
        if max != -1 && count_i > max {
            self.push(
                aql_path,
                format!("too many occurrences: found {count}, expected at most {max}"),
                ValidationKind::Occurrences,
            );
        }
    }

    /// Container-cardinality check: for each constraining cardinality on this
    /// node, count the children under the constrained attribute path and compare
    /// to `min`/`max`. Iterates [`WebTemplateNode::card_all`] — the full AOM 1.4
    /// §cardinality set (`0..1`/`1..1`/`1..*` included), a superset of the
    /// serialized `cardinalities` (which the metadata document filters).
    fn check_cardinalities(&mut self, instance: &Value, wt: &WebTemplateNode, plan: &NodeWalk) {
        for (card, segs) in wt.card_all.iter().zip(&plan.cardinalities) {
            let Some(segments) = segs else {
                continue;
            };
            let Some((last, intermediate)) = segments.split_last() else {
                continue;
            };
            // Navigate all but the last segment to the container, then count the
            // last attribute's children (cardinality is over the whole set).
            //
            // AOM 1.4 §cardinality constrains the container's membership only WHEN
            // the attribute is present; absence is C_ATTRIBUTE.existence's business
            // (the vendored Multi_list template pairs `content` cardinality 1..*
            // with existence 0..1), so an absent or null attribute is no cardinality
            // violation — and the RM list invariants forbid a present-empty `[]`.
            let containers =
                rmpath::navigate_matched(&[instance], intermediate, &plan.name_fallback);
            for container in &containers {
                if matches!(container.get(&last.attribute), None | Some(Value::Null)) {
                    continue;
                }
                let count =
                    i32::try_from(openehr_rm::v1_2::paths::select_children(container, last).len())
                        .unwrap_or(i32::MAX);
                let min = card.min.unwrap_or(0).max(0);
                // The cardinality lower bound is relaxed away for a
                // `553|incomplete|` commit (master06 §"Incomplete Content": "all
                // existence and cardinality lower limits set to zero"); the upper
                // bound still applies.
                if count < min && !self.relax_lower_bounds {
                    self.push(
                        card.path.clone(),
                        format!("cardinality violated: {count} children, expected at least {min}"),
                        ValidationKind::Cardinality,
                    );
                }
                if card.max != -1 && count > card.max {
                    self.push(
                        card.path.clone(),
                        format!(
                            "cardinality violated: {count} children, expected at most {}",
                            card.max
                        ),
                        ValidationKind::Cardinality,
                    );
                }
            }
        }
    }
}

// ── path navigation ───────────────────────────────────────────────────────────
//
// RM-path parsing (`[atNNNN]` / `[archetype-id]` / `[atNNNN,'name']` predicates)
// and per-step navigation over the canonical-JSON tree are the single
// implementation in [`openehr_rm::v1_2::paths`], reached here via [`crate::flat::rmpath`]
// (`parse` / `navigate` / `select_children`). Only the checks below —
// attribute-presence for the existence rule and RM instance-path
// normalisation — are validation-specific.

/// Select the instance children a WebTemplate sibling group claims from
/// `container`, applying the name discriminator for same-`archetype_node_id`
/// siblings so an instance is routed to exactly the sibling whose name
/// constraint it satisfies (RM common `master03-archetyped_package.adoc`
/// §"The `LOCATABLE` class": the runtime `name` distinguishes sibling nodes
/// that share an `archetype_node_id`; BASE
/// `architecture_overview/master11-paths.adoc` §"Using a Name-based Predicate").
///
/// `names` is the [`SiblingNameIndex`] computed once per parent from the sibling
/// paths. Three cases on the group's identity segment `id_seg`:
///
/// * **name-qualified, name-differentiated** (`Some` name, identity in `names`):
///   strict `(archetype_node_id, name)` match with the id-only fallback OFF —
///   the sibling set relies on the name, so widening to id-only would claim a
///   sibling's instances.
/// * **unqualified, name-differentiated** (no name, identity in `names`): match
///   by `archetype_node_id` minus the instances the name-qualified siblings
///   claim ([`rmpath::select_children_excluding_names`]) — the residual arm.
/// * **not name-differentiated** (identity absent from `names`): strict match
///   with the id-only fallback ON, tolerating a runtime-renamed instance when
///   the archetype does not constrain the name (master03 §"The `LOCATABLE`
///   class" L35).
fn select_group_children<'a>(
    container: &'a Value,
    id_seg: &PathSegment,
    names: &SiblingNameIndex,
) -> Vec<&'a Value> {
    let claimed = id_seg
        .predicate
        .archetype_node_id
        .as_ref()
        .and_then(|id| names.get(&(id_seg.attribute.clone(), id.clone())));
    match (id_seg.predicate.name_value.as_ref(), claimed) {
        (Some(_), Some(_)) => rmpath::select_children_matched(container, id_seg, false),
        (None, Some(claimed)) => {
            rmpath::select_children_excluding_names(container, id_seg, claimed)
        }
        (_, None) => rmpath::select_children_matched(container, id_seg, true),
    }
}

/// The index of the group's identity segment.
///
/// That is the last segment carrying a predicate; if none carries one, the last
/// segment (a plain single-valued attribute).
fn identity_index(segments: &[PathSegment]) -> usize {
    segments
        .iter()
        .rposition(|s| !s.predicate.is_empty())
        .unwrap_or(segments.len() - 1)
}

/// The predicate-stripped identity segment for a structurally-matched node.
///
/// A node whose RM type does NOT inherit `LOCATABLE` carries no
/// `archetype_node_id` in canonical JSON (only `LOCATABLE` adds it — RM common
/// `UML/classes/org.openehr.rm.common.locatable.adoc`; `EVENT_CONTEXT` inherits
/// `PATHABLE` directly — RM
/// `UML/classes/org.openehr.rm.composition.event_context.adoc` §Inherit), so it
/// is matched by attribute position instead. [`None`] means the raw segment
/// applies unchanged.
fn structural_identity(
    raw_id_seg: &PathSegment,
    identity_is_locatable: bool,
) -> Option<PathSegment> {
    if identity_is_locatable || raw_id_seg.predicate.is_empty() {
        return None;
    }
    let mut stripped = raw_id_seg.clone();
    stripped.predicate = openehr_rm::v1_2::paths::Predicate::default();
    Some(stripped)
}

/// The group's combined occurrence bounds: the loosest of its members'.
fn group_occurrences(members: &[&WebTemplateNode]) -> (i32, i32) {
    let min = members
        .iter()
        .filter_map(|c| c.min)
        .min()
        .unwrap_or(0)
        .max(0);
    let max = if members.iter().any(|c| c.max == -1) {
        -1
    } else {
        members.iter().map(|c| c.max).max().unwrap_or(-1)
    };
    (min, max)
}

/// Whether an RM type inherits `LOCATABLE` and therefore carries an
/// `archetype_node_id` (and `name`) in canonical JSON (RM common
/// `UML/classes/org.openehr.rm.common.locatable.adoc`). Backed by the
/// BMM-generated RM inheritance graph ([`openehr_rm::v1_2::model::is_a`]). A type the
/// model does not recognise is treated as `LOCATABLE` (the historic default —
/// stay strict rather than silently widen matching for an unknown type). Generic
/// arguments are stripped first.
fn is_locatable(rm_type: &str) -> bool {
    let base = rm_type.split('<').next().unwrap_or(rm_type).trim();
    openehr_rm::v1_2::model::class(base).is_none()
        || openehr_rm::v1_2::model::is_a(base, "LOCATABLE")
}

/// The instance child objects directly under `attr` (array elements or a single
/// object), skipping non-object values — the sibling set for closed-world.
fn children_under_attr<'a>(node: &'a Value, attr: &str) -> Vec<&'a Value> {
    match node.get(attr) {
        Some(Value::Array(a)) => a.iter().filter(|v| v.is_object()).collect(),
        Some(v @ Value::Object(_)) => vec![v],
        _ => Vec::new(),
    }
}

/// Whether an open `ARCHETYPE_SLOT` admits a filler of RM type `child_type` and
/// archetype id `archetype_id` (AOM 1.4 `ARCHETYPE_SLOT`): the type must conform
/// to the slot's `rm_type`, the id must match at least one `includes` regex (an
/// empty `includes` = open to the type), and match no `excludes` regex. A blanket
/// match-all (`.*`) exclude is ignored when `includes` is non-empty — the ADL 1.4
/// closed-slot idiom (AOM 1.4 has no `is_closed`; NOTE: includes then win,
/// matching de-facto CDR behaviour).
fn slot_admits(slot: &WebTemplateArchetypeSlot, child_type: &str, archetype_id: &str) -> bool {
    if !slot.rm_type.is_empty() && !subtype::conforms(child_type, &slot.rm_type) {
        return false;
    }
    let include_ok = slot.includes.is_empty()
        || slot
            .includes
            .iter()
            .any(|p| leaf::matches_pattern(p, archetype_id));
    if !include_ok {
        return false;
    }
    for ex in &slot.excludes {
        if !slot.includes.is_empty() && matches!(ex.trim(), ".*" | ".+") {
            continue; // closed-slot idiom: a specific includes list wins.
        }
        if leaf::matches_pattern(ex, archetype_id) {
            return false;
        }
    }
    true
}

/// Whether an RM attribute field is absent (missing, JSON `null`, or an empty
/// array) on a node — the negation of "the attribute is present" for the
/// existence check.
fn attr_absent(node: &Value, attr: &str) -> bool {
    match node.get(attr) {
        None | Some(Value::Null) => true,
        Some(Value::Array(a)) => a.is_empty(),
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    //! Per-rule unit tests for the composition validator, built on hand-shaped
    //! `WebTemplate` nodes + minimal instances (no OPT parsing) so each rule is
    //! exercised in isolation through the private [`Validator`] walk. End-to-end
    //! corpus + public-seam tests live in `tests/validation.rs`.

    use serde_json::{Value, json};

    use super::*;
    use crate::flat::webtemplate::model::{
        WebTemplateCardinality, WebTemplateClosedAttribute, WebTemplateCodeList,
        WebTemplateCodedValue, WebTemplateExistence, WebTemplateInput, WebTemplateInputType,
        WebTemplateRange, WebTemplateSlot, WebTemplateValidation,
    };

    fn node(rm: &str, path: &str) -> WebTemplateNode {
        WebTemplateNode::new(rm.to_owned(), path.to_owned())
    }

    /// Run only the `WebTemplate` (archetype-conformance) pass for a matched root.
    fn walk_only(instance: &Value, root: &WebTemplateNode) -> Vec<ValidationMessage> {
        let mut v = Validator::default();
        v.walk(instance, root);
        v.out
    }

    fn kinds(msgs: &[ValidationMessage]) -> Vec<ValidationKind> {
        msgs.iter().map(|m| m.kind).collect()
    }

    // ── occurrences ──────────────────────────────────────────────────────────

    #[test]
    fn occurrences_too_few() {
        let mut root = node("COMPOSITION", "");
        let mut sec = node("SECTION", "/content[at0001]");
        sec.min = Some(2);
        sec.max = 5;
        root.children = vec![sec];

        let inst = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x",
            "content": [{"_type": "SECTION", "archetype_node_id": "at0001",
                         "name": {"_type": "DV_TEXT", "value": "s"}}]
        });
        let msgs = walk_only(&inst, &root);
        assert!(
            kinds(&msgs).contains(&ValidationKind::Occurrences),
            "expected Occurrences (too few), got {msgs:?}"
        );
    }

    #[test]
    fn occurrences_too_many() {
        let mut root = node("COMPOSITION", "");
        let mut sec = node("SECTION", "/content[at0001]");
        sec.min = Some(0);
        sec.max = 1;
        root.children = vec![sec];

        let one = json!({"_type": "SECTION", "archetype_node_id": "at0001",
                         "name": {"_type": "DV_TEXT", "value": "s"}});
        let inst = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x",
            "content": [one.clone(), one.clone(), one]
        });
        let msgs = walk_only(&inst, &root);
        assert!(
            msgs.iter()
                .any(|m| m.kind == ValidationKind::Occurrences && m.message.contains("too many")),
            "expected Occurrences (too many), got {msgs:?}"
        );
    }

    #[test]
    fn occurrences_required_missing() {
        let mut root = node("COMPOSITION", "");
        let mut sec = node("SECTION", "/content[at0001]");
        sec.min = Some(1);
        sec.max = 1;
        root.children = vec![sec];

        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": []});
        let msgs = walk_only(&inst, &root);
        assert!(
            kinds(&msgs).contains(&ValidationKind::Required),
            "expected Required, got {msgs:?}"
        );
    }

    // ── cardinality ────────────────────────────────────────────────────────────

    #[test]
    fn cardinality_violation() {
        let mut root = node("COMPOSITION", "");
        root.card_all = vec![WebTemplateCardinality {
            min: Some(1),
            max: 2,
            ids: None,
            path: "/content".to_owned(),
        }];
        let entry = json!({"_type": "OBSERVATION", "archetype_node_id": "a"});
        let inst = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x",
            "content": [entry.clone(), entry.clone(), entry]
        });
        let msgs = walk_only(&inst, &root);
        assert!(
            kinds(&msgs).contains(&ValidationKind::Cardinality),
            "expected Cardinality (>max), got {msgs:?}"
        );
    }

    // ── incomplete-lifecycle (553) relaxation ────────────────────────────────
    // RM common master06 §"Incomplete Content": existence/occurrences/cardinality
    // lower limits treated as zero; upper limits and value/type constraints stay.

    /// Run the archetype-conformance pass with the `553|incomplete|` relaxation.
    fn walk_incomplete(instance: &Value, root: &WebTemplateNode) -> Vec<ValidationMessage> {
        let mut v = Validator {
            relax_lower_bounds: true,
            ..Validator::default()
        };
        v.walk(instance, root);
        v.out
    }

    #[test]
    fn incomplete_suppresses_required_missing_occurrences() {
        let mut root = node("COMPOSITION", "");
        let mut sec = node("SECTION", "/content[at0001]");
        sec.min = Some(1);
        sec.max = 1;
        root.children = vec![sec];

        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": []});
        // Strict: a mandatory node is missing.
        assert!(kinds(&walk_only(&inst, &root)).contains(&ValidationKind::Required));
        // Incomplete: the lower bound is zeroed, so nothing is emitted.
        assert!(
            walk_incomplete(&inst, &root).is_empty(),
            "incomplete commit must tolerate a missing mandatory node"
        );
    }

    #[test]
    fn incomplete_suppresses_too_few_but_keeps_too_many() {
        // too few (min 2, one present) → suppressed under incomplete.
        let mut root = node("COMPOSITION", "");
        let mut sec = node("SECTION", "/content[at0001]");
        sec.min = Some(2);
        sec.max = 3;
        root.children = vec![sec];
        let one = json!({"_type": "SECTION", "archetype_node_id": "at0001",
                         "name": {"_type": "DV_TEXT", "value": "s"}});
        let too_few = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x", "content": [one.clone()]
        });
        assert!(
            walk_incomplete(&too_few, &root).is_empty(),
            "incomplete commit must tolerate too-few occurrences"
        );

        // too many (three present, max 3 ok; four exceeds) → still enforced (upper
        // bound is not relaxed: missing is tolerated, wrong is not).
        let too_many = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x",
            "content": [one.clone(), one.clone(), one.clone(), one]
        });
        assert!(
            walk_incomplete(&too_many, &root)
                .iter()
                .any(|m| m.kind == ValidationKind::Occurrences && m.message.contains("too many")),
            "incomplete commit must still reject too-many occurrences"
        );
    }

    #[test]
    fn incomplete_suppresses_cardinality_lower_but_keeps_upper() {
        // Lower-bound cardinality violation → suppressed.
        let mut low = node("COMPOSITION", "");
        low.card_all = vec![WebTemplateCardinality {
            min: Some(2),
            max: -1,
            ids: None,
            path: "/content".to_owned(),
        }];
        let entry = json!({"_type": "OBSERVATION", "archetype_node_id": "a"});
        let one_child = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x", "content": [entry.clone()]
        });
        assert!(
            walk_incomplete(&one_child, &low).is_empty(),
            "incomplete commit must tolerate a below-minimum container"
        );

        // Upper-bound cardinality violation → still enforced.
        let mut high = node("COMPOSITION", "");
        high.card_all = vec![WebTemplateCardinality {
            min: Some(1),
            max: 2,
            ids: None,
            path: "/content".to_owned(),
        }];
        let three = json!({
            "_type": "COMPOSITION", "archetype_node_id": "x",
            "content": [entry.clone(), entry.clone(), entry]
        });
        assert!(
            kinds(&walk_incomplete(&three, &high)).contains(&ValidationKind::Cardinality),
            "incomplete commit must still reject an over-maximum container"
        );
    }

    #[test]
    fn incomplete_suppresses_existence() {
        let mut root = node("COMPOSITION", "");
        root.existence = vec![WebTemplateExistence {
            min: 1,
            max: 1,
            path: "/context".to_owned(),
        }];
        // The mandatory `context` field is absent.
        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x"});
        assert!(kinds(&walk_only(&inst, &root)).contains(&ValidationKind::Required));
        assert!(
            walk_incomplete(&inst, &root).is_empty(),
            "incomplete commit must tolerate a missing mandatory attribute (existence)"
        );
    }

    // ── numeric range ────────────────────────────────────────────────────────

    fn count_node_range(min: i64, max: i64) -> WebTemplateNode {
        let mut n = node("DV_COUNT", "/count");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Integer, None);
        input.validation = Some(WebTemplateValidation {
            range: Some(WebTemplateRange {
                min_op: Some(">=".to_owned()),
                min: Some(Value::from(min)),
                max_op: Some("<=".to_owned()),
                max: Some(Value::from(max)),
            }),
            ..Default::default()
        });
        n.inputs = vec![input];
        n
    }

    #[test]
    fn range_error_out_of_bounds() {
        let n = count_node_range(0, 10);
        let inst = json!({"_type": "DV_COUNT", "magnitude": 42});
        let msgs = walk_only(&inst, &n);
        assert_eq!(kinds(&msgs), vec![ValidationKind::RangeError], "{msgs:?}");
    }

    #[test]
    fn range_ok_in_bounds() {
        let n = count_node_range(0, 10);
        let inst = json!({"_type": "DV_COUNT", "magnitude": 5});
        assert!(walk_only(&inst, &n).is_empty());
    }

    // ── DV_MULTIMEDIA.size (C_INTEGER list / range) ──────────────────────────
    // RM `data_types` §DV_MULTIMEDIA (`size: Integer`); AOM 1.4
    // `master04-constraint_model_package.adoc` §C_INTEGER (list + range).

    #[test]
    fn multimedia_size_out_of_range_reported() {
        let mut n = node("DV_MULTIMEDIA", "/media");
        n.inputs = vec![WebTemplateInput::new(WebTemplateInputType::Text, None)];
        n.numeric_ranges = vec![(
            "size".to_owned(),
            WebTemplateRange {
                min_op: Some(">=".to_owned()),
                min: Some(Value::from(200)),
                max_op: Some("<=".to_owned()),
                max: Some(Value::from(1000)),
            },
        )];
        let bad = json!({"_type": "DV_MULTIMEDIA", "uri": "http://x", "size": 123});
        assert_eq!(
            kinds(&walk_only(&bad, &n)),
            vec![ValidationKind::RangeError]
        );
        let good = json!({"_type": "DV_MULTIMEDIA", "uri": "http://x", "size": 500});
        assert!(walk_only(&good, &n).is_empty());
    }

    #[test]
    fn multimedia_size_not_in_list_reported() {
        let mut n = node("DV_MULTIMEDIA", "/media");
        n.inputs = vec![WebTemplateInput::new(WebTemplateInputType::Text, None)];
        n.numeric_lists = vec![("size".to_owned(), vec![10.0, 100.0, 1000.0])];
        let bad = json!({"_type": "DV_MULTIMEDIA", "uri": "http://x", "size": 123});
        assert_eq!(
            kinds(&walk_only(&bad, &n)),
            vec![ValidationKind::CodedValue]
        );
        let good = json!({"_type": "DV_MULTIMEDIA", "uri": "http://x", "size": 100});
        assert!(walk_only(&good, &n).is_empty());
    }

    // ── DV_IDENTIFIER mandatory (existence 1..1) constrained sub-attribute ────
    // RM `data_types` §DV_IDENTIFIER; AOM 1.4 §existence + §C_STRING. An OPT that
    // constrains and mandates `issuer` rejects a value that omits it (the `id`
    // absence is caught separately by the RM invariant).

    #[test]
    fn dv_identifier_mandatory_issuer_enforced() {
        let mut n = node("DV_IDENTIFIER", "/value");
        let mut issuer = WebTemplateInput::new(WebTemplateInputType::Text, Some("issuer"));
        issuer.list = vec![WebTemplateCodedValue::new("XYZ", Some("XYZ".to_owned()))];
        issuer.list_open = Some(false);
        n.inputs = vec![
            WebTemplateInput::new(WebTemplateInputType::Text, Some("id")),
            issuer,
        ];
        n.existence = vec![WebTemplateExistence {
            min: 1,
            max: 1,
            path: "/value/issuer".to_owned(),
        }];
        // issuer absent → Required.
        let absent = json!({"_type": "DV_IDENTIFIER", "id": "x"});
        assert!(kinds(&walk_only(&absent, &n)).contains(&ValidationKind::Required));
        // issuer present but not in the list → CodedValue.
        let wrong = json!({"_type": "DV_IDENTIFIER", "id": "x", "issuer": "ABC"});
        assert!(kinds(&walk_only(&wrong, &n)).contains(&ValidationKind::CodedValue));
        // issuer present and conforming → clean.
        let ok = json!({"_type": "DV_IDENTIFIER", "id": "x", "issuer": "XYZ"});
        assert!(walk_only(&ok, &n).is_empty());
    }

    // ── DV_SCALE generic (C_REAL value list + symbol C_CODE_PHRASE) ───────────
    // AOM 1.4 has no C_DV_SCALE, so DV_SCALE constrains `symbol.defining_code`
    // as a C_CODE_PHRASE code_list (AOM 1.4 §C_CODE_PHRASE; RM §DV_SCALE): a
    // symbol not in the list is rejected, with no (symbol, value) pair check.

    #[test]
    fn dv_scale_generic_symbol_membership_enforced() {
        let mut n = node("DV_SCALE", "/value");
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, None);
        // Generic form: coded symbols carry no scale/ordinal number.
        input.list = vec![
            WebTemplateCodedValue::new("at0005", Some("mild".to_owned())),
            WebTemplateCodedValue::new("at0006", Some("severe".to_owned())),
        ];
        n.inputs = vec![input];
        n.numeric_lists = vec![("value".to_owned(), vec![1.5, 2.4])];
        // Symbol not in the code list (value in the real list) → CodedValue.
        let bad = json!({
            "_type": "DV_SCALE", "value": 1.5,
            "symbol": {"_type": "DV_CODED_TEXT", "value": "?",
                "defining_code": {"_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                    "code_string": "at0666"}}
        });
        assert!(kinds(&walk_only(&bad, &n)).contains(&ValidationKind::CodedValue));
        // Symbol in the list, value in the list → clean (no spurious pair check).
        let ok = json!({
            "_type": "DV_SCALE", "value": 1.5,
            "symbol": {"_type": "DV_CODED_TEXT", "value": "mild",
                "defining_code": {"_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                    "code_string": "at0005"}}
        });
        assert!(walk_only(&ok, &n).is_empty(), "{:?}", walk_only(&ok, &n));
    }

    // ── DV_TIME partial value vs C_TIME pattern (over-rejection fix) ──────────
    // ADL 1.4 `master05-cadl.adoc` §"Date, Time and Date/Time" Patterns: `?` =
    // optional field, `X` = disallowed field; an hour-only value "10" satisfies
    // both "HH:??:??" and "HH:XX:XX".

    fn time_node(pattern: &str) -> WebTemplateNode {
        let mut n = node("DV_TIME", "/time");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Time, None);
        input.validation = Some(WebTemplateValidation {
            pattern: Some(pattern.to_owned()),
            ..Default::default()
        });
        n.inputs = vec![input];
        n
    }

    #[test]
    fn dv_time_hour_only_accepts_optional_and_prohibited_patterns() {
        for pattern in ["HH:??:??", "HH:XX:XX"] {
            let n = time_node(pattern);
            let inst = json!({"_type": "DV_TIME", "value": "10"});
            assert!(
                walk_only(&inst, &n).is_empty(),
                "hour-only \"10\" must satisfy pattern {pattern}: {:?}",
                walk_only(&inst, &n)
            );
        }
    }

    // ── string pattern ─────────────────────────────────────────────────────────

    #[test]
    fn pattern_error() {
        let mut n = node("DV_TEXT", "/text");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Text, None);
        input.validation = Some(WebTemplateValidation {
            pattern: Some("[A-Z]+".to_owned()),
            ..Default::default()
        });
        n.inputs = vec![input];

        let bad = json!({"_type": "DV_TEXT", "value": "abc"});
        assert_eq!(
            kinds(&walk_only(&bad, &n)),
            vec![ValidationKind::PatternError]
        );
        let good = json!({"_type": "DV_TEXT", "value": "ABC"});
        assert!(walk_only(&good, &n).is_empty());
    }

    // ── coded value membership ─────────────────────────────────────────────────

    fn coded_node(codes: &[&str]) -> WebTemplateNode {
        let mut n = node("DV_CODED_TEXT", "/coded");
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
        input.list = codes
            .iter()
            .map(|c| WebTemplateCodedValue::new(*c, Some((*c).to_owned())))
            .collect();
        n.inputs = vec![input];
        n
    }

    #[test]
    fn coded_value_not_in_list() {
        let n = coded_node(&["at0001", "at0002"]);
        let bad = json!({
            "_type": "DV_CODED_TEXT", "value": "x",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                "code_string": "at0099"}
        });
        assert_eq!(
            kinds(&walk_only(&bad, &n)),
            vec![ValidationKind::CodedValue]
        );

        let good = json!({
            "_type": "DV_CODED_TEXT", "value": "x",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                "code_string": "at0001"}
        });
        assert!(walk_only(&good, &n).is_empty());
    }

    #[test]
    fn coded_value_external_terminology_is_skipped() {
        // A SNOMED code is not validated against the archetype's internal list.
        let n = coded_node(&["at0001"]);
        let external = json!({
            "_type": "DV_CODED_TEXT", "value": "x",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "SNOMED-CT"},
                "code_string": "999"}
        });
        assert!(walk_only(&external, &n).is_empty());
    }

    // ── type conformance ───────────────────────────────────────────────────────

    #[test]
    fn wrong_type_reported() {
        let n = node("DV_QUANTITY", "/q");
        let inst = json!({"_type": "DV_TEXT", "value": "x"});
        assert_eq!(
            kinds(&walk_only(&inst, &n)),
            vec![ValidationKind::WrongType]
        );
    }

    #[test]
    fn coded_text_in_text_slot_conforms() {
        // DV_CODED_TEXT is-a DV_TEXT: no WrongType.
        let n = node("DV_TEXT", "/t");
        let inst = json!({
            "_type": "DV_CODED_TEXT", "value": "x",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                "code_string": "at0001"}
        });
        assert!(
            !kinds(&walk_only(&inst, &n)).contains(&ValidationKind::WrongType),
            "DV_CODED_TEXT should conform to a DV_TEXT slot"
        );
    }

    // ── AOM 1.4 C_ATTRIBUTE.existence ────────────────────────────────────────

    #[test]
    fn existence_mandatory_attribute_missing_reported() {
        // A node requiring a mandatory `value` attribute (existence {1..1}); the
        // instance omits it → Required.
        let mut n = node("ELEMENT", "/items[at0001]");
        n.existence = vec![WebTemplateExistence {
            min: 1,
            max: 1,
            path: "/items[at0001]/value".to_owned(),
        }];
        let inst = json!({"_type": "ELEMENT", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "e"}});
        let msgs = walk_only(&inst, &n);
        assert!(
            msgs.iter()
                .any(|m| m.kind == ValidationKind::Required && m.path.ends_with("/value")),
            "expected a Required existence violation for the missing value, got {msgs:?}"
        );
    }

    #[test]
    fn existence_present_attribute_is_clean() {
        let mut n = node("ELEMENT", "/items[at0001]");
        n.existence = vec![WebTemplateExistence {
            min: 1,
            max: 1,
            path: "/items[at0001]/value".to_owned(),
        }];
        let inst = json!({"_type": "ELEMENT", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "e"},
            "value": {"_type": "DV_TEXT", "value": "v"}});
        assert!(
            walk_only(&inst, &n).is_empty(),
            "a present mandatory value should be clean"
        );
    }

    #[test]
    fn existence_empty_array_counts_as_absent() {
        let mut n = node("COMPOSITION", "");
        n.existence = vec![WebTemplateExistence {
            min: 1,
            max: -1,
            path: "/content".to_owned(),
        }];
        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": []});
        let msgs = walk_only(&inst, &n);
        assert!(
            msgs.iter().any(|m| m.kind == ValidationKind::Required),
            "an empty mandatory container attribute is absent, got {msgs:?}"
        );
    }

    // ── path parsing ─────────────────────────────────────────────────────────

    #[test]
    fn segment_parsing_respects_brackets() {
        // Parsing routes through the single `openehr_rm::v1_2::paths` implementation
        // via `crate::flat::rmpath`; this asserts the validator sees the same segments.
        let segs = rmpath::parse("/content[openEHR-EHR-SECTION.x.v1]/items[at0004,'Sys']/value");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].attribute, "content");
        assert_eq!(
            segs[0].predicate.archetype_node_id.as_deref(),
            Some("openEHR-EHR-SECTION.x.v1")
        );
        assert_eq!(segs[0].predicate.name_value, None);
        assert_eq!(
            segs[1].predicate.archetype_node_id.as_deref(),
            Some("at0004")
        );
        assert_eq!(segs[1].predicate.name_value.as_deref(), Some("Sys"));
        assert!(segs[2].predicate.is_empty());
    }

    // ── CNF-hardening additions (master15/16/17 truth tables) ─────────────────

    /// `1..*` container cardinality with zero members → Cardinality (master15
    /// CONT-COMP-content_card_1plus; AOM 1.4 §cardinality).
    #[test]
    fn cardinality_one_plus_empty_rejected() {
        let mut root = node("COMPOSITION", "");
        root.card_all = vec![WebTemplateCardinality {
            min: Some(1),
            max: -1,
            ids: None,
            path: "/content".to_owned(),
        }];
        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": []});
        let msgs = walk_only(&inst, &root);
        assert!(
            kinds(&msgs).contains(&ValidationKind::Cardinality),
            "expected Cardinality for 1..* with 0 members, got {msgs:?}"
        );
    }

    /// A bare mandatory attribute (existence `1..1`, no value constraint) must be
    /// present (master15 `context_mand`; AOM 1.4 §existence).
    #[test]
    fn bare_mandatory_attribute_absent_rejected() {
        let mut root = node("COMPOSITION", "");
        root.existence = vec![WebTemplateExistence {
            min: 1,
            max: 1,
            path: "/context".to_owned(),
        }];
        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x"});
        let msgs = walk_only(&inst, &root);
        assert!(
            kinds(&msgs).contains(&ValidationKind::Required),
            "expected Required for absent mandatory context, got {msgs:?}"
        );
    }

    /// A hoisted-wrapper slot narrowed to a concrete subtype rejects a sibling
    /// subtype and accepts the narrowed one; an abstract slot accepts any subtype
    /// (master16 §`ITEM_STRUCTURE/§EVENT` "Class not allowed").
    #[test]
    fn slot_narrowing() {
        let mut eval = node("EVALUATION", "/content[at0001]");
        eval.slots = vec![WebTemplateSlot {
            path: "/content[at0001]/data[at0002]".to_owned(),
            rm_type: "ITEM_LIST".to_owned(),
        }];
        let wrong = json!({"_type": "EVALUATION", "archetype_node_id": "at0001",
            "data": {"_type": "ITEM_TREE", "archetype_node_id": "at0002", "items": []}});
        let msgs = walk_only(&wrong, &eval);
        assert!(
            msgs.iter()
                .any(|m| m.kind == ValidationKind::WrongType && m.message.contains("not allowed")),
            "expected WrongType for ITEM_TREE in an ITEM_LIST slot, got {msgs:?}"
        );
        let right = json!({"_type": "EVALUATION", "archetype_node_id": "at0001",
            "data": {"_type": "ITEM_LIST", "archetype_node_id": "at0002", "items": []}});
        assert!(
            walk_only(&right, &eval).is_empty(),
            "narrowed subtype accepted"
        );

        eval.slots[0].rm_type = "ITEM_STRUCTURE".to_owned();
        let any = json!({"_type": "EVALUATION", "archetype_node_id": "at0001",
            "data": {"_type": "ITEM_TABLE", "archetype_node_id": "at0002", "rows": []}});
        assert!(
            walk_only(&any, &eval).is_empty(),
            "abstract ITEM_STRUCTURE slot admits any subtype"
        );
    }

    /// `C_INTEGER.list` on `DV_COUNT.magnitude` (master17.3 CONT-DV_COUNT-validate_list).
    #[test]
    fn numeric_list_membership() {
        let mut count = node("DV_COUNT", "/value");
        count.inputs = vec![WebTemplateInput::new(WebTemplateInputType::Integer, None)];
        count.numeric_lists = vec![("magnitude".to_owned(), vec![3.0])];
        let bad = json!({"_type": "DV_COUNT", "magnitude": 7});
        let msgs = walk_only(&bad, &count);
        assert!(
            kinds(&msgs).contains(&ValidationKind::CodedValue),
            "expected CodedValue for magnitude off the list, got {msgs:?}"
        );
        let good = json!({"_type": "DV_COUNT", "magnitude": 3});
        assert!(walk_only(&good, &count).is_empty());
    }

    /// `DV_PROPORTION` `type` kind membership (master17.3 CONT-DV_PROPORTION-*).
    #[test]
    fn proportion_kind_membership() {
        let mut prop = node("DV_PROPORTION", "/value");
        prop.inputs = vec![WebTemplateInput::new(
            WebTemplateInputType::Decimal,
            Some("numerator"),
        )];
        prop.proportion_types = vec!["percent".to_owned()];
        let bad =
            json!({"_type": "DV_PROPORTION", "numerator": 1.0, "denominator": 2.0, "type": 0});
        let msgs = walk_only(&bad, &prop);
        assert!(
            kinds(&msgs).contains(&ValidationKind::CodedValue),
            "expected CodedValue for ratio where percent required, got {msgs:?}"
        );
        let good =
            json!({"_type": "DV_PROPORTION", "numerator": 42.0, "denominator": 100.0, "type": 2});
        assert!(walk_only(&good, &prop).is_empty());
    }

    /// `C_QUANTITY_ITEM.precision` (`OpenehrProfile.xsd`, surfaced as
    /// `inputs[].validation.precision`): the instance's `DV_QUANTITY.precision`
    /// must lie in the interval; an absent instance precision violates
    /// nothing (#3027).
    #[test]
    fn quantity_precision_interval() {
        let mut q = node("DV_QUANTITY", "/q");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Decimal, Some("magnitude"));
        input.validation = Some(WebTemplateValidation {
            pattern: None,
            range: None,
            precision: Some(WebTemplateRange {
                min_op: Some(">=".to_owned()),
                min: Some(json!(0)),
                max_op: Some("<=".to_owned()),
                max: Some(json!(1)),
            }),
        });
        q.inputs = vec![input];

        let over =
            json!({"_type": "DV_QUANTITY", "magnitude": 5.25, "units": "kg", "precision": 3});
        assert!(
            kinds(&walk_only(&over, &q)).contains(&ValidationKind::RangeError),
            "precision 3 violates [0, 1]"
        );
        let ok = json!({"_type": "DV_QUANTITY", "magnitude": 5.2, "units": "kg", "precision": 1});
        assert!(walk_only(&ok, &q).is_empty());
        let absent = json!({"_type": "DV_QUANTITY", "magnitude": 5.2, "units": "kg"});
        assert!(
            walk_only(&absent, &q).is_empty(),
            "an absent optional precision violates nothing"
        );
    }

    /// `DV_PROPORTION.precision` under the same interval contract as
    /// `DV_QUANTITY` (the `|precision` suffix in the `master05` table; #3027).
    #[test]
    fn proportion_precision_interval() {
        let mut prop = node("DV_PROPORTION", "/p");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Decimal, Some("numerator"));
        input.validation = Some(WebTemplateValidation {
            pattern: None,
            range: None,
            precision: Some(WebTemplateRange {
                min_op: Some(">=".to_owned()),
                min: Some(json!(0)),
                max_op: Some("<=".to_owned()),
                max: Some(json!(0)),
            }),
        });
        prop.inputs = vec![input];
        let bad = json!({"_type": "DV_PROPORTION", "numerator": 42.0, "denominator": 100.0,
            "type": 2, "precision": 2});
        assert!(
            kinds(&walk_only(&bad, &prop)).contains(&ValidationKind::RangeError),
            "precision 2 violates [0, 0]"
        );
        let ok = json!({"_type": "DV_PROPORTION", "numerator": 42.0, "denominator": 100.0,
            "type": 2, "precision": 0});
        assert!(walk_only(&ok, &prop).is_empty());
    }

    /// `C_DATE` pattern + range (master17.4 CONT-DV_DATE-validate_constraint/-range).
    #[test]
    fn temporal_pattern_and_range() {
        let mut date = node("DV_DATE", "/value");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Date, None);
        input.validation = Some(WebTemplateValidation {
            pattern: Some("yyyy-mm-dd".to_owned()),
            range: Some(WebTemplateRange {
                min_op: Some(">=".to_owned()),
                min: Some(json!("2021-01-01")),
                max_op: Some("<=".to_owned()),
                max: Some(json!("2021-12-31")),
            }),
            precision: None,
        });
        date.inputs = vec![input];

        let partial = json!({"_type": "DV_DATE", "value": "2021"});
        assert!(
            kinds(&walk_only(&partial, &date)).contains(&ValidationKind::PatternError),
            "partial date violates yyyy-mm-dd"
        );
        let out = json!({"_type": "DV_DATE", "value": "2025-06-01"});
        assert!(
            kinds(&walk_only(&out, &date)).contains(&ValidationKind::RangeError),
            "date outside the range"
        );
        let ok = json!({"_type": "DV_DATE", "value": "2021-10-18"});
        assert!(walk_only(&ok, &date).is_empty());
    }

    /// `C_TIME` pattern: a partial time violates HH:MM:SS (master17.4 CONT-DV_TIME).
    #[test]
    fn time_pattern_partial_rejected() {
        let mut time = node("DV_TIME", "/value");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Time, None);
        input.validation = Some(WebTemplateValidation {
            pattern: Some("HH:MM:SS".to_owned()),
            range: None,
            precision: None,
        });
        time.inputs = vec![input];
        assert!(
            kinds(&walk_only(
                &json!({"_type": "DV_TIME", "value": "22"}),
                &time
            ))
            .contains(&ValidationKind::PatternError)
        );
        assert!(walk_only(&json!({"_type": "DV_TIME", "value": "22:18:16"}), &time).is_empty());
    }

    /// `C_DURATION` allowed fields + range (master17.4 CONT-DV_DURATION-*).
    #[test]
    fn duration_fields_and_range() {
        let mut dur = node("DV_DURATION", "/value");
        dur.inputs = vec![
            WebTemplateInput::new(WebTemplateInputType::Integer, Some("hour")),
            WebTemplateInput::new(WebTemplateInputType::Integer, Some("minute")),
        ];
        dur.duration_range = Some(WebTemplateRange {
            min_op: Some(">=".to_owned()),
            min: Some(json!("PT0S")),
            max_op: Some("<=".to_owned()),
            max: Some(json!("PT1H")),
        });
        assert!(
            kinds(&walk_only(
                &json!({"_type": "DV_DURATION", "value": "P1Y"}),
                &dur
            ))
            .contains(&ValidationKind::PatternError),
            "year field forbidden by the pattern"
        );
        assert!(
            kinds(&walk_only(
                &json!({"_type": "DV_DURATION", "value": "PT5H"}),
                &dur
            ))
            .contains(&ValidationKind::RangeError),
            "PT5H outside [PT0S,PT1H]"
        );
        assert!(walk_only(&json!({"_type": "DV_DURATION", "value": "PT30M"}), &dur).is_empty());
    }

    /// An enumerated **external** `C_CODE_PHRASE` list constrains membership
    /// (master17.2 CONT-DV_CODED_TEXT-validate_ext_term; AOM 1.4 §`C_CODE_PHRASE`).
    #[test]
    fn external_code_list_membership() {
        let mut coded = node("DV_CODED_TEXT", "/value");
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
        input.terminology = Some("SNOMED-CT".to_owned());
        input.list = vec![WebTemplateCodedValue::new("73211009", None)];
        coded.inputs = vec![input];

        let bad = json!({"_type": "DV_CODED_TEXT", "value": "x", "defining_code": {
            "terminology_id": {"value": "SNOMED-CT"}, "code_string": "99999999"}});
        assert!(
            kinds(&walk_only(&bad, &coded)).contains(&ValidationKind::CodedValue),
            "external code off the enumerated list"
        );
        let good = json!({"_type": "DV_CODED_TEXT", "value": "x", "defining_code": {
            "terminology_id": {"value": "SNOMED-CT"}, "code_string": "73211009"}});
        assert!(walk_only(&good, &coded).is_empty());
        // A code from a different terminology than the constraint's is not judged.
        let other = json!({"_type": "DV_CODED_TEXT", "value": "x", "defining_code": {
            "terminology_id": {"value": "ICD10"}, "code_string": "A00"}});
        assert!(walk_only(&other, &coded).is_empty());
    }

    /// `C_CODE_PHRASE` on a coded attribute outside `defining_code`
    /// (`DV_MULTIMEDIA.media_type` — master17.6 CONT-DV_MULTIMEDIA-validate_media_type).
    #[test]
    fn media_type_code_list() {
        let mut mm = node("DV_MULTIMEDIA", "/value");
        mm.inputs = vec![WebTemplateInput::new(WebTemplateInputType::Text, None)];
        mm.code_lists = vec![WebTemplateCodeList {
            attr: "media_type".to_owned(),
            terminology: Some("IANA_media-types".to_owned()),
            codes: vec!["image/png".to_owned()],
        }];
        let bad = json!({"_type": "DV_MULTIMEDIA", "size": 1, "media_type": {
            "terminology_id": {"value": "IANA_media-types"}, "code_string": "image/gif"}});
        assert!(
            kinds(&walk_only(&bad, &mm)).contains(&ValidationKind::CodedValue),
            "media_type off the enumerated list"
        );
        let good = json!({"_type": "DV_MULTIMEDIA", "size": 1, "media_type": {
            "terminology_id": {"value": "IANA_media-types"}, "code_string": "image/png"}});
        assert!(walk_only(&good, &mm).is_empty());
    }

    // ── closed-archetype walk ─────────────────────────────────────────────────

    /// A COMPOSITION whose `content` is closed to `openEHR-EHR-SECTION.x.v1`: the
    /// defined section is accepted, a foreign OBSERVATION is rejected as unexpected
    /// (closed-world rule 1).
    #[test]
    fn closed_world_rejects_foreign_content() {
        let mut root = node("COMPOSITION", "");
        root.closed_attributes = vec![WebTemplateClosedAttribute {
            path: "/content".to_owned(),
            allowed_ids: vec!["openEHR-EHR-SECTION.x.v1".to_owned()],
            slots: vec![],
        }];
        let inst = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": [
            {"_type": "SECTION", "archetype_node_id": "openEHR-EHR-SECTION.x.v1",
             "name": {"_type": "DV_TEXT", "value": "s"}},
            {"_type": "OBSERVATION", "archetype_node_id": "openEHR-EHR-OBSERVATION.foreign.v1",
             "name": {"_type": "DV_TEXT", "value": "o"}}
        ]});
        // The closed-world admission rule: an unmatched *archetype-rooted* child is
        // tolerated (the flat OPT does not enumerate the full slot-fill universe;
        // the CNF corpus itself commits such ENTRYs).
        let msgs = walk_only(&inst, &root);
        assert!(
            msgs.is_empty(),
            "foreign archetype-rooted content is tolerated (the closed-world admission rule), got {msgs:?}"
        );
        // At-coded children remain closed: an at-coded child matching no sibling
        // constraint is rejected (closed-world rule 1).
        let at_foreign = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": [
            {"_type": "SECTION", "archetype_node_id": "at0099",
             "name": {"_type": "DV_TEXT", "value": "s"}}
        ]});
        let msgs = walk_only(&at_foreign, &root);
        assert!(
            msgs.iter()
                .any(|m| m.kind == ValidationKind::Unexpected && m.message.contains("at0099")),
            "expected an Unexpected violation for the foreign at-coded child, got {msgs:?}"
        );
        let ok = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": [
            {"_type": "SECTION", "archetype_node_id": "openEHR-EHR-SECTION.x.v1",
             "name": {"_type": "DV_TEXT", "value": "s"}}]});
        assert!(
            walk_only(&ok, &root).is_empty(),
            "the defined section is admitted"
        );
    }

    /// A metadata value (no `archetype_node_id`, i.e. non-LOCATABLE) under a closed
    /// attribute is never flagged (closed-world rule 2 — the `archetype_node_id`
    /// discriminator).
    #[test]
    fn closed_world_ignores_metadata_values() {
        let mut root = node("ELEMENT", "/items[at0001]");
        root.closed_attributes = vec![WebTemplateClosedAttribute {
            path: "/items[at0001]/value".to_owned(),
            allowed_ids: vec!["at9999".to_owned()],
            slots: vec![],
        }];
        let inst = json!({"_type": "ELEMENT", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "e"},
            "value": {"_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg"}});
        assert!(
            walk_only(&inst, &root).is_empty(),
            "a DATA_VALUE with no archetype_node_id must not be flagged by closure"
        );
    }

    // ── ARCHETYPE_SLOT enforcement ────────────────────────────────────────────

    fn obs_slot(includes: &[&str], excludes: &[&str], min: i32, max: i32) -> WebTemplateNode {
        let mut root = node("COMPOSITION", "");
        root.closed_attributes = vec![WebTemplateClosedAttribute {
            path: "/content".to_owned(),
            allowed_ids: vec![],
            slots: vec![WebTemplateArchetypeSlot {
                rm_type: "OBSERVATION".to_owned(),
                min,
                max,
                includes: includes.iter().map(|s| (*s).to_owned()).collect(),
                excludes: excludes.iter().map(|s| (*s).to_owned()).collect(),
            }],
        }];
        root
    }

    fn content_obs(archetype_id: &str, rm_type: &str) -> Value {
        json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": [
            {"_type": rm_type, "archetype_node_id": archetype_id,
             "name": {"_type": "DV_TEXT", "value": "o"}}]})
    }

    #[test]
    fn slot_admits_include_rejects_others() {
        let root = obs_slot(&[r"openEHR-EHR-OBSERVATION\..*"], &[], 0, -1);
        assert!(
            walk_only(
                &content_obs("openEHR-EHR-OBSERVATION.bp.v1", "OBSERVATION"),
                &root
            )
            .is_empty(),
            "an include-matching filler is admitted"
        );
        let msgs = walk_only(
            &content_obs("openEHR-EHR-EVALUATION.x.v1", "EVALUATION"),
            &root,
        );
        assert!(
            msgs.iter().any(|m| m.kind == ValidationKind::Unexpected),
            "a wrong-rm-type filler is rejected, got {msgs:?}"
        );
    }

    #[test]
    fn slot_exclude_rejects_matching_filler() {
        let root = obs_slot(
            &[r"openEHR-EHR-OBSERVATION\..*"],
            &[r"openEHR-EHR-OBSERVATION\.secret\..*"],
            0,
            -1,
        );
        let msgs = walk_only(
            &content_obs("openEHR-EHR-OBSERVATION.secret.v1", "OBSERVATION"),
            &root,
        );
        assert!(
            msgs.iter().any(|m| m.kind == ValidationKind::Unexpected),
            "an exclude-matching filler is rejected, got {msgs:?}"
        );
    }

    #[test]
    fn slot_blanket_exclude_ignored_when_includes_present() {
        // ADL 1.4 closed-slot idiom: specific include + a blanket `.*` exclude. The
        // specific include wins (AOM 1.4 has no is_closed) — the filler passes.
        let root = obs_slot(&[r"openEHR-EHR-OBSERVATION\.bp\.v1"], &[".*"], 0, -1);
        assert!(
            walk_only(
                &content_obs("openEHR-EHR-OBSERVATION.bp.v1", "OBSERVATION"),
                &root
            )
            .is_empty(),
            "a specific include overrides a blanket `.*` exclude"
        );
    }

    #[test]
    fn slot_occurrences_min_and_max() {
        let root = obs_slot(&[r"openEHR-EHR-OBSERVATION\..*"], &[], 1, 1);
        let empty = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": []});
        assert!(
            walk_only(&empty, &root)
                .iter()
                .any(|m| m.kind == ValidationKind::Required),
            "an unfilled mandatory slot is Required"
        );
        let two = json!({"_type": "COMPOSITION", "archetype_node_id": "x", "content": [
            {"_type": "OBSERVATION", "archetype_node_id": "openEHR-EHR-OBSERVATION.a.v1",
             "name": {"_type": "DV_TEXT", "value": "a"}},
            {"_type": "OBSERVATION", "archetype_node_id": "openEHR-EHR-OBSERVATION.b.v1",
             "name": {"_type": "DV_TEXT", "value": "b"}}]});
        assert!(
            walk_only(&two, &root)
                .iter()
                .any(|m| m.kind == ValidationKind::Occurrences),
            "too many slot fillers is Occurrences"
        );
    }

    // ── DV_ORDINAL / DV_SCALE (symbol, value) pairing ─────────────────────────

    fn ordinal_node(rm: &str, scale: bool) -> WebTemplateNode {
        let mut n = node(rm, "/value");
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, None);
        let mk = |code: &str, v: i32| {
            let mut cv = WebTemplateCodedValue::new(code, None);
            if scale {
                cv.scale = Some(f64::from(v));
            } else {
                cv.ordinal = Some(v);
            }
            cv
        };
        input.list = vec![mk("at0014", 0), mk("at0015", 1)];
        n.inputs = vec![input];
        n
    }

    fn ordinal_value(rm: &str, v: &Value, code: &str) -> Value {
        json!({"_type": rm, "value": v, "symbol": {"_type": "DV_CODED_TEXT",
            "value": "s", "defining_code": {"_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"}, "code_string": code}}})
    }

    #[test]
    fn ordinal_pair_must_match() {
        let n = ordinal_node("DV_ORDINAL", false);
        assert!(walk_only(&ordinal_value("DV_ORDINAL", &json!(0), "at0014"), &n).is_empty());
        assert!(
            kinds(&walk_only(
                &ordinal_value("DV_ORDINAL", &json!(1), "at0014"),
                &n
            ))
            .contains(&ValidationKind::CodedValue),
            "value 1 does not pair with symbol at0014"
        );
        assert!(
            kinds(&walk_only(
                &ordinal_value("DV_ORDINAL", &json!(0), "at0666"),
                &n
            ))
            .contains(&ValidationKind::CodedValue),
            "symbol at0666 is off the list"
        );
    }

    #[test]
    fn scale_pair_must_match() {
        let n = ordinal_node("DV_SCALE", true);
        assert!(walk_only(&ordinal_value("DV_SCALE", &json!(0.0), "at0014"), &n).is_empty());
        assert!(
            kinds(&walk_only(
                &ordinal_value("DV_SCALE", &json!(1.0), "at0014"),
                &n
            ))
            .contains(&ValidationKind::CodedValue),
            "scale 1.0 does not pair with symbol at0014"
        );
    }

    // ── C_STRING fail-closed with fancy-regex fallback ────────────────────────

    #[test]
    fn c_string_backreference_is_enforced() {
        // `(a)\1` uses a backreference the `regex` crate rejects; `fancy-regex`
        // compiles it, so the pattern is enforced instead of silently passing.
        let mut n = node("DV_TEXT", "/text");
        let mut input = WebTemplateInput::new(WebTemplateInputType::Text, None);
        input.validation = Some(WebTemplateValidation {
            pattern: Some(r"(a)\1".to_owned()),
            ..Default::default()
        });
        n.inputs = vec![input];
        assert!(
            kinds(&walk_only(&json!({"_type": "DV_TEXT", "value": "ab"}), &n))
                .contains(&ValidationKind::PatternError),
            "`ab` must fail the backreference pattern rather than silently pass"
        );
        assert!(walk_only(&json!({"_type": "DV_TEXT", "value": "aa"}), &n).is_empty());
    }

    // ── C_TIME / C_DATE_TIME timezone_validity ────────────────────────────────

    #[test]
    fn timezone_validity_mandatory_and_disallowed() {
        let mut n = node("DV_TIME", "/value");
        n.inputs = vec![WebTemplateInput::new(WebTemplateInputType::Time, None)];
        // 1001 = mandatory timezone.
        n.tz_validity = Some(1001);
        assert!(
            kinds(&walk_only(
                &json!({"_type": "DV_TIME", "value": "10:30:00"}),
                &n
            ))
            .contains(&ValidationKind::PatternError),
            "a missing mandatory timezone is rejected"
        );
        assert!(walk_only(&json!({"_type": "DV_TIME", "value": "10:30:00Z"}), &n).is_empty());
        // 1003 = disallowed timezone.
        n.tz_validity = Some(1003);
        assert!(
            kinds(&walk_only(
                &json!({"_type": "DV_TIME", "value": "10:30:00+01:00"}),
                &n
            ))
            .contains(&ValidationKind::PatternError),
            "a present disallowed timezone is rejected"
        );
        assert!(walk_only(&json!({"_type": "DV_TIME", "value": "10:30:00"}), &n).is_empty());
    }

    // ── name-differentiated same-archetype-id siblings ────────────────────────
    //
    // A template may fill the same archetype twice under one container, the fills
    // differentiated by their runtime `name` (RM common
    // `master03-archetyped_package.adoc` §"The `LOCATABLE` class"; AOM 1.4
    // `master04-constraint_model_package.adoc` §`node_id`). Templates fix
    // `name/value` on all-but-one sibling, so the one *unqualified* sibling admits
    // only the instances no name-qualified sibling claims.

    /// Two same-archetype siblings under `items`, one unqualified (name "A", inner
    /// `items` closed to `at0004`) and one name-qualified ('B', inner `items`
    /// closed to `at0013`).
    fn name_diff_parent() -> WebTemplateNode {
        let mut root = node("CLUSTER", "");
        let sib_a = {
            let mut n = node("CLUSTER", "/items[openEHR-EHR-CLUSTER.c.v1]");
            n.name = Some("A".to_owned());
            n.min = Some(0);
            n.max = 1;
            n.closed_attributes = vec![WebTemplateClosedAttribute {
                path: "/items[openEHR-EHR-CLUSTER.c.v1]/items".to_owned(),
                allowed_ids: vec!["at0004".to_owned()],
                slots: vec![],
            }];
            n
        };
        let sib_b = {
            let mut n = node("CLUSTER", "/items[openEHR-EHR-CLUSTER.c.v1,'B']");
            n.name = Some("B".to_owned());
            n.min = Some(0);
            n.max = 1;
            n.closed_attributes = vec![WebTemplateClosedAttribute {
                path: "/items[openEHR-EHR-CLUSTER.c.v1,'B']/items".to_owned(),
                allowed_ids: vec!["at0013".to_owned()],
                slots: vec![],
            }];
            n
        };
        root.children = vec![sib_a, sib_b];
        root
    }

    /// A same-archetype CLUSTER instance with `name` and a single at-coded child.
    fn c_instance(name: &str, child_id: &str) -> Value {
        json!({
            "_type": "CLUSTER", "archetype_node_id": "openEHR-EHR-CLUSTER.c.v1",
            "name": {"_type": "DV_TEXT", "value": name},
            "items": [{"_type": "ELEMENT", "archetype_node_id": child_id,
                       "name": {"_type": "DV_TEXT", "value": "leaf"},
                       "value": {"_type": "DV_TEXT", "value": "v"}}]
        })
    }

    fn unexpected_of(msgs: &[ValidationMessage]) -> Vec<&ValidationMessage> {
        msgs.iter()
            .filter(|m| m.kind == ValidationKind::Unexpected)
            .collect()
    }

    #[test]
    fn name_diff_siblings_route_each_instance_to_its_own_overlay() {
        let root = name_diff_parent();
        // Both instances present, each carrying its own overlay's child.
        let inst = json!({
            "_type": "CLUSTER", "archetype_node_id": "x",
            "name": {"_type": "DV_TEXT", "value": "root"},
            "items": [c_instance("A", "at0004"), c_instance("B", "at0013")]
        });
        let msgs = walk_only(&inst, &root);
        assert!(
            unexpected_of(&msgs).is_empty(),
            "both name-differentiated siblings should validate against their own \
             overlay, got {msgs:?}"
        );
    }

    #[test]
    fn name_qualified_siblings_child_in_unqualified_instance_is_unexpected() {
        let root = name_diff_parent();
        // `at0013` belongs to sibling 'B' only; inside the instance named "A" it
        // must still be Unexpected (true rejections preserved).
        let inst = json!({
            "_type": "CLUSTER", "archetype_node_id": "x",
            "name": {"_type": "DV_TEXT", "value": "root"},
            "items": [c_instance("A", "at0013")]
        });
        let msgs = walk_only(&inst, &root);
        assert!(
            unexpected_of(&msgs)
                .iter()
                .any(|m| m.message.contains("at0013")),
            "at0013 in the unqualified sibling's instance must be Unexpected, got {msgs:?}"
        );
    }

    #[test]
    fn unqualified_sibling_admits_a_runtime_named_residual_instance() {
        let root = name_diff_parent();
        // An instance whose name matches NO name-qualified sibling ("other") routes
        // to the unqualified (residual) sibling — its `name` being unconstrained,
        // master03 §"The `LOCATABLE` class" L35. Its own overlay child (`at0004`)
        // therefore validates clean…
        let ok = json!({
            "_type": "CLUSTER", "archetype_node_id": "x",
            "name": {"_type": "DV_TEXT", "value": "root"},
            "items": [c_instance("other", "at0004")]
        });
        assert!(
            unexpected_of(&walk_only(&ok, &root)).is_empty(),
            "a residual-named instance must validate against the unqualified overlay"
        );
        // …but a child that overlay forbids (`at0013`) is still Unexpected there.
        let bad = json!({
            "_type": "CLUSTER", "archetype_node_id": "x",
            "name": {"_type": "DV_TEXT", "value": "root"},
            "items": [c_instance("other", "at0013")]
        });
        assert!(
            unexpected_of(&walk_only(&bad, &root))
                .iter()
                .any(|m| m.message.contains("at0013")),
            "the residual instance is still closed-world checked against the \
             unqualified overlay"
        );
    }
}
