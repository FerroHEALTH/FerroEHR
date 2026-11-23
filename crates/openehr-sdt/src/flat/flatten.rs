// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM → simplified tree, the template-driven flattener.
//!
//! Walks the [`WebTemplate`] beside the composition, typed or canonical JSON.
//! A child's RM values are located by the relative RM path between the two
//! `aqlPath`s, so the containers and wrappers of `simplified_formats/master04`
//! §Level Removal never appear as segments. Leaves emit per the
//! `master05-rm_mapping.adoc` tables; in-context metadata with a `ctx/` key emits
//! once as `ctx/…` (master06), and `category`, which has none, as a path key.

#![expect(
    clippy::disallowed_types,
    reason = "the canonical-JSON value a 553|incomplete commit keeps is one of the two instance \
              forms the flattener reads, and a datum of the simplified wire is a JSON scalar \
              (#1694, #6003)"
)]

use std::borrow::Cow;

use openehr_rm::prelude::Composition;
use openehr_rm::v1_2::access::as_rm_object;
use openehr_rm::v1_2::typing::base_name;
use serde_json::Value;

use crate::flat::error::FlatError;
use crate::flat::instance::{Instance, at};
use crate::flat::map;
use crate::flat::rmpath;
use crate::flat::sim::{SimDocument, SimNode};
use crate::flat::typed::view;
use crate::flat::validation::node_ids::Mapped;
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
    Ok(flatten_instance(composition, wt))
}

/// Flatten a typed composition into the simplified tree, driven by `wt`,
/// reading the RM through the reflective accessor with no canonical-JSON
/// value in between (#6003).
#[must_use]
pub fn flatten_typed(composition: &Composition, wt: &WebTemplate) -> SimDocument {
    flatten_instance(view(as_rm_object(composition)), wt)
}

/// The walk both flatteners share.
fn flatten_instance<'a, I: Instance<'a>>(composition: I, wt: &'a WebTemplate) -> SimDocument {
    let mut doc = SimNode::default();
    match &wt.archetype_refs {
        Some(refs) => fill(Mapped::Node(composition, refs), wt, &mut doc),
        None => fill(composition, wt, &mut doc),
    }
    doc.prune_empty();
    doc
}

/// Emit the `ctx/` keys and the template walk of `composition` into `doc`.
fn fill<'a, I: Instance<'a>>(composition: I, wt: &WebTemplate, doc: &mut SimNode) {
    crate::flat::ctx_emit::emit(composition, doc.occurrence_mut("ctx", None));
    let root = doc.occurrence_mut(&wt.tree.id, None);
    walk(&wt.tree, composition, root);
    if let Some(composer) = composition
        .attr("composer")
        .filter(|c| !map::parties::ref_type_survives(*c, "PERSON"))
        .and_then(Instance::to_raw)
    {
        root.occurrence_mut("composer", None).raw = Some(composer);
    }
}

/// Whether an RM value's `_type` matches a web-template node's (possibly
/// generic) rm type (base-name comparison, e.g. `DV_INTERVAL<DV_QUANTITY>`).
fn type_matches<'a, I: Instance<'a>>(rm: I, rm_type: &str) -> bool {
    let want = rm_type.split('<').next().unwrap_or(rm_type);
    match rm.rm_type() {
        Some(actual) => actual == want,
        None => true,
    }
}

/// A composition-level in-context child whose FLAT surface is the `ctx/`
/// vocabulary (`master06-context_information.adoc`); `category` has no
/// `ctx/category`, so it stays a path key.
fn covered_by_ctx(node: &WebTemplateNode, child: &WebTemplateNode) -> bool {
    // `context` is always walked: its EVENT_CONTEXT leaves surface as `ctx/`
    // scalars, its `other_context` and `_`-attribute families as path keys
    // (master05 §EVENT_CONTEXT).
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

/// A per-entry in-context leaf that emits only when it differs from the `ctx/`
/// default.
fn is_default_entry_context<'a, I: Instance<'a>>(child: &WebTemplateNode, rm: I) -> bool {
    if child.in_context != Some(true) {
        return false;
    }
    match child.id.as_str() {
        // PARTY_SELF with no external_ref is the ENTRY.subject default
        // (`master05 §OBSERVATION` — "set to PARTY_SELF if not explicitly
        // set").
        "subject" => rm.rm_type() == Some("PARTY_SELF") && !rm.is_present("external_ref"),
        // ENTRY.language and encoding default from ctx (`master06 §Language and
        // Territory`); `ctx_emit::emit` handles diverging entries.
        "language" | "encoding" => true,
        // EVENT time / history origin surface via ctx/history_origin +
        // ctx/time only when they equal the derived defaults; a distinct
        // per-event time is real data and must stay a path key.
        _ => false,
    }
}

fn walk<'a, I: Instance<'a>>(node: &WebTemplateNode, rm: I, out: &mut SimNode) {
    if node.has_input() {
        // A value not conforming to the declared leaf type is embedded as `|raw`
        // (master04 §Raw canonical JSON).
        if !leaf_type_conforms(rm, &node.rm_type) {
            out.raw = rm.to_raw();
            return;
        }
        let list_open = node.inputs.iter().find_map(|i| i.list_open);
        map::emit::emit_leaf(rm, &node.rm_type, list_open, out);
        // Value-level `_` RM attributes of the leaf value itself
        // (`master05` per-type tables: `_normal_range`, `_mapping`, …).
        map::emit::emit_rm_attrs(rm, base_name(&node.rm_type), out);
        return;
    }
    // Container-level `_` RM attribute families (`master05` per-class
    // tables: `_uid`, `_link:i`, `_feeder_audit`, `_work_flow_id`, …).
    map::emit::emit_rm_attrs(rm, base_name(&node.rm_type), out);

    let names = name_fallback(node);
    for (index, child) in node.children.iter().enumerate() {
        // master05 §ACTIVITY: `/action_archetype_id` is the bare String, which
        // `emit_activity_paths` writes; its C_STRING template child holds no object.
        let direct = node.rm_type == "ACTIVITY" && child.id == "action_archetype_id";
        if !covered_by_ctx(node, child) && !direct {
            walk_child(node, index, child, rm, out, &names);
        }
    }
    emit_direct_rm_paths(node, rm, out);
}

/// The per-step id-only name-fallback table for `node`'s children, shared with
/// the archetype-conformance walk ([`rmpath::NameFallback`]).
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
/// Choice alternatives share one `aqlPath`, so an occurrence emits only under
/// the alternative whose RM type matches the value's `_type`.
fn walk_child<'a, I: Instance<'a>>(
    node: &WebTemplateNode,
    index: usize,
    child: &WebTemplateNode,
    rm: I,
    out: &mut SimNode,
    names: &rmpath::NameFallback,
) {
    let rel = rmpath::child_segments(node, index, child);
    let in_choice = child.alt_json_id.is_some()
        || node
            .children
            .iter()
            .any(|c| c.id != child.id && c.aql_path == child.aql_path);
    // NOTE: no openEHR spec governs which of several same-type alternatives
    // carries a value in FLAT, our own design: the first one, so it is emitted once.
    if in_choice && shadowed_by_earlier_alternative(node, child) {
        return;
    }
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
        // The wrapping ELEMENT's `_` attribute family (`master05 §ELEMENT`).
        if let Some(element) = occurrence.element {
            map::emit::emit_rm_attrs(element, "ELEMENT", slot);
        }
    }
}

/// Whether an earlier choice alternative of `node` shares `child`'s path and
/// RM type, and so already emits every value `child` would.
fn shadowed_by_earlier_alternative(node: &WebTemplateNode, child: &WebTemplateNode) -> bool {
    node.children
        .iter()
        .take_while(|c| !std::ptr::eq(*c, child))
        .any(|c| c.aql_path == child.aql_path && base_name(&c.rm_type) == base_name(&child.rm_type))
}

/// One emitted occurrence of a template child: the RM value plus the wrapping
/// `ELEMENT`, which owns the `master05 §ELEMENT` `_`-attribute rows.
struct Occurrence<I> {
    /// `None` for a value-less ELEMENT (RM data_structures §ELEMENT
    /// `Inv_null_flavour_indicated`).
    value: Option<I>,
    element: Option<I>,
}

/// The occurrences of template child `child` under RM value `rm`.
///
/// An ELEMENT-wrapped leaf resolves through its wrappers, so a value-less
/// null-flavoured ELEMENT is still reached (master05 §ELEMENT).
fn occurrences_of<'a, I: Instance<'a>>(
    child: &WebTemplateNode,
    rm: I,
    rel: &[openehr_rm::v1_2::paths::PathSegment],
    in_choice: bool,
    names: &rmpath::NameFallback,
) -> Vec<Occurrence<I>> {
    let Some(wrappers) = value_step_owners(child, rm, rel, names) else {
        return rmpath::resolve_matched(rm, rel, names)
            .into_iter()
            .filter(|value| !in_choice || type_matches(*value, &child.rm_type))
            .map(|value| Occurrence {
                value: Some(value),
                element: None,
            })
            .collect();
    };
    wrappers
        .into_iter()
        .filter_map(|wrapper| {
            let element = (wrapper.rm_type() == Some("ELEMENT")).then_some(wrapper);
            match wrapper.attr("value") {
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
                None => (element.is_some_and(|e| e.is_present("null_flavour"))
                    && (!in_choice || child.alt_json_id.is_none()))
                .then_some(Occurrence {
                    value: None,
                    element,
                }),
            }
        })
        .collect()
}

/// The RM nodes owning the final `value` step of an ELEMENT-wrapped leaf, or
/// `None` for a plain value-driven resolution.
fn value_step_owners<'a, I: Instance<'a>>(
    child: &WebTemplateNode,
    rm: I,
    rel: &[openehr_rm::v1_2::paths::PathSegment],
    names: &rmpath::NameFallback,
) -> Option<Vec<I>> {
    if !child.has_input() {
        return None;
    }
    let (last, parents) = rel.split_last()?;
    (last.attribute == "value").then(|| rmpath::resolve_matched(rm, parents, names))
}

/// Whether the template-child walk already emitted a child realizing RM
/// attribute `attr` on `node`; that realization then wins over the direct path.
fn attr_emitted(node: &WebTemplateNode, out: &SimNode, attr: &str) -> bool {
    node.children.iter().enumerate().any(|(index, c)| {
        out.children.contains_key(&c.id)
            && rmpath::child_segments(node, index, c)
                .first()
                .is_some_and(|s| s.attribute == attr)
    })
}

/// Emit the direct RM-attribute paths the master05 tables declare on this node
/// but the OPT left unconstrained, the mirror of [`crate::flat::build`]'s
/// direct-path handling, unless [`attr_emitted`].
///
/// EVENT_CONTEXT `start_time` and `setting` surface through `ctx/` (master06;
/// [`crate::flat::ctx`]) instead.
fn emit_direct_rm_paths<'a, I: Instance<'a>>(node: &WebTemplateNode, rm: I, out: &mut SimNode) {
    match base_name(&node.rm_type) {
        "ACTION" => emit_action_paths(node, rm, out),
        // master05 §ISM_TRANSITION: the coded rows live on the transition node.
        "ISM_TRANSITION" => {
            for attr in ["current_state", "transition", "careflow_step"] {
                emit_direct_leaf(node, out, attr, "DV_CODED_TEXT", rm.attr(attr));
            }
        }
        "INSTRUCTION" => {
            emit_direct_leaf(node, out, "narrative", "DV_TEXT", rm.attr("narrative"));
        }
        "OBSERVATION" => emit_history_origin(rm, out),
        "ACTIVITY" => emit_activity_paths(node, rm, out),
        // NOTE: an abstract EVENT node holds whichever concrete event the data is
        // (RM data_structures master06 §EVENT), so an interval keeps its rows.
        "EVENT" if rm.rm_type() == Some("INTERVAL_EVENT") => {
            emit_interval_event_paths(node, rm, out);
        }
        "POINT_EVENT" | "EVENT" => {
            emit_direct_leaf(node, out, "time", "DV_DATE_TIME", rm.attr("time"));
        }
        "INTERVAL_EVENT" => emit_interval_event_paths(node, rm, out),
        "DV_INTERVAL" => emit_interval_flags(rm, out),
        _ => {}
    }
}

/// Emits the four DV_INTERVAL flags of an interval whose bounds are template
/// nodes (master05 §DV_INTERVAL: "only in output if true", "only in output if
/// false"), plus any flag contradicting the build's derivation.
fn emit_interval_flags<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    for side in ["lower", "upper"] {
        let has_bound = rm.is_present(side);
        let unbounded = format!("{side}_unbounded");
        let included = format!("{side}_included");
        if let Some(flag) = rm.attr(&unbounded).and_then(Instance::as_bool)
            && (flag || flag == has_bound)
        {
            out.attrs.insert(unbounded, Value::Bool(flag));
        }
        if let Some(flag) = rm.attr(&included).and_then(Instance::as_bool)
            && (!flag || flag != has_bound)
        {
            out.attrs.insert(included, Value::Bool(flag));
        }
    }
}

/// Emits one datum under `name` unless the template-child walk already did.
fn emit_direct_leaf<'a, I: Instance<'a>>(
    node: &WebTemplateNode,
    out: &mut SimNode,
    name: &str,
    rm_type: &str,
    value: Option<I>,
) {
    if attr_emitted(node, out, name) {
        return;
    }
    if let Some(v) = value {
        map::emit::emit_leaf(v, rm_type, None, out.occurrence_mut(name, None));
    }
}

/// Emits the ACTION rows the template left unconstrained (master05 §ACTION).
fn emit_action_paths<'a, I: Instance<'a>>(node: &WebTemplateNode, rm: I, out: &mut SimNode) {
    emit_direct_leaf(node, out, "time", "DV_DATE_TIME", rm.attr("time"));
    if !attr_emitted(node, out, "ism_transition")
        && let Some(ism) = rm.attr("ism_transition")
    {
        emit_ism_transition(ism, out.occurrence_mut("ism_transition", None));
    }
}

/// Emits `history_origin`, the nested `data.origin` (master05 §OBSERVATION),
/// unless the walk already produced it.
fn emit_history_origin<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    if !out.children.contains_key("history_origin")
        && let Some(origin) = at(rm, &["data", "origin", "value"]).and_then(Instance::to_value)
    {
        out.occurrence_mut("history_origin", None)
            .attrs
            .insert(String::new(), origin);
    }
}

/// Emits the ACTIVITY rows the template left unconstrained (master05 §ACTIVITY).
fn emit_activity_paths<'a, I: Instance<'a>>(node: &WebTemplateNode, rm: I, out: &mut SimNode) {
    emit_direct_leaf(node, out, "timing", "DV_PARSABLE", rm.attr("timing"));
    // The `/.*/` default (master05 §ACTIVITY) is re-synthesised on build, so only
    // an explicit value is emitted.
    if !attr_emitted(node, out, "action_archetype_id")
        && let Some(aid) = rm.attr("action_archetype_id").and_then(Instance::as_str)
        && aid != "/.*/"
    {
        out.occurrence_mut("action_archetype_id", None)
            .attrs
            .insert(String::new(), Value::String(aid.to_owned()));
    }
}

/// Emits the INTERVAL_EVENT rows the template left unconstrained (master05
/// §INTERVAL_EVENT).
fn emit_interval_event_paths<'a, I: Instance<'a>>(
    node: &WebTemplateNode,
    rm: I,
    out: &mut SimNode,
) {
    emit_direct_leaf(node, out, "time", "DV_DATE_TIME", rm.attr("time"));
    emit_direct_leaf(node, out, "width", "DV_DURATION", rm.attr("width"));
    emit_direct_leaf(
        node,
        out,
        "math_function",
        "DV_CODED_TEXT",
        rm.attr("math_function"),
    );
    // `|sample_count` (INTEGER) is a datum suffix on the event node itself, not
    // a sub-path — the section's second example spells it
    // `…/any_event:0|sample_count: 5`.
    if let Some(n) = rm.attr("sample_count").and_then(Instance::to_value) {
        out.attrs.insert("sample_count".to_owned(), n);
    }
}

/// Emit an ISM_TRANSITION as its master05 sub-paths, the mirror of
/// [`crate::flat::build`]'s `build_ism_transition`.
///
/// The spelling is generic: `ISM_TRANSITION` is not `LOCATABLE` (RM
/// `org.openehr.rm.composition.ism_transition.adoc` §Inherit), as master05
/// §ISM_TRANSITION's own example shows.
fn emit_ism_transition<'a, I: Instance<'a>>(ism: I, out: &mut SimNode) {
    for attr in ["current_state", "transition", "careflow_step"] {
        if let Some(cs) = ism.attr(attr) {
            map::emit::emit_leaf(cs, "DV_CODED_TEXT", None, out.occurrence_mut(attr, None));
        }
    }
    map::emit::emit_rm_attrs(ism, "ISM_TRANSITION", out);
}

/// Whether an RM leaf value's `_type` conforms to the declared leaf type,
/// DV_TEXT/DV_CODED_TEXT (master04 §Open Value-Sets) and the PARTY_PROXY family
/// included.
fn leaf_type_conforms<'a, I: Instance<'a>>(rm: I, declared: &str) -> bool {
    let Some(actual) = rm.rm_type() else {
        return true;
    };
    let declared = base_name(declared);
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
