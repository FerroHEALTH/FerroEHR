// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Dialect-neutral Web-Template tree shaping over a built [`WebTemplateNode`]
//! tree, shared by [`super::builder`] and [`super::builder_v2_4`]
//! (`simplified_formats master04-basic_concepts.adoc` §"Web Template Metadata",
//! §"Level Removal").

use indexmap::IndexMap;

use super::constraint::{
    WebTemplateClosedAttribute, WebTemplateExistence, WebTemplateSlot, WebTemplateStructuralStub,
};
use super::model::{
    WebTemplateCardinality, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
};

/// The Web-Template metadata **format** version string emitted by both front
/// ends (`ITS-REST simplified_formats master04 §"Web Template Metadata"`:
/// `version`, the format version, not the template version).
pub(super) const CURRENT_VERSION: &str = "2.3";

/// RM types whose wrapper node is always hoisted away (children lifted to parent).
const ALWAYS_COMPACTABLE: [&str; 6] = [
    "ITEM_TREE",
    "ITEM_LIST",
    "ITEM_SINGLE",
    "ITEM_TABLE",
    "ITEM_STRUCTURE",
    "HISTORY",
];

/// RM types compacted only when singular and without a matching sibling.
const SINGLE_COMPACTABLE: [&str; 3] = ["POINT_EVENT", "INTERVAL_EVENT", "EVENT"];

/// RM types dropped when they end up empty (master04 §"Level Removal": a
/// structural wrapper carrying no content collapses out). No openEHR spec
/// enumerates this exact set — our own design/extension.
const SKIP_IF_EMPTY: [&str; 12] = [
    "CLUSTER",
    "ELEMENT",
    "ITEM_TREE",
    "ITEM_LIST",
    "ITEM_SINGLE",
    "ITEM_TABLE",
    "ITEM_STRUCTURE",
    "HISTORY",
    "POINT_EVENT",
    "INTERVAL_EVENT",
    "EVENT",
    "ITEM",
];

/// A placeholder root the type demands when compaction would (impossibly) drop
/// the root; the root is never removed, so this only satisfies the type.
fn tree_placeholder() -> WebTemplateNode {
    WebTemplateNode::new("COMPOSITION".to_owned(), String::new())
}

/// The passes both front ends run on a built tree, in order: compaction
/// (master04 §"Level Removal"), the in-context RM children, the node ids, and
/// the validation walk plan. The in-context children are synthesized before id
/// assignment, so their ids go through the sibling uniqueness of [`super::id`].
pub(super) fn finish(tree: WebTemplateNode) -> WebTemplateNode {
    let mut tree = compact(tree, 1).unwrap_or_else(tree_placeholder);
    super::in_context::synthesize_in_context(&mut tree);
    super::id::build_ids(&mut tree);
    crate::flat::validation::plan::prepare_walk(&mut tree);
    tree
}

// ── post-processing ──────────────────────────────────────────────────────────

/// Per-RM-type node fix-ups applied once during the build (before compaction).
pub(super) fn post_process(node: &mut WebTemplateNode) {
    match node.rm_type.as_str() {
        "ELEMENT" => post_process_element(node),
        "COMPOSITION" => post_process_composition(node),
        "OBSERVATION" => post_process_observation(node),
        _ => {}
    }
}

/// The ELEMENT attributes that qualify a missing value (RM `data_structures`
/// §ELEMENT `null_flavour`, `null_reason`), which ITS-REST `simplified_formats`
/// master05 §ELEMENT addresses as `/_null_flavour` and `/_null_reason`.
const NULL_ATTRIBUTES: [&str; 2] = ["null_flavour", "null_reason"];

/// Moves an ELEMENT's `null_flavour` and `null_reason` constraints into
/// [`WebTemplateNode::null_constraints`] (master05 §ELEMENT), then merges the
/// coded-text and text alternatives of one attribute into a coded text with
/// `other` (master04 §"Open Value-Sets and the `|other` Suffix").
fn post_process_element(node: &mut WebTemplateNode) {
    let element_path = node.aql_path.clone();
    let (null_constraints, values): (Vec<_>, Vec<_>) = std::mem::take(&mut node.children)
        .into_iter()
        .partition(|c| null_attribute(&element_path, c).is_some());
    node.children = values;
    for mut constraint in null_constraints {
        if let Some(id) = null_attribute(&element_path, &constraint).map(|a| format!("_{a}")) {
            constraint.id = id;
        }
        node.null_constraints.push(constraint);
    }
    if let [first_child, second_child] = node.children.as_slice()
        && first_child.aql_path == second_child.aql_path
    {
        let first = first_child.first_input_type();
        let second = second_child.first_input_type();
        if first == Some(WebTemplateInputType::CodedText)
            && second == Some(WebTemplateInputType::Text)
        {
            compact_to_coded_with_other(node, 0, 1);
        } else if second == Some(WebTemplateInputType::CodedText)
            && first == Some(WebTemplateInputType::Text)
        {
            compact_to_coded_with_other(node, 1, 0);
        }
    }
    if node.children.len() > 1 {
        for child in &mut node.children {
            child.min = Some(0);
        }
    }
}

/// The null attribute (`null_flavour` or `null_reason`) a child of the ELEMENT
/// at `element_path` constrains, `None` for a `value` constraint.
fn null_attribute<'a>(element_path: &str, child: &'a WebTemplateNode) -> Option<&'a str> {
    let attribute = child
        .aql_path
        .strip_prefix(element_path)?
        .strip_prefix('/')?;
    NULL_ATTRIBUTES.contains(&attribute).then_some(attribute)
}

fn compact_to_coded_with_other(node: &mut WebTemplateNode, coded: usize, text: usize) {
    let Some(coded_child) = node.children.get_mut(coded) else {
        return;
    };
    if let Some(input) = coded_child.inputs.first_mut() {
        input.list_open = Some(true);
    }
    let mut other = WebTemplateInput::new(WebTemplateInputType::Text, Some("other"));
    other.list_open = Some(true);
    coded_child.inputs.push(other);
    let text_child = node.children.remove(text);
    keep_as_alternative(&mut node.children, coded, text, text_child);
}

/// Records a folded sibling on the node that absorbed it, so the validation
/// walk still accepts it. `kept` and `dropped` are indices before removal.
fn keep_as_alternative(
    children: &mut [WebTemplateNode],
    kept: usize,
    dropped: usize,
    folded: WebTemplateNode,
) {
    let kept = if dropped < kept { kept - 1 } else { kept };
    if let Some(node) = children.get_mut(kept) {
        node.alternatives.push(folded);
    }
}

fn post_process_composition(node: &mut WebTemplateNode) {
    let mut category = None;
    let mut language = None;
    for (i, child) in node.children.iter().enumerate() {
        if child.aql_path == "/category" {
            category = Some(i);
        } else if child.aql_path == "/language" {
            language = Some(i);
        }
    }
    if let Some(cat) = category {
        let node_cat = node.children.remove(cat);
        match language {
            Some(lang) if lang > 0 => node.children.insert(lang - 1, node_cat),
            _ => node.children.push(node_cat),
        }
    }
}

fn post_process_observation(node: &mut WebTemplateNode) {
    let data_path = format!("{}/data", node.aql_path);
    let state_prefix = format!("{}/state", node.aql_path);
    let protocol_prefix = format!("{}/protocol", node.aql_path);
    for child in &mut node.children {
        if child.aql_path.starts_with(&state_prefix) || child.aql_path.starts_with(&protocol_prefix)
        {
            child.depends_on = Some(vec![data_path.clone()]);
        }
    }
}

// ── careflow-state merging (master05 §ISM_TRANSITION) ────────────────────────

/// How a front end gives a built node its archetype identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Identity {
    /// From the constraint object: its archetype-node-id path predicate, `nodeId`,
    /// and rubric / `name` constraint.
    Archetyped,
    /// From the RM attribute alone — no path predicate, no `nodeId`, no rubric
    /// name — so several constraint objects can collapse into one node
    /// ([`merge_alternatives`]).
    AttributeOnly,
}

/// The RM attribute whose sibling constraint objects merge into one node.
///
/// `ACTION.ism_transition` is `1..1` (RM `org.openehr.rm.composition.action.adoc`)
/// while an archetype constrains it once per careflow step; master05 §ACTION and
/// §ISM_TRANSITION map one `/ism_transition` node.
pub(super) const MERGED_ATTRIBUTE: &str = "ism_transition";

/// Fold the constraint alternatives of [`MERGED_ATTRIBUTE`] into the single
/// node master05 §ISM_TRANSITION maps.
///
/// Children match by `(aqlPath, rmType)`, a coded leaf takes the union of the
/// options, and a child only some alternatives carry becomes optional.
pub(super) fn merge_alternatives(nodes: Vec<WebTemplateNode>) -> Option<WebTemplateNode> {
    nodes.into_iter().reduce(|mut acc, other| {
        merge_into(&mut acc, other);
        acc
    })
}

fn merge_into(acc: &mut WebTemplateNode, other: WebTemplateNode) {
    acc.min = match (acc.min, other.min) {
        (Some(a), Some(b)) => Some(a.min(b)),
        // An unstated lower bound constrains nothing, so it wins the relaxation.
        _ => None,
    };
    acc.max = if acc.max == -1 || other.max == -1 {
        -1
    } else {
        acc.max.max(other.max)
    };
    // The closed-local reading survives only if EVERY alternative scopes its
    // code list to `local` (AOM 1.4 `c_coded_text` §C_CODED_TEXT: a code list is
    // scoped to the terminology it names).
    acc.coded_terminology_local &= other.coded_terminology_local;
    merge_inputs(&mut acc.inputs, other.inputs);
    merge_children(acc, other.children);
    merge_existence(&mut acc.existence, other.existence);
    extend_by_key(&mut acc.cardinalities, other.cardinalities, |c| {
        c.path.clone()
    });
    extend_by_key(&mut acc.card_all, other.card_all, |c| c.path.clone());
    extend_by_key(&mut acc.slots, other.slots, |s| {
        (s.path.clone(), s.rm_type.clone())
    });
    extend_by_key(&mut acc.closed_attributes, other.closed_attributes, |c| {
        c.path.clone()
    });
    extend_by_key(&mut acc.code_lists, other.code_lists, |c| {
        (c.attr.clone(), c.terminology.clone())
    });
    extend_by_key(
        &mut acc.constraint_bindings,
        other.constraint_bindings,
        |b| (b.attr.clone(), b.ac_code.clone(), b.terminology.clone()),
    );
    for (terminology, binding) in other.term_bindings {
        acc.term_bindings.entry(terminology).or_insert(binding);
    }
}

/// Merge one alternative's children into `acc`, matching by `(aqlPath, rmType)`
/// and relaxing every child either side does not carry to optional.
fn merge_children(acc: &mut WebTemplateNode, children: Vec<WebTemplateNode>) {
    let mut matched = vec![false; acc.children.len()];
    for child in children {
        let hit = acc
            .children
            .iter()
            .position(|c| c.aql_path == child.aql_path && c.rm_type == child.rm_type);
        if let Some(index) = hit {
            if let Some(seen) = matched.get_mut(index) {
                *seen = true;
            }
            if let Some(existing) = acc.children.get_mut(index) {
                merge_into(existing, child);
            }
        } else {
            let mut child = child;
            child.min = Some(0);
            acc.children.push(child);
        }
    }
    for (existing, seen) in acc.children.iter_mut().zip(&matched) {
        if !*seen {
            existing.min = Some(0);
        }
    }
}

/// Union the alternatives' coded options per input (matched by suffix + type),
/// preserving first-seen order and deduplicating by code.
fn merge_inputs(acc: &mut Vec<WebTemplateInput>, inputs: Vec<WebTemplateInput>) {
    for input in inputs {
        let hit = acc
            .iter()
            .position(|i| i.suffix == input.suffix && i.input_type == input.input_type);
        if let Some(existing) = hit.and_then(|index| acc.get_mut(index)) {
            for option in input.list {
                if !existing.list.iter().any(|held| held.value == option.value) {
                    existing.list.push(option);
                }
            }
        } else {
            acc.push(input);
        }
    }
}

/// Merge the alternatives' existence constraints, keeping one entry per path at
/// the loosest lower bound (an instance conforming to any one alternative must
/// not be rejected).
fn merge_existence(acc: &mut Vec<WebTemplateExistence>, existence: Vec<WebTemplateExistence>) {
    for entry in existence {
        let hit = acc.iter().position(|e| e.path == entry.path);
        if let Some(held) = hit.and_then(|index| acc.get_mut(index)) {
            held.min = held.min.min(entry.min);
        } else {
            acc.push(entry);
        }
    }
}

/// Append the entries of `extra` whose key is not already held by `acc`.
fn extend_by_key<T, K: PartialEq>(acc: &mut Vec<T>, extra: Vec<T>, key: impl Fn(&T) -> K) {
    for item in extra {
        if !acc.iter().any(|held| key(held) == key(&item)) {
            acc.push(item);
        }
    }
}

// ── compaction (master04 §"Level Removal") ───────────────────────────────────

/// Compact `node`'s subtree per master04 §"Level Removal" (`depth` is 1 at the
/// root); `None` when the node itself collapses out.
pub(super) fn compact(mut node: WebTemplateNode, depth: usize) -> Option<WebTemplateNode> {
    // Medium: hoist ALWAYS/SINGLE-compactable wrapper children into this node.
    let children = std::mem::take(&mut node.children);
    let mut hoisted = Hoisted {
        cardinalities: std::mem::take(&mut node.cardinalities),
        existence: std::mem::take(&mut node.existence),
        card_all: std::mem::take(&mut node.card_all),
        slots: std::mem::take(&mut node.slots),
        closed_attributes: std::mem::take(&mut node.closed_attributes),
        structural_stubs: std::mem::take(&mut node.structural_stubs),
    };
    node.children = get_compacted(children, &mut hoisted);
    node.cardinalities = hoisted.cardinalities;
    node.existence = hoisted.existence;
    node.card_all = hoisted.card_all;
    node.slots = hoisted.slots;
    node.closed_attributes = hoisted.closed_attributes;
    node.structural_stubs = hoisted.structural_stubs;
    // Recurse into the (post-hoist) children.
    let children = std::mem::take(&mut node.children);
    node.children = children
        .into_iter()
        .filter_map(|c| compact(c, depth + 1))
        .collect();
    process_children(node, depth)
}

/// The constraint sets a hoisted wrapper re-homes onto its parent (all keyed by
/// absolute archetype paths, so they stay valid after the hoist).
struct Hoisted {
    cardinalities: Vec<WebTemplateCardinality>,
    existence: Vec<WebTemplateExistence>,
    card_all: Vec<WebTemplateCardinality>,
    slots: Vec<WebTemplateSlot>,
    closed_attributes: Vec<WebTemplateClosedAttribute>,
    structural_stubs: Vec<WebTemplateStructuralStub>,
}

fn get_compacted(children: Vec<WebTemplateNode>, parent: &mut Hoisted) -> Vec<WebTemplateNode> {
    let originals: Vec<(String, i32)> = children
        .iter()
        .map(|c| (c.rm_type.clone(), c.max))
        .collect();
    let mut out = Vec::new();
    for mut child in children {
        if is_compactable(&child, &originals) {
            parent
                .cardinalities
                .append(&mut child.cardinalities.clone());
            // A hoisted wrapper's constraints use absolute paths, so they stay valid on
            // the parent.
            parent
                .existence
                .append(&mut std::mem::take(&mut child.existence));
            parent
                .card_all
                .append(&mut std::mem::take(&mut child.card_all));
            parent.slots.append(&mut std::mem::take(&mut child.slots));
            parent
                .closed_attributes
                .append(&mut std::mem::take(&mut child.closed_attributes));
            parent
                .structural_stubs
                .append(&mut std::mem::take(&mut child.structural_stubs));
            // The hoisted wrapper's RM type is kept as a slot (master16
            // §ITEM_STRUCTURE/§EVENT "Class not allowed").
            parent.slots.push(WebTemplateSlot {
                path: child.aql_path.clone(),
                rm_type: child.rm_type.clone(),
                name: child.name.clone(),
            });
            out.extend(get_compacted(child.children, parent));
        } else {
            out.push(child);
        }
    }
    out
}

fn is_compactable(child: &WebTemplateNode, siblings: &[(String, i32)]) -> bool {
    if ALWAYS_COMPACTABLE.contains(&child.rm_type.as_str()) {
        return true;
    }
    child.max == 1
        && SINGLE_COMPACTABLE.contains(&child.rm_type.as_str())
        && !siblings
            .iter()
            .any(|(rm, _)| rm != &child.rm_type && types_match(&child.rm_type, rm))
}

fn types_match(a: &str, b: &str) -> bool {
    a == b || (a.ends_with("EVENT") && b.ends_with("EVENT"))
}

fn process_children(mut node: WebTemplateNode, depth: usize) -> Option<WebTemplateNode> {
    compact_coded_text_with_other(&mut node.children);
    compact_multiple_coded_texts(&mut node.children);

    if !node.has_input() && node.children.len() == 1 && depth > 1 && is_skippable(&node.rm_type) {
        let mut child = node.children.remove(0);
        if node.rm_type == "ELEMENT" && (child.min.is_none() || child.min == Some(0)) {
            child.min = Some(1);
        }
        copy_values(&node, &mut child);
        Some(child)
    } else if node.children.is_empty()
        && node.null_constraints.is_empty()
        && SKIP_IF_EMPTY.contains(&node.rm_type.as_str())
    {
        None
    } else {
        Some(node)
    }
}

fn is_skippable(rm_type: &str) -> bool {
    // A DV_INTERVAL wrapper is never promoted: its bounds are FLAT sub-paths of
    // the interval node (master05 §DV_INTERVAL).
    (rm_type.starts_with("DV_") && !rm_type.starts_with("DV_INTERVAL")) || rm_type == "ELEMENT"
}

/// Merge a same-path `DV_CODED_TEXT` + `DV_TEXT` pair into a coded text with
/// `other` (master04 §"Open Value-Sets and the `|other` Suffix").
fn compact_coded_text_with_other(children: &mut Vec<WebTemplateNode>) {
    let [first, second] = children.as_slice() else {
        return;
    };
    let rms = [first.rm_type.as_str(), second.rm_type.as_str()];
    let is_pair = rms.contains(&"DV_TEXT") && rms.contains(&"DV_CODED_TEXT");
    if is_pair && first.aql_path == second.aql_path {
        let (coded, text) = if first.rm_type == "DV_CODED_TEXT" {
            (0, 1)
        } else {
            (1, 0)
        };
        let Some(coded_child) = children.get_mut(coded) else {
            return;
        };
        if let Some(input) = coded_child.inputs.first_mut() {
            input.list_open = Some(true);
        }
        let mut other = WebTemplateInput::new(WebTemplateInputType::Text, Some("other"));
        other.list_open = Some(true);
        coded_child.inputs.push(other);
        let text_child = children.remove(text);
        keep_as_alternative(children, coded, text, text_child);
    }
}

/// Merge two same-path coded-text siblings into one coded node: a
/// validation-constrained one wins, else the code lists are unioned. The dropped
/// node moves to the kept node's `alternatives`.
///
/// NOTE: no openEHR spec governs presenting a choice of coded texts as one
/// `inputs[].list`: our own design.
#[expect(
    clippy::indexing_slicing,
    reason = "`groups` holds `enumerate()` indices into `children` and nothing is removed before the indexed accesses (the `to_remove` list is applied after), so every index is in bounds by construction"
)]
fn compact_multiple_coded_texts(children: &mut Vec<WebTemplateNode>) {
    // Group child indices by aql path, in first-seen order.
    let mut groups: IndexMap<String, Vec<usize>> = IndexMap::new();
    for (i, child) in children.iter().enumerate() {
        groups.entry(child.aql_path.clone()).or_default().push(i);
    }

    // (keep, drop) pairs: the dropped sibling folds into the kept one.
    let mut to_remove: Vec<(usize, usize)> = Vec::new();
    // (keep, drop) pairs whose coded lists are unioned.
    let mut merges: Vec<(usize, usize)> = Vec::new();
    for idxs in groups.values() {
        // Only an exact pair of coded-text siblings is compacted.
        let [a, b] = idxs[..] else { continue };
        if !is_coded_text(&children[a]) || !is_coded_text(&children[b]) {
            continue;
        }
        match (
            is_input_constrained(&children[a]),
            is_input_constrained(&children[b]),
        ) {
            (true, false) => to_remove.push((a, b)),
            (false, true) => to_remove.push((b, a)),
            _ => merges.push((a, b)),
        }
    }

    for (keep, drop) in merges {
        union_coded_lists(children, keep, drop);
        to_remove.push((keep, drop));
    }

    // The dropped coded constraint stays an alternative of the kept node
    // (ADL 1.4 master05-cadl §"Single-valued Attributes").
    for &(keep, drop) in &to_remove {
        let folded = children[drop].clone();
        children[keep].alternatives.push(folded);
    }
    let mut dropped: Vec<usize> = to_remove.into_iter().map(|(_, drop)| drop).collect();
    dropped.sort_unstable();
    dropped.dedup();
    for i in dropped.into_iter().rev() {
        children.remove(i);
    }
}

/// Unions the dropped sibling's coded list into the kept one, deduplicated by
/// code with first-seen order preserved.
#[expect(
    clippy::indexing_slicing,
    reason = "both indices are `enumerate()` indices into `children` and nothing is removed before this runs, so they are in bounds by construction"
)]
fn union_coded_lists(children: &mut [WebTemplateNode], keep: usize, drop: usize) {
    let drop_list = children[drop]
        .inputs
        .first()
        .map(|i| i.list.clone())
        .unwrap_or_default();
    if let Some(input) = children[keep].inputs.first_mut() {
        for cv in drop_list {
            if !input.list.iter().any(|existing| existing.value == cv.value) {
                input.list.push(cv);
            }
        }
    }
}

/// A coded-text-family node (its `value` is `DV_CODED_TEXT`/`CODE_PHRASE`).
fn is_coded_text(node: &WebTemplateNode) -> bool {
    matches!(node.rm_type.as_str(), "DV_CODED_TEXT" | "CODE_PHRASE")
}

/// Whether the node's first input carries a validation.
fn is_input_constrained(node: &WebTemplateNode) -> bool {
    node.inputs.first().is_some_and(|i| i.validation.is_some())
}

/// Copy the collapsed wrapper's identity/occurrences onto the promoted child
/// (master04 §"Level Removal": the parent connects directly to the wrapper's
/// contents, so the surviving node inherits the collapsed node's identity).
fn copy_values(from: &WebTemplateNode, to: &mut WebTemplateNode) {
    to.name.clone_from(&from.name);
    to.localized_name.clone_from(&from.localized_name);
    to.localized_names.clone_from(&from.localized_names);
    for (k, v) in &from.localized_descriptions {
        to.localized_descriptions
            .entry(k.clone())
            .or_insert_with(|| v.clone());
    }
    to.node_id.clone_from(&from.node_id);
    to.name_code.clone_from(&from.name_code);
    to.name_coded.clone_from(&from.name_coded);
    to.name_rm_type.clone_from(&from.name_rm_type);

    let to_is_dv = to.rm_type.starts_with("DV_");
    if to.min.is_none()
        || from.min.is_some_and(|fm| to.min.is_some_and(|tm| tm > fm))
        || (to_is_dv && to.min == Some(1))
    {
        to.min = from.min;
    }
    if from.max == -1 || (to.max != -1 && to.max < from.max) {
        to.max = from.max;
    }
    for (k, v) in &from.annotations {
        to.annotations.insert(k.clone(), v.clone());
    }
    for (k, v) in &from.term_bindings {
        to.term_bindings.insert(k.clone(), v.clone());
    }
    // Validation-only constraint sets survive the promotion (absolute paths; a
    // path that is not a descendant of the promoted node's own aql path is
    // neutralised by the walk's prefix-strip).
    to.existence.extend(from.existence.iter().cloned());
    to.card_all.extend(from.card_all.iter().cloned());
    to.slots.extend(from.slots.iter().cloned());
    to.closed_attributes
        .extend(from.closed_attributes.iter().cloned());
    // The promoted value presents the collapsed ELEMENT, so the ELEMENT's null
    // constraints move with it (master05 §ELEMENT `/_null_flavour`).
    to.null_constraints
        .extend(from.null_constraints.iter().cloned());
    // A collapsed wrapper's constraint bindings move to the survivor, deduplicated.
    for binding in &from.constraint_bindings {
        if !to.constraint_bindings.contains(binding) {
            to.constraint_bindings.push(binding.clone());
        }
    }
}
