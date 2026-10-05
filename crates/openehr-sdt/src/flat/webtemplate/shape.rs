// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Dialect-neutral Web-Template tree shaping.
//!
//! Everything here operates on the already-built [`WebTemplateNode`] tree, so it
//! is independent of the source dialect: both [`super::builder`] and
//! [`super::builder_v2_4`] produce the same tree and hand it to these passes,
//! and the semantics are written once.
//!
//! The three passes, in the order the front ends run them:
//!
//! 1. [`compact`] — master04 §"Level Removal": elide container attribute names,
//!    collapse the always-collapsed wrapper types (`ITEM_*`/`ITEM_STRUCTURE`/
//!    `HISTORY`) and a conditionally-collapsed single `EVENT`, promote an
//!    `ELEMENT`/`DATA_VALUE` single child, and drop empties.
//! 2. [`synthesize_in_context`] — master04 §"Web Template Metadata": inject the
//!    RM-mandatory in-context children (`context`/`category`/… on COMPOSITION,
//!    `language`/`encoding`/`subject` on ENTRY, `time` on EVENT) an OPT commonly
//!    leaves unconstrained, so the simplified path keys resolve.
//! 3. [`post_process`] — per-RM-type node fix-ups (called during the build, once
//!    per node, before compaction: the ELEMENT coded/`other` merge, the
//!    COMPOSITION `category` reorder, the OBSERVATION `depends_on`).
//!
//! [`merge_alternatives`] is the fourth shared pass, applied by each front end
//! while it builds an attribute's children.
//!
//! `ITS-REST simplified_formats master04-basic_concepts.adoc` is the wire oracle
//! (§"Web Template Metadata", §"Level Removal").

use indexmap::IndexMap;

use super::model::{
    WebTemplateCardinality, WebTemplateClosedAttribute, WebTemplateExistence, WebTemplateInput,
    WebTemplateInputType, WebTemplateNode, WebTemplateSlot,
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
pub(super) fn tree_placeholder() -> WebTemplateNode {
    WebTemplateNode::new("COMPOSITION".to_owned(), String::new())
}

pub(super) fn is_entry_family(rm_type: &str) -> bool {
    matches!(
        rm_type,
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY" | "GENERIC_ENTRY"
    )
}

fn is_event_family(rm_type: &str) -> bool {
    matches!(rm_type, "EVENT" | "POINT_EVENT" | "INTERVAL_EVENT")
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

fn post_process_element(node: &mut WebTemplateNode) {
    if let [first_child, second_child] = node.children.as_slice() {
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
    node.children.remove(text);
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
/// `ACTION.ism_transition` is `1..1` (RM
/// `UML/classes/org.openehr.rm.composition.action.adoc`), so an ACTION instance
/// carries exactly one `ISM_TRANSITION`, while an archetype constrains the
/// attribute once per careflow step. `ITS-REST simplified_formats
/// master05-rm_mapping.adoc` §ACTION maps the whole transition to one
/// `/ism_transition` row, and §ISM_TRANSITION gives that node its
/// `/current_state`, `/transition`, `/careflow_step` children — one node, three
/// coded children, never one node per state.
pub(super) const MERGED_ATTRIBUTE: &str = "ism_transition";

/// Fold the constraint alternatives of [`MERGED_ATTRIBUTE`] into the single
/// node master05 §ISM_TRANSITION maps.
///
/// Each front end builds the alternatives **without** their careflow at-code
/// identity, so all of them already share one `aqlPath` and no path is
/// rewritten here. The fold is pairwise: children match by `(aqlPath, rmType)`,
/// a coded leaf takes the union of the alternatives' options in first-seen
/// order, and a child that only some alternatives carry becomes optional — so
/// an instance conforming to any one alternative conforms to the merged node.
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

// ── in-context synthesis (master04 §"Web Template Metadata", the `inContext` marker) ──

/// Synthesize the in-context RM children the `ITS-REST simplified_formats
/// master04-basic_concepts.adoc` §"Web Template Metadata" example carries for
/// the RM-mandatory (or RM-defaulted) structural attributes an operational
/// template commonly leaves unconstrained, so the simplified path keys that
/// address them (`…/any_event:0/time`, `…/category|code`, per-ENTRY
/// `language|code`, …) resolve to real web-template nodes instead of being
/// rejected as unknown paths:
///
/// * **COMPOSITION**: `context` (EVENT_CONTEXT) with `start_time` + `setting`;
///   `category`; `language`; `territory`; `composer`.
/// * **ENTRY** (`OBSERVATION`/`EVALUATION`/`INSTRUCTION`/`ACTION`/`ADMIN_ENTRY`
///   /`GENERIC_ENTRY`): `language`; `encoding`; `subject`.
/// * **EVENT** family (`POINT_EVENT`/`INTERVAL_EVENT`/`EVENT`): `time`.
///
/// A synthesized child is added only where the OPT did not already produce an
/// equivalent child at the same `aqlPath` (matched by `aqlPath`), so an OPT that
/// constrains e.g. `EVENT.time` or `COMPOSITION.category` yields the real node
/// and is never duplicated.
///
/// NOTE: no normative Web-Template document spec exists — the master04 example
/// is the shape oracle. The example marks the leaf in-context children with
/// `"inContext": true` (and the `context`/`category` nodes with `"nodeId": ""`)
/// but leaves the `context` wrapper itself unmarked; that is reproduced verbatim.
pub(super) fn synthesize_in_context(node: &mut WebTemplateNode) {
    for child in &mut node.children {
        synthesize_in_context(child);
    }
    if node.rm_type == "COMPOSITION" {
        synth_composition(node);
    } else if is_entry_family(&node.rm_type) {
        synth_entry(node);
    } else if is_event_family(&node.rm_type) {
        synth_event(node);
    }
}

/// The COMPOSITION-level in-context children (master04 §"Web Template Metadata"):
/// the `context` wrapper (with `start_time`/`setting`), then `category`,
/// `language`, `territory`, `composer` in the example's order.
fn synth_composition(node: &mut WebTemplateNode) {
    ensure_context(node);
    ensure_child(
        node,
        "/category",
        "DV_CODED_TEXT",
        None,
        category_inputs(),
        true,
    );
    ensure_child(
        node,
        "/language",
        "CODE_PHRASE",
        Some("Language"),
        vec![],
        false,
    );
    ensure_child(
        node,
        "/territory",
        "CODE_PHRASE",
        Some("Territory"),
        vec![],
        false,
    );
    ensure_child(
        node,
        "/composer",
        "PARTY_PROXY",
        Some("Composer"),
        party_inputs(),
        false,
    );
}

/// The COMPOSITION `context` (EVENT_CONTEXT) node with its `start_time` and
/// `setting` in-context leaves. The wrapper is found-or-created (an OPT that
/// constrains `other_context` already yields a `/context` node) and prepended so
/// it leads the children, as in the master04 example; the leaves are then
/// ensured under it.
fn ensure_context(node: &mut WebTemplateNode) {
    let idx = if let Some(i) = node.children.iter().position(|c| c.aql_path == "/context") {
        i
    } else {
        let mut ctx = WebTemplateNode::new("EVENT_CONTEXT".to_owned(), "/context".to_owned());
        // The example gives the context wrapper `nodeId: ""` and no `inContext`
        // marker; its leaf children carry `inContext: true`.
        ctx.node_id = Some(String::new());
        ctx.min = Some(1);
        ctx.max = 1;
        node.children.insert(0, ctx);
        0
    };
    let Some(ctx) = node.children.get_mut(idx) else {
        return;
    };
    ensure_child(
        ctx,
        "/context/start_time",
        "DV_DATE_TIME",
        Some("Start_time"),
        datetime_inputs(),
        false,
    );
    ensure_child(
        ctx,
        "/context/setting",
        "DV_CODED_TEXT",
        Some("Setting"),
        setting_inputs(),
        false,
    );
}

/// The ENTRY-level in-context children (master04 §"Web Template Metadata":
/// `language`, `encoding`, `subject` on every ENTRY node).
fn synth_entry(node: &mut WebTemplateNode) {
    let base = node.aql_path.clone();
    ensure_child(
        node,
        &format!("{base}/language"),
        "CODE_PHRASE",
        Some("Language"),
        vec![],
        false,
    );
    ensure_child(
        node,
        &format!("{base}/encoding"),
        "CODE_PHRASE",
        Some("Encoding"),
        vec![],
        false,
    );
    ensure_child(
        node,
        &format!("{base}/subject"),
        "PARTY_PROXY",
        Some("Subject"),
        party_inputs(),
        false,
    );
}

/// The EVENT-level in-context child (master04 §"Web Template Metadata": every
/// retained EVENT-family node carries a `time`).
// NOTE: master04 §"Web Template Metadata" renders an EVENT with exactly one
// inContext child, so only `time` is synthesized; an INTERVAL_EVENT's `width`,
// `math_function` and `|sample_count` stay addressable as master05 datum paths.
fn synth_event(node: &mut WebTemplateNode) {
    let base = node.aql_path.clone();
    ensure_child(
        node,
        &format!("{base}/time"),
        "DV_DATE_TIME",
        Some("Time"),
        datetime_inputs(),
        false,
    );
}

/// Append an in-context leaf child at `aql_path` (min 1 / max 1, `inContext:
/// true`) unless a child already occupies that `aqlPath`. `empty_node_id`
/// emits `nodeId: ""` (the master04 shape for `category`).
fn ensure_child(
    parent: &mut WebTemplateNode,
    aql_path: &str,
    rm_type: &str,
    name: Option<&str>,
    inputs: Vec<WebTemplateInput>,
    empty_node_id: bool,
) {
    if parent.children.iter().any(|c| c.aql_path == aql_path) {
        return;
    }
    let mut n = WebTemplateNode::new(rm_type.to_owned(), aql_path.to_owned());
    n.min = Some(1);
    n.max = 1;
    n.in_context = Some(true);
    if let Some(name) = name {
        n.name = Some(name.to_owned());
        n.localized_name = Some(name.to_owned());
    }
    if empty_node_id {
        n.node_id = Some(String::new());
    }
    n.inputs = inputs;
    parent.children.push(n);
}

/// The single `DATETIME` input of a synthesized `start_time`/EVENT `time` leaf.
fn datetime_inputs() -> Vec<WebTemplateInput> {
    vec![WebTemplateInput::new(WebTemplateInputType::Datetime, None)]
}

/// The `code`/`value` TEXT inputs of a synthesized `setting` leaf (master04
/// §"Web Template Metadata": the `setting` node's `inputs`).
fn setting_inputs() -> Vec<WebTemplateInput> {
    vec![
        WebTemplateInput::new(WebTemplateInputType::Text, Some("code")),
        WebTemplateInput::new(WebTemplateInputType::Text, Some("value")),
    ]
}

/// The four PARTY_PROXY TEXT inputs of a synthesized `composer`/`subject` leaf
/// (master04 §"Web Template Metadata": `id`, `id_scheme`, `id_namespace`,
/// `name`).
fn party_inputs() -> Vec<WebTemplateInput> {
    ["id", "id_scheme", "id_namespace", "name"]
        .into_iter()
        .map(|s| WebTemplateInput::new(WebTemplateInputType::Text, Some(s)))
        .collect()
}

/// The single coded `code` input of a synthesized COMPOSITION `category` leaf,
/// carrying the openEHR `433`/`event` coded value verbatim from the master04
/// §"Web Template Metadata" example (localized label `en: event`).
fn category_inputs() -> Vec<WebTemplateInput> {
    let mut cv = super::model::WebTemplateCodedValue::new("433", Some("event".to_owned()));
    cv.localized_labels
        .insert("en".to_owned(), "event".to_owned());
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = vec![cv];
    input.terminology = Some("openehr".to_owned());
    vec![input]
}

// ── compaction (master04 §"Level Removal") ───────────────────────────────────

/// Compact `node`'s subtree per master04 §"Level Removal" (`depth` is 1 at the
/// root). Hoists the always-/single-compactable wrapper children into `node`,
/// recurses, and then applies the child-level fix-ups; returns `None` when the
/// node itself collapses out.
pub(super) fn compact(mut node: WebTemplateNode, depth: usize) -> Option<WebTemplateNode> {
    // Medium: hoist ALWAYS/SINGLE-compactable wrapper children into this node.
    let children = std::mem::take(&mut node.children);
    let mut hoisted = Hoisted {
        cardinalities: std::mem::take(&mut node.cardinalities),
        existence: std::mem::take(&mut node.existence),
        card_all: std::mem::take(&mut node.card_all),
        slots: std::mem::take(&mut node.slots),
        closed_attributes: std::mem::take(&mut node.closed_attributes),
    };
    node.children = get_compacted(children, &mut hoisted);
    node.cardinalities = hoisted.cardinalities;
    node.existence = hoisted.existence;
    node.card_all = hoisted.card_all;
    node.slots = hoisted.slots;
    node.closed_attributes = hoisted.closed_attributes;
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
            // A hoisted wrapper's existence/cardinality constraints (on its own
            // attributes, e.g. HISTORY.events) reference absolute archetype
            // paths, so they stay valid when re-homed on the parent — the walk
            // still enforces them.
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
            // The hoisted wrapper's own RM type is an archetype constraint the
            // compacted tree would otherwise lose: record it as a slot so the
            // walk still rejects a sibling subtype in a narrowed
            // ITEM_STRUCTURE/EVENT slot (AOM 1.4 type conformance,
            // master16 §ITEM_STRUCTURE/§EVENT "Class not allowed").
            parent.slots.push(WebTemplateSlot {
                path: child.aql_path.clone(),
                rm_type: child.rm_type.clone(),
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
    } else if node.children.is_empty() && SKIP_IF_EMPTY.contains(&node.rm_type.as_str()) {
        None
    } else {
        Some(node)
    }
}

fn is_skippable(rm_type: &str) -> bool {
    // A DV_INTERVAL wrapper is NEVER promoted away: its bounds are FLAT
    // sub-paths of the interval node (ITS-REST simplified_formats master05
    // §DV_INTERVAL — `/lower`, `/upper` each carrying the bound type's own
    // suffixes), so collapsing the wrapper onto a single constrained bound
    // loses the interval identity and mis-shapes the built RM (an
    // ELEMENT.value must stay the interval, RM data_types §DV_INTERVAL).
    (rm_type.starts_with("DV_") && !rm_type.starts_with("DV_INTERVAL")) || rm_type == "ELEMENT"
}

/// Merge a two-child `DV_CODED_TEXT` + `DV_TEXT` pair with equal paths into a
/// coded-text-with-`other` — the open-value-set discriminator of master04
/// §"Open Value-Sets and the `|other` Suffix" (`listOpen: true` plus an `other`
/// free-text input).
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
        children.remove(text);
    }
}

/// Merge multiple sibling coded-text alternatives that share an aql path into a
/// single coded node: when an ELEMENT `value` is a choice of coded texts, one
/// node carries the union of the alternatives' coded values rather than several
/// sibling nodes with polymorphic (`value`/`value2`) ids. No openEHR spec governs
/// how a choice of coded texts is presented as a single `inputs[].list` — our own
/// design/extension.
///
/// The rule: children are grouped by path; a group of exactly two coded-text
/// nodes is compacted — if one carries a validation-constrained input and the
/// other does not, the constrained one is kept and the other dropped; otherwise
/// the two coded lists are unioned (dedup by code, order preserved) onto the
/// first node and the second is dropped.
///
/// Our coded-text nodes fold `defining_code` into the node's `inputs`, so the
/// qualifying pair is two same-path `DV_CODED_TEXT`/`CODE_PHRASE` siblings and the
/// coded lists are unioned directly.
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

    let mut to_remove: Vec<usize> = Vec::new();
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
            (true, false) => to_remove.push(b),
            (false, true) => to_remove.push(a),
            _ => merges.push((a, b)),
        }
    }

    for (keep, drop) in merges {
        union_coded_lists(children, keep, drop);
        to_remove.push(drop);
    }

    to_remove.sort_unstable();
    to_remove.dedup();
    for i in to_remove.into_iter().rev() {
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
    // A collapsed wrapper may itself have carried the `CONSTRAINT_REF` proxy
    // (an ELEMENT whose `value` IS the ac-code reference), so its constraint
    // bindings move to the survivor rather than being dropped. Deduplicated —
    // both nodes can name the same ac-code binding, and one check per binding
    // is enough.
    for binding in &from.constraint_bindings {
        if !to.constraint_bindings.contains(binding) {
            to.constraint_bindings.push(binding.clone());
        }
    }
}
