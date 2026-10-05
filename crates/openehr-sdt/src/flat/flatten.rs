// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM (canonical JSON) → simplified tree — the template-driven flattener.
//!
//! Walks the [`WebTemplate`] tree in parallel with the canonical-JSON
//! composition. Level removal is inherent to the walk: a web-template
//! child's RM value(s) are located by the relative RM path between the
//! parent's and child's `aqlPath`, so the container attributes and
//! collapsed wrappers of ITS-REST `simplified_formats/master04` §Level
//! Removal never appear as segments. Each populated leaf emits its datum
//! parts per the `master05-rm_mapping.adoc` tables (`crate::flat::map`); the
//! composition context emits as `ctx/…` (`crate::flat::ctx`).
//!
//! Output scope: `in-context` metadata whose FLAT surface is the `ctx/`
//! vocabulary (composition language/territory/composer, the EVENT_CONTEXT
//! fields, per-entry language/encoding/subject when they carry the
//! defaults) is emitted once as `ctx/…`, not duplicated as path keys —
//! matching the `master04 §Flat format` example. `category` has no `ctx/`
//! key (`master06` defines none), so it emits as a path key. A per-entry
//! `subject` that is not the PARTY_SELF default emits as a path key
//! (`master05 §OBSERVATION` `/subject` row).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::borrow::Cow;

use serde_json::Value;

use crate::flat::ctx;
use crate::flat::error::FlatError;
use crate::flat::map;
use crate::flat::rmpath;
use crate::flat::sim::{SimDocument, SimNode};
use crate::flat::webtemplate::model::{WebTemplate, WebTemplateNode};

/// Flatten a canonical-JSON composition into the simplified tree, driven by
/// `wt`. The result serializes to either wire variant via [`crate::flat::sim`].
///
/// # Errors
/// [`FlatError::Conversion`] if `composition` is not a JSON object.
pub fn flatten_composition(
    composition: &Value,
    wt: &WebTemplate,
) -> Result<SimDocument, FlatError> {
    if !composition.is_object() {
        return Err(FlatError::Conversion(
            "composition must be a JSON object".to_owned(),
        ));
    }
    let mut doc = SimNode::default();
    ctx::emit(composition, doc.occurrence_mut("ctx", None));
    let root = doc.occurrence_mut(&wt.tree.id, None);
    walk(&wt.tree, composition, root);
    doc.prune_empty();
    Ok(doc)
}

/// Whether an RM value's `_type` matches a web-template node's (possibly
/// generic) rm type (base-name comparison, e.g. `DV_INTERVAL<DV_QUANTITY>`).
fn type_matches(rm: &Value, rm_type: &str) -> bool {
    let want = rm_type.split('<').next().unwrap_or(rm_type);
    match rm.get("_type").and_then(Value::as_str) {
        Some(actual) => actual == want,
        None => true,
    }
}

/// A composition-level in-context child whose FLAT surface is the `ctx/`
/// vocabulary (`master06-context_information.adoc`) rather than a path key.
/// `category` is deliberately absent — `master06` defines no `ctx/category`,
/// so it stays a path key (`master05 §COMPOSITION` `/category` row).
fn covered_by_ctx(node: &WebTemplateNode, child: &WebTemplateNode) -> bool {
    // Composition-level in-context shortcuts (master06). The `context`
    // child is ALWAYS walked: its standard EVENT_CONTEXT leaf fields
    // surface as ctx/ scalars (the inner EVENT_CONTEXT rule below keeps
    // them out of the tree), while its archetyped `other_context` content
    // and the lossless `_`-attribute families (`_health_care_facility`,
    // `_participation:i` — master05 §EVENT_CONTEXT) emit as path keys.
    if node.rm_type == "COMPOSITION"
        && matches!(child.id.as_str(), "language" | "territory" | "composer")
    {
        return true;
    }
    // Standard EVENT_CONTEXT fields (start_time, setting, participations, …)
    // surface via ctx/; only archetyped other_context content is tree data.
    if node.rm_type == "EVENT_CONTEXT" {
        return !child.aql_path.contains("other_context");
    }
    false
}

/// A per-entry in-context leaf that emits only when it differs from the
/// value the `ctx/` defaults would rebuild (round-trip stability; the input
/// side accepts the path form regardless, per the `master05` per-entry
/// table rows).
fn is_default_entry_context(child: &WebTemplateNode, rm: &Value) -> bool {
    if child.in_context != Some(true) {
        return false;
    }
    match child.id.as_str() {
        // PARTY_SELF with no external_ref is the ENTRY.subject default
        // (`master05 §OBSERVATION` — "set to PARTY_SELF if not explicitly
        // set").
        "subject" => {
            rm.get("_type").and_then(Value::as_str) == Some("PARTY_SELF")
                && rm.get("external_ref").is_none()
        }
        // ENTRY.language / encoding default from ctx (`master06 §Language
        // and Territory`); their non-default values still round-trip via
        // the ctx emission of the composition language, so per-entry copies
        // are only emitted when they diverge — handled by ctx::emit
        // comparing entries. Here the walk keeps them out of the tree data.
        "language" | "encoding" => true,
        // EVENT time / history origin surface via ctx/history_origin +
        // ctx/time only when they equal the derived defaults; a distinct
        // per-event time is real data and must stay a path key.
        _ => false,
    }
}

fn walk(node: &WebTemplateNode, rm: &Value, out: &mut SimNode) {
    if node.has_input() {
        // A stored value whose `_type` does not conform to the template's
        // declared leaf type cannot be decomposed faithfully into that
        // type's suffixes — embed it verbatim as `|raw` instead
        // (master04 §Raw canonical JSON: pre-existing canonical data /
        // shapes the simplified form cannot express). Lossless both ways.
        if !leaf_type_conforms(rm, &node.rm_type) {
            out.attrs.insert("raw".to_owned(), rm.clone());
            return;
        }
        let list_open = node.inputs.iter().find_map(|i| i.list_open);
        map::emit_leaf(rm, &node.rm_type, list_open, out);
        // Value-level `_` RM attributes of the leaf value itself
        // (`master05` per-type tables: `_normal_range`, `_mapping`, …).
        map::emit_rm_attrs(rm, base_type(&node.rm_type), out);
        return;
    }
    // Container-level `_` RM attribute families (`master05` per-class
    // tables: `_uid`, `_link:i`, `_feeder_audit`, `_work_flow_id`, …).
    map::emit_rm_attrs(rm, base_type(&node.rm_type), out);

    let names = name_fallback(node);
    for child in &node.children {
        if !covered_by_ctx(node, child) {
            walk_child(node, child, rm, out, &names);
        }
    }
    emit_direct_rm_paths(node, rm, out);
}

/// The per-step id-only name-fallback table for `node`'s children
/// ([`rmpath::NameFallback`]).
///
/// Reuses the table the web-template builder cached on the node's walk plan so
/// the projection and the archetype-conformance walk decide a renamed instance
/// node by ONE rule; a hand-built node carrying no plan gets an equivalent
/// table computed here.
fn name_fallback(node: &WebTemplateNode) -> Cow<'_, rmpath::NameFallback> {
    match &node.walk {
        Some(plan) => Cow::Borrowed(&plan.name_fallback),
        None => Cow::Owned(rmpath::name_fallback_from_paths(
            &node.aql_path,
            node.children.iter().map(|c| c.aql_path.as_str()),
        )),
    }
}

/// Emits every occurrence of one template child under its simplified slot.
///
/// Polymorphic choice alternatives share one `aqlPath`, so an occurrence is
/// emitted only under the alternative whose RM type matches the value's
/// `_type`. That filter must apply to EVERY member of a choice group — the
/// first alternative carries no `alt_json_id`, so sharing an `aqlPath` with a
/// sibling is the group marker.
fn walk_child(
    node: &WebTemplateNode,
    child: &WebTemplateNode,
    rm: &Value,
    out: &mut SimNode,
    names: &rmpath::NameFallback,
) {
    let rel = rmpath::relative(&node.aql_path, &child.aql_path);
    let in_choice = child.alt_json_id.is_some()
        || node
            .children
            .iter()
            .any(|c| c.id != child.id && c.aql_path == child.aql_path);
    let occurrences = occurrences_of(child, rm, &rel, in_choice, names);
    if occurrences.is_empty() {
        return;
    }
    if child.max == -1 || child.max > 1 {
        out.children.entry(child.id.clone()).or_default().indexed = true;
    }
    for (i, occurrence) in occurrences.iter().enumerate() {
        if occurrence
            .value
            .is_some_and(|v| is_default_entry_context(child, v))
        {
            continue;
        }
        let slot = out.place_mut(&child.id, u32::try_from(i).unwrap_or(u32::MAX));
        if let Some(value) = occurrence.value {
            walk(child, value, slot);
        }
        // The leaf's wrapping ELEMENT carries its own `_` attribute family
        // (`master05 §ELEMENT`: `_uid`, `_null_flavour`, `_null_reason`,
        // `_link:i`, `_feeder_audit`); the leaf walk above saw only the DV
        // value, so surface the wrapper's here.
        if let Some(element) = occurrence.element {
            map::emit_rm_attrs(element, "ELEMENT", slot);
        }
    }
}

/// One emitted occurrence of a template child: the RM value its relative path
/// reaches, plus the wrapping `ELEMENT` when the child is an ELEMENT-wrapped
/// leaf (the wrapper owns the `master05 §ELEMENT` `_`-attribute rows, which
/// belong on the same simplified node as the datum).
struct Occurrence<'a> {
    /// `None` for a **value-less** ELEMENT. RM data_structures §ELEMENT
    /// (`Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`)
    /// makes `value` and `null_flavour` mutually exclusive, so a
    /// null-flavoured element carries no datum at all — only the
    /// `/_null_flavour` and `/_null_reason` rows of master05 §ELEMENT.
    value: Option<&'a Value>,
    element: Option<&'a Value>,
}

/// The occurrences of template child `child` under RM value `rm`.
///
/// An ELEMENT-wrapped leaf (its relative RM path ends in the `value` step) is
/// resolved through its **wrappers**, not through the values: the wrapper list
/// is the authoritative occurrence list, so (a) a value-less null-flavoured
/// ELEMENT is still reached (master05 §ELEMENT `/_null_flavour`,
/// `/_null_reason`), and (b) wrapper↔value alignment holds even when only some
/// wrappers carry a value. Every other child resolves straight to its values.
fn occurrences_of<'a>(
    child: &WebTemplateNode,
    rm: &'a Value,
    rel: &[openehr_rm::v1_2::paths::PathSegment],
    in_choice: bool,
    names: &rmpath::NameFallback,
) -> Vec<Occurrence<'a>> {
    let Some(wrappers) = value_step_owners(child, rm, rel, names) else {
        return rmpath::resolve_matched(rm, rel, names)
            .into_iter()
            .filter(|value| !in_choice || type_matches(value, &child.rm_type))
            .map(|value| Occurrence {
                value: Some(value),
                element: None,
            })
            .collect();
    };
    wrappers
        .into_iter()
        .filter_map(|wrapper| {
            let element = (wrapper.get("_type").and_then(Value::as_str) == Some("ELEMENT"))
                .then_some(wrapper);
            match wrapper.get("value").filter(|v| !v.is_null()) {
                Some(value) if !in_choice || type_matches(value, &child.rm_type) => {
                    Some(Occurrence {
                        value: Some(value),
                        element,
                    })
                }
                Some(_) => None,
                // NOTE: RM data_structures §ELEMENT `Inv_null_flavour_indicated`
                // makes a value-less ELEMENT with no null flavour RM-invalid.
                //
                // NOTE: no openEHR spec governs a null-flavoured CHOICE — our
                // own design/extension: emitted once, under the first alternative.
                None => (element
                    .is_some_and(|e| e.get("null_flavour").is_some_and(|v| !v.is_null()))
                    && (!in_choice || child.alt_json_id.is_none()))
                .then_some(Occurrence {
                    value: None,
                    element,
                }),
            }
        })
        .collect()
}

/// The RM nodes that own the final `value` step of an ELEMENT-wrapped leaf —
/// `Some` only when `child` is a datum leaf whose relative RM path ends in
/// `value`. `None` selects the plain value-driven resolution (a leaf reached
/// without a wrapper, e.g. `EVENT.time`, or a container child).
fn value_step_owners<'a>(
    child: &WebTemplateNode,
    rm: &'a Value,
    rel: &[openehr_rm::v1_2::paths::PathSegment],
    names: &rmpath::NameFallback,
) -> Option<Vec<&'a Value>> {
    if !child.has_input() {
        return None;
    }
    let (last, parents) = rel.split_last()?;
    (last.attribute == "value").then(|| rmpath::resolve_matched(rm, parents, names))
}

/// Whether the template-child walk already emitted a child that realizes RM
/// attribute `attr` on `node` — i.e. some template child whose relative RM path
/// (master04 §Level Removal) begins with `attr` produced an entry in `out`.
///
/// This is the correct suppression signal for the direct-RM-path fallback: the
/// web template may constrain an attribute under a node-id-specialized child
/// whose id is NOT the attribute name, so an id-string check on the attribute
/// name misses it. Conversely, when the template models the attribute but no
/// such child matched this instance, the walk emits nothing — and the direct
/// path MUST still emit the datum so it is not lost. Hence: suppress iff a
/// realizing child was actually emitted; the WT-child realization then wins
/// entirely (it carries the constrained node identity/name the direct path
/// cannot reconstruct).
fn attr_emitted(node: &WebTemplateNode, out: &SimNode, attr: &str) -> bool {
    node.children.iter().any(|c| {
        out.children.contains_key(&c.id)
            && rmpath::relative(&node.aql_path, &c.aql_path)
                .first()
                .is_some_and(|s| s.attribute == attr)
    })
}

/// Emit the direct RM-attribute paths the master05 per-type mapping tables
/// declare on this node but that the OPT left unconstrained (so the compacted
/// web-template carries no child for them) — the mirror of
/// [`crate::flat::build`]'s direct-path handling, keeping the RM⇄FLAT round-trip
/// lossless. Each attribute is emitted ONLY when the template-child walk did
/// not already realize that RM attribute ([`attr_emitted`]); whenever a
/// WT-child realized it, that realization wins entirely.
///
/// `master05-rm_mapping.adoc` §§ACTION (`/time`, `/ism_transition`),
/// INSTRUCTION (`/narrative`), OBSERVATION (`/history_origin`), ACTIVITY
/// (`/timing`, `/action_archetype_id`), POINT_EVENT/INTERVAL_EVENT (`/time`),
/// INTERVAL_EVENT (`/width`, `/math_function`, `|sample_count`). EVENT_CONTEXT `start_time`/
/// `setting` are NOT emitted here — they surface through the `ctx/` vocabulary
/// (master06; [`crate::flat::ctx`]) to avoid a duplicate encoding of the same datum.
fn emit_direct_rm_paths(node: &WebTemplateNode, rm: &Value, out: &mut SimNode) {
    match base_type(&node.rm_type) {
        "ACTION" => emit_action_paths(node, rm, out),
        // master05 §ISM_TRANSITION: the three coded rows are addressable on the
        // transition node itself, so one the template leaves unconstrained is
        // emitted here rather than lost ([`emit_direct_leaf`] skips whatever the
        // template-child walk already realized).
        "ISM_TRANSITION" => {
            for attr in ["current_state", "transition", "careflow_step"] {
                emit_direct_leaf(node, out, attr, "DV_CODED_TEXT", rm.get(attr));
            }
        }
        "INSTRUCTION" => {
            emit_direct_leaf(node, out, "narrative", "DV_TEXT", rm.get("narrative"));
        }
        "OBSERVATION" => emit_history_origin(rm, out),
        "ACTIVITY" => emit_activity_paths(node, rm, out),
        "POINT_EVENT" | "EVENT" => {
            emit_direct_leaf(node, out, "time", "DV_DATE_TIME", rm.get("time"));
        }
        "INTERVAL_EVENT" => emit_interval_event_paths(node, rm, out),
        _ => {}
    }
}

/// Emits one datum under `name` unless the template-child walk already did.
fn emit_direct_leaf(
    node: &WebTemplateNode,
    out: &mut SimNode,
    name: &str,
    rm_type: &str,
    value: Option<&Value>,
) {
    if attr_emitted(node, out, name) {
        return;
    }
    if let Some(v) = value.filter(|v| !v.is_null()) {
        map::emit_leaf(v, rm_type, None, out.occurrence_mut(name, None));
    }
}

/// Emits the ACTION rows the template left unconstrained (master05 §ACTION).
fn emit_action_paths(node: &WebTemplateNode, rm: &Value, out: &mut SimNode) {
    emit_direct_leaf(node, out, "time", "DV_DATE_TIME", rm.get("time"));
    if !attr_emitted(node, out, "ism_transition")
        && let Some(ism) = rm.get("ism_transition").filter(|v| !v.is_null())
    {
        emit_ism_transition(ism, out.occurrence_mut("ism_transition", None));
    }
}

/// Emits `history_origin`, which maps to the nested `data.origin` (master05
/// §OBSERVATION).
///
/// The HISTORY is compacted away, so `origin` is never a template leaf child —
/// it is emitted here unless the walk already produced it.
fn emit_history_origin(rm: &Value, out: &mut SimNode) {
    if !out.children.contains_key("history_origin")
        && let Some(origin) = rm.pointer("/data/origin/value")
    {
        out.occurrence_mut("history_origin", None)
            .attrs
            .insert(String::new(), origin.clone());
    }
}

/// Emits the ACTIVITY rows the template left unconstrained (master05 §ACTIVITY).
fn emit_activity_paths(node: &WebTemplateNode, rm: &Value, out: &mut SimNode) {
    emit_direct_leaf(node, out, "timing", "DV_PARSABLE", rm.get("timing"));
    // `action_archetype_id` is the match-all `/.*/` when unset (master05
    // §ACTIVITY: "Will be set to /.*/ if not set explicit."); that default is
    // re-synthesised on build, so emitting it would desync the round-trip —
    // emit only an explicit non-default value.
    if !attr_emitted(node, out, "action_archetype_id")
        && let Some(aid) = rm.get("action_archetype_id").and_then(Value::as_str)
        && aid != "/.*/"
    {
        out.occurrence_mut("action_archetype_id", None)
            .attrs
            .insert(String::new(), Value::String(aid.to_owned()));
    }
}

/// Emits the INTERVAL_EVENT rows the template left unconstrained (master05
/// §INTERVAL_EVENT).
fn emit_interval_event_paths(node: &WebTemplateNode, rm: &Value, out: &mut SimNode) {
    emit_direct_leaf(node, out, "time", "DV_DATE_TIME", rm.get("time"));
    emit_direct_leaf(node, out, "width", "DV_DURATION", rm.get("width"));
    emit_direct_leaf(
        node,
        out,
        "math_function",
        "DV_CODED_TEXT",
        rm.get("math_function"),
    );
    // `|sample_count` (INTEGER) is a datum suffix on the event node itself, not
    // a sub-path — the section's second example spells it
    // `…/any_event:0|sample_count: 5`.
    if let Some(n) = rm.get("sample_count").filter(|v| !v.is_null()) {
        out.attrs.insert("sample_count".to_owned(), n.clone());
    }
}

/// Emit an ISM_TRANSITION as its master05 sub-paths — `/current_state`,
/// `/transition`, `/careflow_step` (DV_CODED_TEXT) and the `/_reason:i`
/// DV_TEXT list (master05 §ISM_TRANSITION). The mirror of
/// [`crate::flat::build`]'s `build_ism_transition`.
///
/// Reached only for an ACTION whose template constrains no `ism_transition` at
/// all; a template that constrains one carries the merged transition node
/// ([`crate::flat::webtemplate`]) and the walk emits the same paths through it.
///
/// The spelling is GENERIC in both routes: `ISM_TRANSITION` inherits `PATHABLE`,
/// not `LOCATABLE` (RM
/// `UML/classes/org.openehr.rm.composition.ism_transition.adoc` §Inherit), so an
/// instance carries no `archetype_node_id` a careflow-state path predicate could
/// match, and master05 §ISM_TRANSITION's own worked example puts a
/// careflow-stepped transition (`careflow_step|code: at0006`) under
/// `…/ism_transition/…`, never under a careflow-state child id.
fn emit_ism_transition(ism: &Value, out: &mut SimNode) {
    for attr in ["current_state", "transition", "careflow_step"] {
        if let Some(cs) = ism.get(attr).filter(|v| !v.is_null()) {
            map::emit_leaf(cs, "DV_CODED_TEXT", None, out.occurrence_mut(attr, None));
        }
    }
    map::emit_rm_attrs(ism, "ISM_TRANSITION", out);
}

/// The base (generic-stripped) RM type name.
fn base_type(rm_type: &str) -> &str {
    rm_type.split('<').next().unwrap_or(rm_type)
}

/// Whether an RM leaf value's `_type` conforms to the template's declared
/// leaf type, including the specialisations the simplified mappings handle
/// natively: DV_TEXT ⇄ DV_CODED_TEXT (the `|other` open-value-set pair,
/// master04 §Open Value-Sets) and the PARTY_PROXY family (master05
/// §PARTY_PROXY dispatches on the concrete party type). Anything else is a
/// non-conforming stored value and is embedded as `|raw`.
fn leaf_type_conforms(rm: &Value, declared: &str) -> bool {
    let Some(actual) = rm.get("_type").and_then(Value::as_str) else {
        return true;
    };
    let declared = base_type(declared);
    if actual == declared {
        return true;
    }
    matches!(
        (declared, actual),
        ("DV_TEXT", "DV_CODED_TEXT")
            | ("DV_CODED_TEXT", "DV_TEXT")
            | (
                "PARTY_PROXY",
                "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED"
            )
    )
}
