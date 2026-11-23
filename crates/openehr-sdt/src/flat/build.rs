// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Simplified tree → typed RM, the reverse of [`crate::flat::flatten`].
//!
//! The [`WebTemplate`] is walked against the [`SimDocument`]: leaves rebuilt per
//! `master05-rm_mapping.adoc`, omitted levels re-materialised (master04 §Level
//! Removal), the `ctx/` vocabulary applied (master06) and RM-mandatory fields
//! filled, through `openehr_rm::v1_2::access`.
//!
//! Unknown field identifiers are rejected (`master04 §Validation`).

#![expect(
    clippy::disallowed_types,
    reason = "a client datum and a |raw fragment arrive as JSON (ITS-REST Simplified Formats); \
              the canonical-JSON party and root projections render the typed value (#1694, #6003)"
)]

use std::collections::HashSet;
use std::sync::LazyLock;

use openehr_rm::prelude::Composition;
use openehr_rm::v1_2::access;
use openehr_rm::v1_2::access::{RmObject, RmValue};
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::typing::base_name;
use serde_json::Value;

use crate::flat::ctx;
use crate::flat::defaults;
use crate::flat::error::FlatError;
use crate::flat::identity::{
    default_name, name_value, recorded_archetype_id, stamp_archetype_details,
    stamp_locatable_identity,
};

use crate::flat::map;
use crate::flat::rmpath;
use crate::flat::sim::{SimDocument, SimNode};
use crate::flat::typed::{self, Build, Obj, o, s};
use crate::flat::webtemplate::constraint::WebTemplateStructuralStub;
use crate::flat::webtemplate::model::{CodedName, WebTemplate, WebTemplateNode};

/// The RM release stamped into `ARCHETYPED.rm_version` when the caller names
/// none (RM `master03-archetyped_package.adoc` §ARCHETYPED).
pub const RM_VERSION: &str = "1.2.0";

/// The fixed time of the RM-mandatory temporal fields the simplified form
/// never carries (`HISTORY.origin`, an `EVENT.time` with no origin), so a
/// round trip is stable.
pub(crate) const DEFAULT_TIME: &str = "1970-01-01T00:00:00Z";

/// What a caller chooses about a build beyond the document and the template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildOptions<'a> {
    /// The value written into every `archetype_details.rm_version`.
    pub rm_version: &'a str,
}

impl Default for BuildOptions<'_> {
    fn default() -> Self {
        Self {
            rm_version: RM_VERSION,
        }
    }
}

impl<'a> BuildOptions<'a> {
    /// The options with `rm_version` stamped into every `ARCHETYPED` the build
    /// writes.
    #[must_use]
    pub fn with_rm_version(mut self, rm_version: &'a str) -> Self {
        self.rm_version = rm_version;
        self
    }

    /// The options for `wt`: an OPT2's `rm_release` (AOM2
    /// `master03-archetype_package.adoc`), else [`RM_VERSION`].
    #[must_use]
    pub fn for_template(wt: &'a WebTemplate) -> Self {
        Self::default().with_rm_version(wt.tree.rm_release.as_deref().unwrap_or(RM_VERSION))
    }
}

/// Converts a parsed simplified document to a canonical-JSON composition with
/// [`BuildOptions::for_template`].
///
/// # Errors
///
/// As [`build_composition_with`].
pub fn build_composition(
    doc: &SimDocument,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    build_composition_with(doc, wt, now, &BuildOptions::for_template(wt))
}

/// Converts a parsed simplified document to a canonical-JSON composition.
///
/// `now` is the `ctx/time` default (`master04 §Context`); a document naming
/// no `category` takes the template's default.
///
/// # Errors
///
/// [`FlatError::UnknownPath`] for a field identifier matching no
/// web-template node; the `crate::flat::map`/`crate::flat::ctx` errors for datum
/// and context violations; [`FlatError::MissingRequiredSuffix`] naming
/// `<root>/category|code` when the document names no category and the
/// template admits several with no default; [`FlatError::Conversion`] when the
/// document carries nothing buildable.
pub fn build_composition_with(
    doc: &SimDocument,
    wt: &WebTemplate,
    now: &str,
    options: &BuildOptions<'_>,
) -> Result<Composition, FlatError> {
    Ok(access::finish(composition_tree(doc, wt, now, options)?)?)
}

/// The COMPOSITION tree [`build_composition_with`] finishes, mandatory
/// attributes filled and unchecked.
///
/// # Errors
///
/// As [`build_composition_with`], less the mandatory attributes the finish
/// refuses.
pub(crate) fn composition_tree(
    doc: &SimDocument,
    wt: &WebTemplate,
    now: &str,
    options: &BuildOptions<'_>,
) -> Result<Obj, FlatError> {
    let root_id = &wt.tree.id;
    let mut data_root = None;
    let mut ctx_node = None;
    for (name, child) in &doc.children {
        match name.as_str() {
            "ctx" => ctx_node = child.occurrences.first(),
            n if n == root_id => data_root = child.occurrences.first(),
            other => return Err(FlatError::UnknownPath(other.to_owned())),
        }
    }
    if data_root.is_none() && ctx_node.is_none() {
        return Err(FlatError::Conversion(
            "empty simplified document: no data root and no ctx".to_owned(),
        ));
    }

    let empty = SimNode::default();
    let data_root = data_root.unwrap_or(&empty);
    // Root-level path spellings of the context attributes (`master05
    // §COMPOSITION`); an explicit `ctx/…` key wins.
    let merged_ctx = merge_root_ctx_spellings(ctx_node, data_root);
    let explicit_event_context = has_explicit_event_context(merged_ctx.as_ref());
    let mut defaults = ctx::resolve(merged_ctx.as_ref(), now)?;
    apply_root_raw(&mut defaults, data_root, root_id)?;

    // The template's narrowings of the folded wrappers, so a re-materialised
    // node takes its constrained type (master04 §Level Removal).
    let build_ctx = BuildCtx::new(wt, options);

    let mut comp = match build(&wt.tree, data_root, root_id, &build_ctx)? {
        Some(built) => built,
        None => access::construct("COMPOSITION")?,
    };
    finish_identity(
        &mut *comp,
        &wt.tree,
        true,
        &wt.template_id,
        options.rm_version,
    )?;
    ensure_template_id(&mut *comp, &wt.tree, &wt.template_id, options.rm_version)?;
    // Category first: a persistent composition may omit the context (RM ehr
    // master05 §"Persistent Compositions may optionally have an Event context").
    ensure_category(&mut *comp, &wt.tree)?;
    defaults::apply_composition_ctx(&mut *comp, &defaults, explicit_event_context)?;
    defaults::apply_root_links(&mut *comp, &defaults)?;
    complete_tree(&mut *comp, Some(&defaults))?;
    Ok(comp)
}

/// The concrete party classes a party build produces (RM demographic
/// `master02-demographic_package.adoc`).
pub(crate) const PARTY_CLASSES: [&str; 5] = ["PERSON", "ORGANISATION", "GROUP", "AGENT", "ROLE"];

/// Converts a parsed simplified document of a party template to a
/// canonical-JSON party, checked to decode as the root's class.
///
/// `<root>/performer|id`, `|namespace`, `|type` and `|id_scheme`, or
/// `|raw`, build `ROLE.performer`.
///
/// NOTE: ITS-REST `simplified_formats/master04` defines FLAT for compositions only; a party build is our own extension over the same walk.
///
/// # Errors
///
/// [`FlatError::NotAPartyTemplate`] when the template root is not a party
/// class; [`FlatError::ContextOnParty`] for a `ctx/` key;
/// [`FlatError::UnknownPath`] for a field identifier matching no web-template
/// node; the datum errors of [`build_composition_with`];
/// [`FlatError::Conversion`] when the document carries no data root; and
/// [`FlatError::PartyDecode`] when the built object does not decode as the root
/// class.
pub fn build_party(
    doc: &SimDocument,
    wt: &WebTemplate,
    options: &BuildOptions<'_>,
) -> Result<Value, FlatError> {
    use openehr_rm::v1_2::demographic::party::Party;
    let class = concrete_type(&wt.tree.rm_type);
    if !PARTY_CLASSES.contains(&class) {
        return Err(FlatError::NotAPartyTemplate {
            root: wt.tree.rm_type.clone(),
        });
    }
    let tree = build_root_tree(doc, wt, options)?;
    let party = access::finish::<Party>(tree).map_err(|e| FlatError::PartyDecode {
        class: class.to_owned(),
        source: Box::new(e),
    })?;
    Ok(openehr_its::json::to_canonical_value(&party))
}

/// Converts a parsed simplified document to an unchecked instance of the
/// template's root class; a `ctx/` key is refused.
///
/// NOTE: ITS-REST `simplified_formats/master04` defines FLAT for compositions only; a non-COMPOSITION root is our own extension over the same walk.
///
/// # Errors
///
/// [`FlatError::ContextOnParty`] for a `ctx/` key; [`FlatError::UnknownPath`]
/// for a field identifier matching no web-template node; the datum errors of
/// [`build_composition_with`]; and [`FlatError::Conversion`] when the document
/// carries no data root.
pub fn build_root(
    doc: &SimDocument,
    wt: &WebTemplate,
    options: &BuildOptions<'_>,
) -> Result<Value, FlatError> {
    Ok(typed::render(&*build_root_tree(doc, wt, options)?))
}

/// The unfinished tree [`build_root`] renders and [`build_party`] finishes.
///
/// # Errors
///
/// As [`build_root`].
fn build_root_tree(
    doc: &SimDocument,
    wt: &WebTemplate,
    options: &BuildOptions<'_>,
) -> Result<Obj, FlatError> {
    let root_id = &wt.tree.id;
    let mut data_root = None;
    for (name, child) in &doc.children {
        match name.as_str() {
            "ctx" => return Err(FlatError::ContextOnParty),
            n if n == root_id => data_root = child.occurrences.first(),
            other => return Err(FlatError::UnknownPath(other.to_owned())),
        }
    }
    let Some(data_root) = data_root else {
        return Err(FlatError::Conversion(
            "empty simplified document: no data root".to_owned(),
        ));
    };
    let build_ctx = BuildCtx::new(wt, options);
    let mut root = match build(&wt.tree, data_root, root_id, &build_ctx)? {
        Some(built) => built,
        None => access::construct(concrete_type(&wt.tree.rm_type))?,
    };
    finish_identity(
        &mut *root,
        &wt.tree,
        true,
        &wt.template_id,
        options.rm_version,
    )?;
    ensure_template_id(&mut *root, &wt.tree, &wt.template_id, options.rm_version)?;
    complete_tree(&mut *root, None)?;
    Ok(root)
}

/// Decodes `value` as the party class `class`.
///
/// # Errors
///
/// [`FlatError::PartyDecode`] carrying the strict reader's refusal.
pub(crate) fn check_party_decodes(value: &Value, class: &str) -> Result<(), FlatError> {
    use openehr_its::json::from_canonical_value;
    use openehr_rm::v1_2::demographic;
    let decoded = match class {
        "PERSON" => from_canonical_value::<demographic::person::Person>(value).map(drop),
        "ORGANISATION" => {
            from_canonical_value::<demographic::organisation::Organisation>(value).map(drop)
        }
        "GROUP" => from_canonical_value::<demographic::group::Group>(value).map(drop),
        "AGENT" => from_canonical_value::<demographic::agent::Agent>(value).map(drop),
        _ => from_canonical_value::<demographic::role::Role>(value).map(drop),
    };
    decoded.map_err(|source| FlatError::PartyDecode {
        class: class.to_owned(),
        source: Box::new(source),
    })
}

/// Merges the root-level path spellings of the context attributes
/// (`master05 §COMPOSITION`, `§EVENT_CONTEXT`) into the `ctx` tree, filling
/// gaps only; an explicit `ctx/…` key wins.
fn merge_root_ctx_spellings(ctx_node: Option<&SimNode>, data_root: &SimNode) -> Option<SimNode> {
    let mut merged = ctx_node.cloned().unwrap_or_default();
    let mut fill = |key: &str, value: Option<&Value>| {
        if let Some(v) = value
            && !merged.children.contains_key(key)
        {
            merged
                .occurrence_mut(key, None)
                .attrs
                .insert(String::new(), v.clone());
        }
    };
    let attr = |name: &str, suffix: &str| data_root.child(name).and_then(|n| n.attrs.get(suffix));
    fill("language", attr("language", "code"));
    fill("territory", attr("territory", "code"));
    fill("composer_name", attr("composer", "name"));
    fill("composer_id", attr("composer", "id"));
    fill("id_namespace", attr("composer", "id_namespace"));
    fill("id_scheme", attr("composer", "id_scheme"));
    let context = data_root.child("context");
    let context_attr = |name: &str, suffix: &str| {
        context
            .and_then(|c| c.child(name))
            .and_then(|n| n.attrs.get(suffix))
    };
    fill("time", context_attr("start_time", ""));
    fill("end_time", context_attr("_end_time", ""));
    fill("location", context_attr("_location", ""));
    fill(
        "setting",
        context_attr("setting", "code").or_else(|| context_attr("setting", "value")),
    );
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

/// Takes the `language`, `territory` and `composer` sent as `|raw` (master04
/// §Raw canonical JSON) into the context where nothing has set them.
///
/// # Errors
/// [`FlatError::RawWithoutType`] for a `|raw` value without `_type`;
/// [`FlatError::InvalidRaw`] for one of the wrong RM type, or one beside other
/// content.
fn apply_root_raw(
    defaults: &mut ctx::CtxDefaults,
    data_root: &SimNode,
    root_id: &str,
) -> Result<(), FlatError> {
    let raw = |seg: &str, types: &[&str]| {
        data_root
            .child(seg)
            .and_then(|node| map::raw_value(node, &format!("{root_id}/{seg}"), types))
            .transpose()
    };
    if let Some(phrase) = raw("language", &["CODE_PHRASE"])?
        && defaults.language.is_none()
    {
        defaults.language_phrase = Some(phrase);
    }
    if let Some(phrase) = raw("territory", &["CODE_PHRASE"])?
        && defaults.territory.is_none()
    {
        defaults.territory_phrase = Some(phrase);
    }
    if let Some(composer) = raw("composer", map::PARTY_PROXY_CLASSES)?
        && defaults.composer.is_none()
    {
        defaults.composer = Some(composer);
    }
    Ok(())
}

/// Whether the client supplied explicit EVENT_CONTEXT content.
fn has_explicit_event_context(ctx_node: Option<&SimNode>) -> bool {
    let Some(ctx) = ctx_node else { return false };
    ctx.children.keys().any(|key| {
        matches!(
            key.as_str(),
            "time" | "end_time" | "setting" | "location" | "health_care_facility"
        ) || key.starts_with("participation_")
    })
}

// ── the template walk ──────────────────────────────────────────────────────

/// The ELEMENT-level `_` attribute names (`master05 §ELEMENT`).
fn is_element_level(seg: &str) -> bool {
    matches!(
        seg,
        "_uid" | "_null_flavour" | "_null_reason" | "_link" | "_feeder_audit"
    )
}

/// Whether [`map::build_leaf`] already consumed the `_` segment `seg` on a
/// leaf of type `base`: `/_relationship` and `/_identifier:i` on a
/// PARTY_PROXY (master05 §§PARTY_IDENTIFIED, PARTY_RELATED), which
/// `map::parties::build_party` reads.
fn leaf_consumes_segment(base: &str, seg: &str) -> bool {
    matches!(
        seg,
        map::parties::RELATIONSHIP_TABLE_SEGMENT | map::parties::IDENTIFIER_SEGMENT
    ) && matches!(
        base,
        "PARTY_PROXY" | "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED"
    )
}

/// One hoisted wrapper of the template: its parsed absolute archetype path,
/// its constrained RM type and the name its term definition gives.
pub(crate) struct SlotType {
    /// The parsed absolute archetype path of the wrapper.
    path: Vec<PathSegment>,
    /// The constrained RM type.
    rm_type: String,
    /// The wrapper's name from the template, when it has one.
    name: Option<String>,
}

/// What one build carries down the walk: the hoisted wrappers and the
/// `rm_version` to stamp.
struct BuildCtx<'a> {
    /// The template, whose hoisted wrappers a node without a prepared walk
    /// plan looks up by absolute path.
    wt: &'a WebTemplate,
    /// The hoisted wrappers of `wt`, collected on first use.
    slots: std::cell::OnceCell<Vec<SlotType>>,
    /// The fingerprint of those wrappers, computed on first use.
    slots_fingerprint: std::cell::OnceCell<u64>,
    /// The value written into every `archetype_details.rm_version`.
    rm_version: &'a str,
}

impl<'a> BuildCtx<'a> {
    /// The context for building against `wt` with `options`.
    fn new(wt: &'a WebTemplate, options: &BuildOptions<'a>) -> Self {
        Self {
            wt,
            slots: std::cell::OnceCell::new(),
            slots_fingerprint: std::cell::OnceCell::new(),
            rm_version: options.rm_version,
        }
    }

    /// The fingerprint a prepared walk plan's wrappers must carry to be read.
    fn slots_fingerprint(&self) -> u64 {
        *self
            .slots_fingerprint
            .get_or_init(|| crate::flat::validation::plan::slots_fingerprint(&self.wt.tree))
    }

    /// The template's hoisted wrappers.
    fn slots(&self) -> &[SlotType] {
        self.slots.get_or_init(|| {
            let mut slots = Vec::new();
            collect_slot_types(&self.wt.tree, &mut slots);
            slots
        })
    }
}

/// Collects the template's hoisted-wrapper narrowings and names by absolute
/// path.
pub(crate) fn collect_slot_types(node: &WebTemplateNode, out: &mut Vec<SlotType>) {
    for slot in &node.slots {
        out.push(SlotType {
            path: rmpath::parse(&slot.path),
            rm_type: slot.rm_type.clone(),
            name: slot.name.clone(),
        });
    }
    for child in &node.children {
        collect_slot_types(child, out);
    }
}

/// Whether two archetype-path slices name the same path, by attribute and node
/// id.
fn segments_match(a: &[PathSegment], b: &[PathSegment]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.attribute == y.attribute
                && x.predicate.archetype_node_id == y.predicate.archetype_node_id
        })
}

/// The template's record of one structural wrapper on a placement path: its
/// constrained concrete RM type and its name.
#[derive(Debug, Clone, Default)]
pub(crate) struct WrapperFacts {
    /// The constrained RM type, `None` when the template records no narrowing
    /// (the caller then falls back to [`infer_type`]).
    rm_type: Option<String>,
    /// The name the template gives the wrapper, `None` when it gives none.
    name: Option<String>,
}

/// What the template records about the wrapper at each relative segment of
/// `child`, index-aligned.
pub(crate) fn wrapper_types(
    child_aql: &str,
    rel_len: usize,
    slots: &[SlotType],
) -> Vec<WrapperFacts> {
    let child_abs = rmpath::parse(child_aql);
    if child_abs.len() < rel_len {
        return vec![WrapperFacts::default(); rel_len];
    }
    let base = child_abs.len() - rel_len;
    (0..rel_len)
        .map(|i| {
            child_abs
                .get(..=base + i)
                .and_then(|abs| slots.iter().find(|slot| segments_match(&slot.path, abs)))
                .map(|slot| WrapperFacts {
                    rm_type: Some(slot.rm_type.clone()),
                    name: slot.name.clone(),
                })
                .unwrap_or_default()
        })
        .collect()
}

/// Builds the RM value for `node` from the occurrence `sim`.
///
/// `Ok(None)` is a null-flavoured leaf: its ELEMENT carries `null_flavour` and
/// no `value` (RM `ELEMENT` `Inv_null_flavour_indicated`).
fn build(
    node: &WebTemplateNode,
    sim: &SimNode,
    path: &str,
    ctx: &BuildCtx<'_>,
) -> Result<Option<Obj>, FlatError> {
    if node.has_input() {
        return build_leaf(node, sim, path);
    }
    let interval;
    let node = if is_interval_input(node, sim) {
        interval = WebTemplateNode {
            rm_type: "INTERVAL_EVENT".to_owned(),
            ..node.clone()
        };
        &interval
    } else {
        node
    };

    let mut obj = access::construct(concrete_type(&node.rm_type))?;
    // `OBSERVATION/history_origin` (master05 §OBSERVATION) sets `data.origin`
    // once HISTORY exists.
    let mut history_origin: Option<String> = None;

    walk_template_children(node, sim, path, ctx, &mut *obj)?;
    apply_container_attrs(node, sim, path, &mut *obj, &mut history_origin)?;
    // A container takes a datum only as `|raw` (master04 §Raw canonical JSON)
    // or a suffix the master05 tables place on it.
    if let Some(raw) = &sim.raw {
        return typed::raw_object(raw, path).map(Some);
    }
    map::apply_container_datum_attrs(node, sim, path, &mut *obj)?;

    finish_identity(&mut *obj, node, false, "", ctx.rm_version)?;
    // Applied after HISTORY exists, so the datum wins the default.
    if let Some(ts) = &history_origin
        && let Some(hist) = obj.object_mut("data")
    {
        hist.put("origin", o(typed::dv_date_time(ts)?))?;
    }
    apply_interval_flags(&mut *obj, &node.rm_type)?;
    Ok(Some(obj))
}

/// Builds the DATA_VALUE of a leaf node; `Ok(None)` for [`is_null_flavoured`].
/// ELEMENT-level `_` attributes go to the ELEMENT by [`place`].
///
/// # Errors
/// [`FlatError`] from the leaf build or an unusable RM attribute.
fn build_leaf(node: &WebTemplateNode, sim: &SimNode, path: &str) -> Result<Option<Obj>, FlatError> {
    if is_null_flavoured(sim) {
        return Ok(None);
    }
    let base = base_name(&node.rm_type);
    let mut dv = map::build_leaf(sim, base, Some(node), path)?;
    for (seg, child) in &sim.children {
        if seg.starts_with('_') && !is_element_level(seg) && !leaf_consumes_segment(base, seg) {
            apply_rm_attr(&mut *dv, seg, &child.occurrences, base, path)?;
        }
    }
    Ok(Some(dv))
}

/// Builds every template child the walk covers into `obj`.
///
/// # Errors
/// [`FlatError`] from a child's own build or its placement.
fn walk_template_children(
    node: &WebTemplateNode,
    sim: &SimNode,
    path: &str,
    ctx: &BuildCtx<'_>,
    obj: &mut dyn RmObject,
) -> Result<(), FlatError> {
    for (index, child) in node.children.iter().enumerate() {
        if !child_is_walked(node, child) {
            continue;
        }
        let Some(sim_child) = sim.children.get(&child.id).or_else(|| {
            child
                .alt_json_id
                .as_deref()
                .and_then(|a| sim.children.get(a))
        }) else {
            continue;
        };
        let rel = rmpath::child_segments(node, index, child);
        let types: std::borrow::Cow<'_, [WrapperFacts]> = match node
            .walk
            .as_ref()
            .and_then(|plan| plan.wrappers_of(node, index, child, ctx.slots_fingerprint()))
        {
            Some(cached) => std::borrow::Cow::Borrowed(cached),
            None => std::borrow::Cow::Owned(wrapper_types(&child.aql_path, rel.len(), ctx.slots())),
        };
        for (i, occ) in sim_child.occurrences.iter().enumerate() {
            if occ.is_empty() {
                continue; // a preserved index hole
            }
            let child_path = format!("{path}/{}:{i}", child.id);
            let child_val = build(child, occ, &child_path, ctx)?;
            place(obj, &rel, &types, child_val, child, occ, &child_path, ctx)?;
        }
    }
    Ok(())
}

/// Applies the datum segments addressed to a container node itself; one no
/// walked child takes falls back to [`direct_rm_path`] (`ACTION/time`).
///
/// # Errors
/// [`FlatError::UnknownPath`] when an identifier matches no template child and
/// no spec-listed RM path (master04 §Validation), plus the RM-attribute
/// rejections.
fn apply_container_attrs(
    node: &WebTemplateNode,
    sim: &SimNode,
    path: &str,
    obj: &mut dyn RmObject,
    history_origin: &mut Option<String>,
) -> Result<(), FlatError> {
    for (seg, child) in &sim.children {
        if seg.starts_with('_') {
            apply_rm_attr(obj, seg, &child.occurrences, base_name(&node.rm_type), path)?;
            continue;
        }
        let known = node.children.iter().any(|c| {
            (c.id == *seg || c.alt_json_id.as_deref() == Some(seg)) && child_is_walked(node, c)
        });
        let ctx_covered = node.rm_type == "COMPOSITION"
            && matches!(
                seg.as_str(),
                "language" | "territory" | "composer" | "category"
            );
        if known || ctx_covered {
            continue;
        }
        if place_direct_rm_path(
            obj,
            &node.rm_type,
            seg,
            &child.occurrences,
            path,
            history_origin,
        )? {
            continue;
        }
        return Err(FlatError::UnknownPath(format!("{path}/{seg}")));
    }
    Ok(())
}

/// Whether a leaf is a value-less ELEMENT: no datum, a `/_null_flavour`
/// family (master05 §ELEMENT).
fn is_null_flavoured(sim: &SimNode) -> bool {
    sim.attrs.is_empty()
        && sim.raw.is_none()
        && sim
            .children
            .get("_null_flavour")
            .is_some_and(|c| c.occurrences.iter().any(|o| !o.is_empty()))
}

/// Whether the walk builds `child` from its own sub-tree.
///
/// Not walked: EVENT_CONTEXT standard fields and COMPOSITION
/// `language`/`territory`/`composer` (from `ctx/`, master06), ENTRY
/// `language`/`encoding` (defaulted from the context) and ACTIVITY
/// `action_archetype_id`; their path spellings reach [`direct_rm_path`].
fn child_is_walked(node: &WebTemplateNode, child: &WebTemplateNode) -> bool {
    if node.rm_type == "EVENT_CONTEXT" && !child.aql_path.contains("other_context") {
        return false;
    }
    if node.rm_type == "COMPOSITION"
        && matches!(child.id.as_str(), "language" | "territory" | "composer")
    {
        return false;
    }
    if is_entry_family(&node.rm_type) && matches!(child.id.as_str(), "language" | "encoding") {
        return false;
    }
    if node.rm_type == "ACTIVITY" && child.id == "action_archetype_id" {
        return false;
    }
    true
}

/// Sets the four boundary flags of an interval built from bound children,
/// from bound presence; an explicit flag wins (BASE `Interval`
/// `Limits_consistent`).
fn apply_interval_flags(m: &mut dyn RmObject, rm_type: &str) -> Result<(), FlatError> {
    if base_name(rm_type) != "DV_INTERVAL" {
        return Ok(());
    }
    let has_lower = m.is_set("lower");
    let has_upper = m.is_set("upper");
    m.or_put("lower_unbounded", || Ok(RmValue::Boolean(!has_lower)))?;
    m.or_put("upper_unbounded", || Ok(RmValue::Boolean(!has_upper)))?;
    m.or_put("lower_included", || Ok(RmValue::Boolean(has_lower)))?;
    m.or_put("upper_included", || Ok(RmValue::Boolean(has_upper)))
}

/// A direct RM-attribute path the master05 tables make addressable on `host`
/// where the template has no child.
enum DirectPath {
    /// A single `DATA_VALUE` leaf attribute of the named RM type.
    Leaf {
        /// The RM attribute name the value is inserted under.
        attr: &'static str,
        /// The concrete leaf RM type to build.
        rm_type: &'static str,
    },
    /// `ACTION.ism_transition` (master05 §ISM_TRANSITION).
    Ism,
    /// A `DV_CODED_TEXT` bound to an openEHR group, so a bare `|code` resolves
    /// its rubric.
    CodedGroup {
        /// The RM attribute name the value is inserted under.
        attr: &'static str,
        /// The openEHR terminology group id the row's ValueSet names.
        group: &'static str,
    },
    /// `ACTIVITY.action_archetype_id` — a plain-String RM field
    /// (master05 §ACTIVITY).
    ActionArchetypeId,
    /// `OBSERVATION.history_origin` — the alias for `data.origin`
    /// (master05 §OBSERVATION `/history_origin`).
    HistoryOrigin,
    /// `ROLE.performer` — the `PARTY_REF` to the actor playing the role (RM
    /// demographic §ROLE), built from the master05 §OBJECT_REF suffixes.
    Performer,
}

/// The direct RM-attribute path (if any) named by `seg` on a node of base RM
/// type `host`, per the master05 per-type mapping tables.
fn direct_rm_path(host: &str, seg: &str) -> Option<DirectPath> {
    use DirectPath::{ActionArchetypeId, CodedGroup, HistoryOrigin, Ism, Leaf, Performer};
    Some(match (host, seg) {
        // master05 ENTRY tables `/language`.
        (_, "language") if is_entry_family(host) => Leaf {
            attr: "language",
            rm_type: "CODE_PHRASE",
        },
        // `/encoding` has no ENTRY row, but the master05 §COMPOSITION example spells
        // `…/encoding|code`.
        (_, "encoding") if is_entry_family(host) => Leaf {
            attr: "encoding",
            rm_type: "CODE_PHRASE",
        },
        // master05 §§ACTION, POINT_EVENT, INTERVAL_EVENT: `/time` (DV_DATE_TIME)
        // is addressable on the ACTION and on every EVENT concrete type.
        ("ACTION" | "POINT_EVENT" | "INTERVAL_EVENT" | "EVENT", "time") => Leaf {
            attr: "time",
            rm_type: "DV_DATE_TIME",
        },
        // master05 §ACTION `/ism_transition`, when the template constrains none.
        ("ACTION", "ism_transition") => Ism,
        // master05 §ISM_TRANSITION; `current_state` and `transition` name a group
        // (`org.openehr.rm.composition.ism_transition.adoc` §Invariants).
        ("ISM_TRANSITION", "current_state") => CodedGroup {
            attr: "current_state",
            group: "instruction_states",
        },
        ("ISM_TRANSITION", "transition") => CodedGroup {
            attr: "transition",
            group: "instruction_transitions",
        },
        ("ISM_TRANSITION", "careflow_step") => Leaf {
            attr: "careflow_step",
            rm_type: "DV_CODED_TEXT",
        },
        // master05 §INSTRUCTION: `/narrative` (DV_TEXT).
        ("INSTRUCTION", "narrative") => Leaf {
            attr: "narrative",
            rm_type: "DV_TEXT",
        },
        // master05 §OBSERVATION: `/history_origin` → `history.origin`.
        ("OBSERVATION", "history_origin") => HistoryOrigin,
        // master05 §ACTIVITY: `/timing` (DV_PARSABLE), `/action_archetype_id`.
        ("ACTIVITY", "timing") => Leaf {
            attr: "timing",
            rm_type: "DV_PARSABLE",
        },
        ("ACTIVITY", "action_archetype_id") => ActionArchetypeId,
        // master05 §INTERVAL_EVENT: `/width` (DV_DURATION), `/math_function`.
        ("INTERVAL_EVENT", "width") => Leaf {
            attr: "width",
            rm_type: "DV_DURATION",
        },
        ("INTERVAL_EVENT", "math_function") => Leaf {
            attr: "math_function",
            rm_type: "DV_CODED_TEXT",
        },
        // master05 §EVENT_CONTEXT `/start_time` and `/setting`.
        ("EVENT_CONTEXT", "start_time") => Leaf {
            attr: "start_time",
            rm_type: "DV_DATE_TIME",
        },
        ("EVENT_CONTEXT", "setting") => CodedGroup {
            attr: "setting",
            group: "setting",
        },
        // RM demographic §ROLE `performer: PARTY_REF` (1..1).
        // NOTE: Simplified Formats define no party rows; the master05 §OBJECT_REF suffixes reused here are our own extension.
        ("ROLE", "performer") => Performer,
        _ => return None,
    })
}

/// Builds and places a direct RM path onto `obj`, returning whether `seg` is
/// one; a value the walk already produced is never replaced.
fn place_direct_rm_path(
    obj: &mut dyn RmObject,
    host_rm_type: &str,
    seg: &str,
    occurrences: &[SimNode],
    path: &str,
    history_origin: &mut Option<String>,
) -> Result<bool, FlatError> {
    let Some(dp) = direct_rm_path(base_name(host_rm_type), seg) else {
        return Ok(false);
    };
    let occ = occurrences.iter().find(|o| !o.is_empty());
    let sub_path = format!("{path}/{seg}");
    match dp {
        DirectPath::Leaf { attr, rm_type } => {
            if let Some(node) = occ
                && !obj.is_set(attr)
            {
                let value = map::build_leaf(node, rm_type, None, &sub_path)?;
                obj.put(attr, o(value))?;
            }
        }
        DirectPath::CodedGroup { attr, group } => {
            if let Some(node) = occ
                && !obj.is_set(attr)
            {
                let value = coded_group_value(node, group, &sub_path)?;
                obj.put(attr, o(value))?;
            }
        }
        DirectPath::Ism => {
            if let Some(node) = occ
                && !obj.is_set("ism_transition")
            {
                let value = build_ism_transition(node, &sub_path)?;
                obj.put("ism_transition", o(value))?;
            }
        }
        DirectPath::ActionArchetypeId => {
            if let Some(v) = occ.and_then(SimNode::bare)
                && !obj.is_set("action_archetype_id")
            {
                obj.put_datum("action_archetype_id", v)?;
            }
        }
        DirectPath::HistoryOrigin => {
            *history_origin = occ
                .and_then(SimNode::bare)
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        DirectPath::Performer => {
            if let Some(node) = occ
                && !obj.is_set("performer")
            {
                obj.put("performer", o(build_party_ref(node, &sub_path)?))?;
            }
        }
    }
    Ok(true)
}

/// Builds a `PARTY_REF` from `|id`, `|namespace`, `|type` and `|id_scheme`
/// (master05 §OBJECT_REF), or from `|raw`; the first three are mandatory
/// (BASE `master05-identification_package.adoc` §References).
///
/// # Errors
/// [`FlatError::MissingRequiredSuffix`] naming the first absent suffix,
/// [`FlatError::UnknownSuffix`] for any other suffix, and
/// [`FlatError::RawWithoutType`] or [`FlatError::InvalidRaw`] for a `|raw`
/// value that is no lone `PARTY_REF`.
fn build_party_ref(node: &SimNode, path: &str) -> Result<Obj, FlatError> {
    if let Some(raw) = map::raw_value(node, path, &["PARTY_REF"]) {
        return raw;
    }
    if let Some(suffix) = node
        .attrs
        .keys()
        .find(|k| !matches!(k.as_str(), "id" | "namespace" | "type" | "id_scheme"))
    {
        return Err(FlatError::UnknownSuffix {
            rm_type: "PARTY_REF".to_owned(),
            suffix: suffix.clone(),
            path: path.to_owned(),
        });
    }
    let required = |suffix: &str| {
        node.attrs
            .get(suffix)
            .and_then(Value::as_str)
            .ok_or_else(|| FlatError::MissingRequiredSuffix {
                key: format!("{path}|{suffix}"),
            })
    };
    let id = required("id")?;
    let namespace = required("namespace")?;
    let party_type = required("type")?;
    let id_obj = match node.attrs.get("id_scheme").and_then(Value::as_str) {
        Some(scheme) => typed::make("GENERIC_ID", [("value", s(id)), ("scheme", s(scheme))])?,
        None => typed::hier_object_id(id, &format!("{path}|id"))?,
    };
    typed::make(
        "PARTY_REF",
        [
            ("namespace", s(namespace)),
            ("type", s(party_type)),
            ("id", o(id_obj)),
        ],
    )
}

/// The DV_CODED_TEXT of a direct path bound to an openEHR group: a bare
/// `|code` resolves its rubric (master06 §setting), `|value` stands as given.
///
/// # Errors
/// [`FlatError`] from the leaf build.
fn coded_group_value(node: &SimNode, group: &str, sub_path: &str) -> Result<Obj, FlatError> {
    let bound =
        node.attrs.get("code").and_then(Value::as_str).filter(|_| {
            !node.attrs.contains_key("value") && !node.attrs.contains_key("terminology")
        });
    match bound {
        Some(code) => map::coded_from_group(group, code),
        None => map::build_leaf(node, "DV_CODED_TEXT", None, sub_path),
    }
}

/// Builds an `ACTION.ism_transition` from `/current_state`, `/transition`,
/// `/careflow_step` and `/_reason:i` (master05 §ISM_TRANSITION).
fn build_ism_transition(node: &SimNode, path: &str) -> Result<Obj, FlatError> {
    let mut ism = access::construct("ISM_TRANSITION")?;
    for (seg, group) in [
        ("current_state", Some("instruction_states")),
        ("transition", Some("instruction_transitions")),
        ("careflow_step", None),
    ] {
        if let Some(cs) = node.child(seg).filter(|n| !n.is_empty()) {
            ism.put(
                seg,
                o(build_ism_coded(cs, group, &format!("{path}/{seg}"))?),
            )?;
        }
    }
    for (seg, child) in &node.children {
        if seg.starts_with('_') {
            apply_rm_attr(&mut *ism, seg, &child.occurrences, "ISM_TRANSITION", path)?;
        } else if !matches!(
            seg.as_str(),
            "current_state" | "transition" | "careflow_step"
        ) {
            return Err(FlatError::UnknownPath(format!("{path}/{seg}")));
        }
    }
    Ok(ism)
}

/// Builds one ISM_TRANSITION state as a DV_CODED_TEXT, a bare openEHR `|code`
/// resolving its rubric from `group`.
fn build_ism_coded(node: &SimNode, group: Option<&str>, path: &str) -> Result<Obj, FlatError> {
    let has_value = node.attrs.contains_key("value");
    let terminology = node.attrs.get("terminology").and_then(Value::as_str);
    if !has_value
        && let Some(group) = group
        && let Some(code) = node.attrs.get("code").and_then(Value::as_str)
        && matches!(terminology, None | Some("openehr"))
    {
        return map::coded_from_group(group, code);
    }
    map::build_leaf(node, "DV_CODED_TEXT", None, path)
}

/// Apply one `_`-segment RM attribute family onto the RM object under
/// construction.
fn apply_rm_attr(
    obj: &mut dyn RmObject,
    seg: &str,
    occurrences: &[SimNode],
    host_rm_type: &str,
    path: &str,
) -> Result<(), FlatError> {
    if let Some((attr, value)) = map::build_rm_attr(seg, occurrences, host_rm_type, path)? {
        value.write(obj, attr)?;
    }
    Ok(())
}

// ── structural re-materialisation ──────────────────────────────────────────

/// Inserts `child_value` into `parent` at the relative path `rel`,
/// re-materialising the collapsed nodes (`master04 §Level Removal`); `None`
/// places a null-flavoured ELEMENT without `value`.
#[expect(
    clippy::too_many_arguments,
    reason = "the placement's invariant context, gathered into `Placement` below"
)]
fn place(
    parent: &mut dyn RmObject,
    rel: &[PathSegment],
    types: &[WrapperFacts],
    child_value: Option<Obj>,
    child: &WebTemplateNode,
    occ: &SimNode,
    path: &str,
    ctx: &BuildCtx<'_>,
) -> Result<(), FlatError> {
    if rel.is_empty() {
        return Ok(());
    }
    // NOTE: master04 §"Conditionally Collapsed Wrapper Types": a collapsed EVENT's
    // `events` level is a structural wrapper the leaf below it shares.
    let id_idx = rel
        .iter()
        .rposition(|s| is_multiple(&s.attribute))
        .filter(|&i| {
            i + 1 == rel.len() || types.get(i).is_none_or(|facts| facts.rm_type.is_none())
        });
    let placement = Placement {
        rel,
        id_idx,
        child,
        occ,
        path,
        types,
        ctx,
    };
    place_rec(parent, 0, child_value, &placement)
}

/// The fixed context of one placement walk.
struct Placement<'a> {
    rel: &'a [PathSegment],
    /// The index of the child's OWN repeating array level, if it has one.
    id_idx: Option<usize>,
    child: &'a WebTemplateNode,
    occ: &'a SimNode,
    path: &'a str,
    types: &'a [WrapperFacts],
    ctx: &'a BuildCtx<'a>,
}

impl Placement<'_> {
    /// The constrained RM type of the wrapper at relative segment `i`.
    fn type_at(&self, i: usize) -> Option<&str> {
        self.types.get(i).and_then(|facts| facts.rm_type.as_deref())
    }

    /// The template's name for the wrapper at relative segment `i`.
    fn name_at(&self, i: usize) -> Option<&str> {
        self.types.get(i).and_then(|facts| facts.name.as_deref())
    }
}

/// Materialises the wrapper chain of one relative segment.
///
/// # Errors
/// [`FlatError`] from a nested placement or an ELEMENT attribute family.
#[expect(
    clippy::indexing_slicing,
    reason = "`i` is a cursor into `rel` that the recursion only advances while `i + 1 < rel.len()` (the `last` guard), so `rel[i]` and `rel[i + 1..]` are in bounds at every call"
)]
fn place_rec(
    cur: &mut dyn RmObject,
    i: usize,
    child_value: Option<Obj>,
    p: &Placement<'_>,
) -> Result<(), FlatError> {
    let seg = &p.rel[i];
    if Some(i) == p.id_idx {
        return place_own_level(cur, i, seg, child_value, p);
    }
    if is_multiple(&seg.attribute) {
        return place_structural_level(cur, i, seg, child_value, p);
    }
    place_single_attribute(cur, i, seg, child_value, p)
}

/// The child's own array level: the element itself, or a fresh ELEMENT or
/// deeper node carrying the rest of the path.
///
/// # Errors
/// [`FlatError`] from the nested placement or the ELEMENT attribute family.
#[expect(
    clippy::indexing_slicing,
    reason = "`i` indexes `rel` under the same cursor invariant as `place_rec`"
)]
fn place_own_level(
    cur: &mut dyn RmObject,
    i: usize,
    seg: &PathSegment,
    child_value: Option<Obj>,
    p: &Placement<'_>,
) -> Result<(), FlatError> {
    if i + 1 == p.rel.len() {
        let Some(mut el) = child_value else {
            return Ok(());
        };
        set_node_id(&mut *el, seg.predicate.archetype_node_id.as_deref())?;
        return cur.push_obj(&seg.attribute, el);
    }
    let mut el = new_struct(
        cur.rm_type(),
        seg,
        p.rel.get(i + 1),
        p.child.name.as_deref(),
        p.child.name_coded.as_ref(),
        p.type_at(i),
    )?;
    place(
        &mut *el,
        &p.rel[i + 1..],
        p.types.get(i + 1..).unwrap_or_default(),
        child_value,
        p.child,
        p.occ,
        p.path,
        p.ctx,
    )?;
    // The ELEMENT wrapper's own `_` attribute family (master05 §ELEMENT).
    apply_element_attrs(&mut *el, p.occ, p.path)?;
    cur.push_obj(&seg.attribute, el)
}

/// A structural array level: finds or creates the element with this node id.
///
/// # Errors
/// [`FlatError`] from the nested placement.
fn place_structural_level(
    cur: &mut dyn RmObject,
    i: usize,
    seg: &PathSegment,
    child_value: Option<Obj>,
    p: &Placement<'_>,
) -> Result<(), FlatError> {
    let node_id = seg.predicate.archetype_node_id.as_deref();
    // A name conjunct tells same-archetype siblings apart (BASE
    // `master11-paths.adoc` §"Using a Name-based Predicate").
    let name = seg.predicate.name_value.as_deref();
    let existing = cur.items(&seg.attribute).and_then(|mut items| {
        items.position(|e| {
            e.str_of("archetype_node_id") == node_id
                && name.is_none_or(|n| typed::str_at(e, &["name", "value"]) == Some(n))
        })
    });
    let idx = if let Some(pos) = existing {
        pos
    } else {
        let parent = cur.rm_type();
        cur.push_obj(
            &seg.attribute,
            new_struct(
                parent,
                seg,
                p.rel.get(i + 1),
                p.name_at(i),
                None,
                p.type_at(i),
            )?,
        )?;
        cur.items(&seg.attribute)
            .map_or(0, Iterator::count)
            .saturating_sub(1)
    };
    if let Some(m) = cur.items_mut(&seg.attribute).and_then(|mut it| it.nth(idx)) {
        place_rec(m, i + 1, child_value, p)?;
    }
    Ok(())
}

/// A single-valued attribute: the value lands here, or a fresh node carries
/// the rest of the path.
///
/// # Errors
/// [`FlatError`] from the nested placement or the ELEMENT attribute family.
fn place_single_attribute(
    cur: &mut dyn RmObject,
    i: usize,
    seg: &PathSegment,
    child_value: Option<Obj>,
    p: &Placement<'_>,
) -> Result<(), FlatError> {
    if i + 1 == p.rel.len() {
        let wraps_element = seg.attribute == "value";
        if let Some(child_value) = child_value {
            cur.put(&seg.attribute, o(child_value))?;
        }
        if wraps_element {
            // The object receiving `value` is the compacted-away ELEMENT
            // wrapper; apply its `_` attribute family (master05 §ELEMENT).
            apply_element_attrs(cur, p.occ, p.path)?;
        }
        return Ok(());
    }
    if !cur.is_set(&seg.attribute) {
        let parent = cur.rm_type();
        cur.put(
            &seg.attribute,
            o(new_struct(
                parent,
                seg,
                p.rel.get(i + 1),
                p.name_at(i),
                None,
                p.type_at(i),
            )?),
        )?;
    }
    if let Some(m) = cur.object_mut(&seg.attribute) {
        place_rec(m, i + 1, child_value, p)?;
    }
    Ok(())
}

/// Apply the ELEMENT-level `_` attributes of a leaf occurrence onto its
/// re-materialised ELEMENT wrapper (`master05 §ELEMENT`).
fn apply_element_attrs(
    element: &mut dyn RmObject,
    occ: &SimNode,
    path: &str,
) -> Result<(), FlatError> {
    for (seg, child) in &occ.children {
        if is_element_level(seg) {
            apply_rm_attr(element, seg, &child.occurrences, "ELEMENT", path)?;
        }
    }
    Ok(())
}

/// The RM attributes holding a multi-valued structural container reachable
/// from a versioned-object root, from [`openehr_rm::v1_2::model`]; primitive
/// element types (`Array<Octet>`) are excluded.
static MULTIVALUED_ATTRS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    let mut set: HashSet<&'static str> = HashSet::new();
    let mut seen: HashSet<&'static str> = HashSet::new();
    // The versioned-object roots of the EHR and the demographic package.
    let mut queue: Vec<&'static str> = vec![
        "COMPOSITION",
        "EHR_STATUS",
        "FOLDER",
        "PERSON",
        "ORGANISATION",
        "GROUP",
        "AGENT",
        "ROLE",
        "PARTY_RELATIONSHIP",
    ];
    while let Some(cls) = queue.pop() {
        if !seen.insert(cls) {
            continue;
        }
        for attr in openehr_rm::v1_2::model::attributes(cls) {
            // A primitive element is a scalar, never a wrapper.
            if openehr_rm::v1_2::model::is_primitive(attr.declared_type) {
                continue;
            }
            if attr.container != openehr_rm::v1_2::model::Container::None {
                set.insert(attr.name);
            }
            queue.push(attr.declared_type);
            queue.extend(
                openehr_rm::v1_2::model::descendants(attr.declared_type)
                    .iter()
                    .copied(),
            );
        }
    }
    set
});

/// Whether `attr` is a multi-valued structural RM attribute — the model-driven
/// membership test of the FLAT composition builder.
fn is_multiple(attr: &str) -> bool {
    MULTIVALUED_ATTRS.contains(attr)
}

/// Whether the input at an abstract `EVENT` asks for an `INTERVAL_EVENT`: it
/// names `width`, `math_function` or `sample_count` (master05
/// §INTERVAL_EVENT).
fn is_interval_input(node: &WebTemplateNode, sim: &SimNode) -> bool {
    base_name(&node.rm_type) == "EVENT"
        && (sim.children.contains_key("width")
            || sim.children.contains_key("math_function")
            || sim.attrs.contains_key("sample_count"))
}

fn concrete_type(rm_type: &str) -> &str {
    match base_name(rm_type) {
        "EVENT" => "POINT_EVENT",
        other => other,
    }
}

/// The default concrete subtype of an abstract structural type (`EVENT` →
/// `POINT_EVENT`, `ITEM_STRUCTURE` → `ITEM_TREE`).
fn concretize_structural(rm_type: &str) -> &str {
    match rm_type {
        "EVENT" => "POINT_EVENT",
        "ITEM_STRUCTURE" => "ITEM_TREE",
        other => other,
    }
}

/// The concrete ENTRY subtypes, which carry `language`, `encoding` and
/// `subject`.
pub(crate) fn is_entry_family(rm_type: &str) -> bool {
    matches!(
        rm_type,
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY" | "GENERIC_ENTRY"
    )
}

/// Creates a collapsed structural node for `seg`, typed by the template's
/// narrowing or inferred (`master04 §Level Removal`), with its mandatory
/// fields.
fn new_struct<'a>(
    parent: &str,
    seg: &'a PathSegment,
    next: Option<&PathSegment>,
    name: Option<&str>,
    coded_name: Option<&CodedName>,
    constrained: Option<&'a str>,
) -> Result<Obj, FlatError> {
    // An abstract narrowing becomes its default concrete subtype.
    let node_id = seg.predicate.archetype_node_id.as_deref();
    let rm_type: &str = match constrained {
        Some(narrowed) => concretize_structural(narrowed),
        None => conforming_type(
            parent,
            &seg.attribute,
            infer_type(&seg.attribute, next.map(|s| s.attribute.as_str())),
            node_id,
        ),
    };
    let mut o = access::construct(rm_type)?;
    // `name` only on a LOCATABLE; `ISM_TRANSITION` is PATHABLE.
    if is_locatable(rm_type) {
        // A path's name conjunct is the archetype's name for the node and wins
        // (BASE `master11-paths.adoc` §"Using a Name-based Predicate").
        let display = seg
            .predicate
            .name_value
            .as_deref()
            .or(name)
            .map_or_else(|| default_name(rm_type), str::to_owned);
        o.put("name", typed::o(name_value(&display, coded_name)?))?;
        if let Some(nid) = node_id {
            o.put("archetype_node_id", s(nid))?;
        }
    }
    match rm_type {
        // No `events` until an event lands: ITS-JSON `HISTORY.json` gives the
        // array `minItems: 1`.
        "HISTORY" => {
            o.put("origin", typed::o(typed::dv_date_time(DEFAULT_TIME)?))?;
        }
        "POINT_EVENT" | "EVENT" | "INTERVAL_EVENT" => {
            o.put("time", typed::o(typed::dv_date_time(DEFAULT_TIME)?))?;
        }
        // The list attributes are filled from content; an empty list is never
        // written (ITS-REST Resources.md §JSON Format).
        _ => {}
    }
    Ok(o)
}

/// The class of a collapsed step of `parent.attr`: `inferred` when it
/// conforms, else the class the archetype id names.
///
/// NOTE: BASE `master07-referencing.adoc` §Archetype identifier: the rm_entity of an archetype id names its RM class.
fn conforming_type<'a>(
    parent: &str,
    attr: &str,
    inferred: &'a str,
    node_id: Option<&'a str>,
) -> &'a str {
    use openehr_rm::v1_2::access::schema;
    let Some(declared) = schema::attribute(parent, attr).map(|a| a.declared_type) else {
        return inferred;
    };
    if openehr_rm::v1_2::model::is_a(inferred, declared) {
        return inferred;
    }
    node_id
        .filter(|id| openehr_rm::v1_2::paths::is_archetype_root_node_id(id))
        .and_then(|id| id.split('.').next())
        .and_then(|qualified| qualified.rsplit('-').next())
        .filter(|class| openehr_rm::v1_2::model::is_a(class, declared))
        .unwrap_or(inferred)
}

/// Infer the RM type of a collapsed structural step (`master04 §Level
/// Removal`: the elided container attributes name their owner types).
fn infer_type(attr: &str, next: Option<&str>) -> &'static str {
    match (attr, next) {
        ("data", Some("events")) => "HISTORY",
        ("events", _) => "POINT_EVENT",
        // ELEMENT under `items` or `item` (RM data_structures §ITEM_STRUCTURE).
        ("items" | "item", _) => "ELEMENT",
        ("activities", _) => "ACTIVITY",
        _ => "ITEM_TREE",
    }
}

/// Whether `rm_type` inherits `LOCATABLE`, by the generated model
/// (`RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`).
pub(crate) fn is_locatable(rm_type: &str) -> bool {
    openehr_rm::v1_2::model::is_a(rm_type, "LOCATABLE")
}

fn set_node_id(v: &mut dyn RmObject, node_id: Option<&str>) -> Result<(), FlatError> {
    match node_id {
        Some(nid) => v.or_put("archetype_node_id", || Ok(s(nid))),
        None => Ok(()),
    }
}

// ── identity + mandatory-field completion ──────────────────────────────────

fn empty_item_tree() -> Result<RmValue, FlatError> {
    // The `at0001` "Any" placeholder (ADL 1.4 master05-cadl §"Any" Constraints).
    Ok(o(typed::make(
        "ITEM_TREE",
        [
            ("archetype_node_id", s("at0001")),
            ("name", o(typed::dv_text("Tree")?)),
        ],
    )?))
}

/// Fill the mandatory identity/occurrence fields for a built locatable node.
fn finish_identity(
    obj: &mut dyn RmObject,
    node: &WebTemplateNode,
    is_root: bool,
    template_id: &str,
    rm_version: &str,
) -> Result<(), FlatError> {
    let rm_type = concrete_type(&node.rm_type);
    if fill_non_locatable(obj, rm_type)? {
        return Ok(());
    }
    stamp_locatable_identity(obj, node, rm_type)?;
    fill_time_anchor(obj, rm_type)?;
    stamp_archetype_details(obj, node, is_root, template_id, rm_version)?;
    fill_mandatory_entry_attributes(obj, node, rm_type)?;
    crate::flat::stubbed::fill_stubbed_descendants(obj, node)
}

/// Completes a non-LOCATABLE node, returning whether it was one.
fn fill_non_locatable(obj: &mut dyn RmObject, rm_type: &str) -> Result<bool, FlatError> {
    if rm_type.starts_with("DV_") || rm_type == "CODE_PHRASE" {
        return Ok(true); // leaves already complete
    }
    if rm_type == "EVENT_CONTEXT" {
        return Ok(true); // PATHABLE, not LOCATABLE; fields come from ctx/
    }
    // PARTICIPATION is not LOCATABLE; its mandatory `performer` is a `PARTY_SELF`.
    if rm_type == "PARTICIPATION" {
        obj.or_put("performer", || Ok(o(access::construct("PARTY_SELF")?)))?;
        return Ok(true);
    }
    // A PARTY_PROXY's `name` is a String; `Basic_validity` needs a name,
    // identifiers or external_ref.
    if matches!(rm_type, "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED") {
        if rm_type != "PARTY_SELF"
            && !obj.is_set("name")
            && !obj.is_set("identifiers")
            && !obj.is_set("external_ref")
        {
            obj.put("name", s("Example party"))?;
        }
        return Ok(true);
    }
    Ok(false)
}

/// Fills the mandatory time anchor of the two RM classes that carry one.
fn fill_time_anchor(obj: &mut dyn RmObject, rm_type: &str) -> Result<(), FlatError> {
    match rm_type {
        "POINT_EVENT" | "INTERVAL_EVENT" => {
            obj.or_put("time", || Ok(o(typed::dv_date_time(DEFAULT_TIME)?)))
        }
        "HISTORY" => obj.or_put("origin", || Ok(o(typed::dv_date_time(DEFAULT_TIME)?))),
        _ => Ok(()),
    }
}

/// Fills the RM-mandatory ENTRY attributes with no content: the template's
/// structural stub, else the `at0001` "Any" placeholder (ADL 1.4
/// `master05-cadl.adoc`).
fn fill_mandatory_entry_attributes(
    obj: &mut dyn RmObject,
    node: &WebTemplateNode,
    rm_type: &str,
) -> Result<(), FlatError> {
    if matches!(
        rm_type,
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY" | "GENERIC_ENTRY"
    ) {
        obj.or_put("subject", || Ok(o(access::construct("PARTY_SELF")?)))?;
    }
    match rm_type {
        "OBSERVATION" => obj.or_put("data", || {
            Ok(o(structural_from_stub(node, "data", "HISTORY", "History")?))
        }),
        "EVALUATION" | "ADMIN_ENTRY" => obj.or_put("data", || {
            Ok(o(structural_from_stub(node, "data", "ITEM_TREE", "Tree")?))
        }),
        // NOTE: master06 §§instruction_narrative, action_ism_transition_current_state make
        // `INSTRUCTION.narrative` and the ISM state ctx defaults, so complete_tree fills them last.
        "ACTION" => {
            obj.or_put("time", || Ok(o(typed::dv_date_time(DEFAULT_TIME)?)))?;
            obj.or_put("description", || {
                Ok(o(structural_from_stub(
                    node,
                    "description",
                    "ITEM_TREE",
                    "Tree",
                )?))
            })
        }
        _ => Ok(()),
    }
}

/// An empty structural node for a missing mandatory ENTRY attribute, from its
/// stub by absolute path.
fn structural_from_stub(
    node: &WebTemplateNode,
    attr: &str,
    default_rm_type: &str,
    default_name: &str,
) -> Result<Obj, FlatError> {
    let path = format!("{}/{attr}", node.aql_path);
    match node.structural_stubs.iter().find(|s| s.path == path) {
        Some(stub) => structural_from(stub, default_name),
        None => structural_container(default_rm_type, "at0001", default_name),
    }
}

/// Synthesizes the empty structural node a [`WebTemplateStructuralStub`]
/// constrains, named by its rubric or else `default_name`.
pub(crate) fn structural_from(
    stub: &WebTemplateStructuralStub,
    default_name: &str,
) -> Result<Obj, FlatError> {
    let name = stub.name.as_deref().unwrap_or(default_name);
    structural_container(concrete_structural(&stub.rm_type), &stub.node_id, name)
}

/// An abstract `ITEM_STRUCTURE` constraint materialises as `ITEM_TREE`; a
/// concrete type passes through.
fn concrete_structural(rm_type: &str) -> &str {
    match rm_type {
        "ITEM_STRUCTURE" => "ITEM_TREE",
        other => other,
    }
}

fn structural_container(rm_type: &str, node_id: &str, name: &str) -> Result<Obj, FlatError> {
    let mut o = typed::make(
        rm_type,
        [
            ("archetype_node_id", s(node_id)),
            ("name", o(typed::dv_text(name)?)),
        ],
    )?;
    if rm_type == "HISTORY" {
        o.put("origin", typed::o(typed::dv_date_time(DEFAULT_TIME)?))?;
    }
    Ok(o)
}

/// Supplies `COMPOSITION.category` when the input names none.
///
/// # Errors
///
/// [`FlatError::MissingRequiredSuffix`] naming `<root>/category|code` when the
/// template admits several codes and fixes no default.
fn ensure_category(comp: &mut dyn RmObject, root: &WebTemplateNode) -> Result<(), FlatError> {
    if comp.is_set("category") {
        return Ok(());
    }
    let category = template_category(root)?;
    comp.put("category", o(category))
}

/// The default `COMPOSITION.category`: the template's default value, else its
/// single code, else `433` (event) when unconstrained.
///
/// NOTE: no openEHR spec governs the default for an omitted category — our own design/extension.
///
/// # Errors
///
/// The admitted codes when the template admits several and fixes no default.
fn template_category(root: &WebTemplateNode) -> Result<Obj, FlatError> {
    let input = root
        .children
        .iter()
        .find(|child| child.aql_path == "/category")
        .and_then(|node| {
            node.inputs
                .iter()
                .find(|input| input.suffix.as_deref() == Some("code"))
        });
    let Some(input) = input.filter(|input| !input.list.is_empty()) else {
        return typed::dv_coded_text("event", "openehr", "433");
    };
    let terminology = input.terminology.as_deref().unwrap_or("openehr");
    let assumed = input.default_value.as_ref().and_then(Value::as_str);
    let chosen = match (assumed, input.list.as_slice()) {
        (Some(code), list) => list.iter().find(|entry| entry.value == code),
        (None, [only]) => Some(only),
        (None, _) => None,
    };
    match chosen {
        Some(entry) => typed::dv_coded_text(
            entry.label.as_deref().unwrap_or(&entry.value),
            terminology,
            &entry.value,
        ),
        None => Err(FlatError::MissingRequiredSuffix {
            key: format!("{}/category|code", root.id),
        }),
    }
}

/// Ensures `archetype_details` carries `archetype_id` and `template_id`
/// (ITS-REST `Requests_and_responses.md §openehr-template-id`).
fn ensure_template_id(
    comp: &mut dyn RmObject,
    root: &WebTemplateNode,
    template_id: &str,
    rm_version: &str,
) -> Result<(), FlatError> {
    if !comp.is_set("archetype_details") {
        let mut ad = typed::make("ARCHETYPED", [("rm_version", s(rm_version))])?;
        if let Some(id) = recorded_archetype_id(root) {
            ad.put(
                "archetype_id",
                o(typed::make("ARCHETYPE_ID", [("value", s(id))])?),
            )?;
        }
        comp.put("archetype_details", o(ad))?;
    }
    if let Some(ad) = comp.object_mut("archetype_details") {
        ad.or_put("template_id", || {
            Ok(o(typed::make("TEMPLATE_ID", [("value", s(template_id))])?))
        })?;
    }
    Ok(())
}

/// Applies each node's `ctx/` defaults (`master06`) and fills its mandatory
/// fields, in pre-order.
fn complete_tree(
    obj: &mut dyn RmObject,
    defaults: Option<&ctx::CtxDefaults>,
) -> Result<(), FlatError> {
    if let Some(defaults) = defaults {
        defaults::apply_node_defaults(obj, defaults)?;
    }
    let rm_type = obj.rm_type();
    fill_structural_mandatory(obj, rm_type)?;
    for (_, child) in obj.children_mut() {
        if !holds_no_structure(child.rm_type()) {
            complete_tree(child, defaults)?;
        }
    }
    Ok(())
}

/// Whether an object of `rm_type` contains no LOCATABLE: a data value, code
/// phrase, identifier, reference or party proxy.
fn holds_no_structure(rm_type: &str) -> bool {
    use openehr_rm::v1_2::model::is_a;
    rm_type == "CODE_PHRASE"
        || is_a(rm_type, "DATA_VALUE")
        || is_a(rm_type, "PARTY_PROXY")
        || is_a(rm_type, "OBJECT_ID")
        || is_a(rm_type, "OBJECT_REF")
}

/// Fills the absent RM-mandatory fields the simplified wire never carries.
fn fill_structural_mandatory(obj: &mut dyn RmObject, rm_type: &str) -> Result<(), FlatError> {
    let date_time =
        |value: &str| -> Result<RmValue, FlatError> { Ok(o(typed::dv_date_time(value)?)) };
    match rm_type {
        "HISTORY" => obj.or_put("origin", || date_time(DEFAULT_TIME)),
        "POINT_EVENT" | "EVENT" => {
            obj.or_put("time", || date_time(DEFAULT_TIME))?;
            obj.or_put("data", empty_item_tree)
        }
        "INTERVAL_EVENT" => {
            obj.or_put("time", || date_time(DEFAULT_TIME))?;
            obj.or_put("data", empty_item_tree)?;
            // width + math_function are RM-mandatory on INTERVAL_EVENT but
            // never data-entry leaves (openEHR `146` = "mean").
            obj.or_put("width", || {
                Ok(o(typed::make("DV_DURATION", [("value", s("P0D"))])?))
            })?;
            obj.or_put("math_function", || {
                Ok(o(typed::dv_coded_text("mean", "openehr", "146")?))
            })
        }
        // ITEM_SINGLE's mandatory `item`: a null-flavoured ELEMENT, `253` "unknown".
        "ITEM_SINGLE" => obj.or_put("item", || {
            Ok(o(typed::make(
                "ELEMENT",
                [
                    ("archetype_node_id", s("at0001")),
                    ("name", o(typed::dv_text("Element")?)),
                    (
                        "null_flavour",
                        o(typed::dv_coded_text("unknown", "openehr", "253")?),
                    ),
                ],
            )?))
        }),
        "ACTIVITY" => {
            // master05 §ACTIVITY: "Will be set to /.*/ if not set explicit."
            obj.or_put("action_archetype_id", || Ok(s("/.*/")))?;
            obj.or_put("description", empty_item_tree)
        }
        "ISM_TRANSITION" => obj.or_put("current_state", || {
            Ok(o(typed::dv_coded_text("initial", "openehr", "524")?))
        }),
        "INSTRUCTION" => obj.or_put("narrative", || Ok(o(typed::dv_text("<narrative>")?))),
        "ACTION" => obj.or_put("ism_transition", || {
            Ok(o(access::construct("ISM_TRANSITION")?))
        }),
        // The ITEM_TREE / ITEM_LIST / CLUSTER `items` and the ITEM_TABLE `rows`
        // stay unwritten when empty (ITS-REST Resources.md §JSON Format).
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ITS-JSON `Data_structures/HISTORY.json` gives `events` `minItems: 1`,
    /// so neither a synthesised nor a completed HISTORY carries `events: []`.
    #[test]
    fn a_history_without_an_event_carries_no_events_member() {
        let built = structural_container("HISTORY", "at0001", "History").unwrap();
        assert!(!built.is_set("events"), "{built:?}");
        let mut obj = access::construct("HISTORY").unwrap();
        fill_structural_mandatory(&mut *obj, "HISTORY").unwrap();
        assert!(obj.is_set("origin") && !obj.is_set("events"), "{obj:?}");
    }

    /// The two `_` families a PARTY leaf consumes itself.
    #[test]
    fn a_party_leaf_consumes_its_relationship_and_identifier_families() {
        for base in [
            "PARTY_PROXY",
            "PARTY_SELF",
            "PARTY_IDENTIFIED",
            "PARTY_RELATED",
        ] {
            assert!(leaf_consumes_segment(base, "_relationship"), "{base}");
            assert!(leaf_consumes_segment(base, "_identifier"), "{base}");
            // Everything else still routes generically.
            assert!(!leaf_consumes_segment(base, "_feeder_audit"), "{base}");
        }
        // Only a party consumes them: on any other host the router owns the
        // segment (and reports an unknown suffix where there is no arm).
        assert!(!leaf_consumes_segment("DV_TEXT", "_identifier"));
        assert!(!leaf_consumes_segment("ELEMENT", "_relationship"));
    }

    #[test]
    fn infer_type_matches_level_removal_owners() {
        // master04 §Level Removal: the elided container attributes.
        assert_eq!(infer_type("data", Some("events")), "HISTORY");
        assert_eq!(infer_type("events", None), "POINT_EVENT");
        assert_eq!(infer_type("items", None), "ELEMENT");
        assert_eq!(infer_type("activities", None), "ACTIVITY");
        assert_eq!(infer_type("state", Some("items")), "ITEM_TREE");
    }
}

#[cfg(test)]
mod multiplicity_tests {
    use super::is_multiple;

    /// The model-driven set covers the COMPOSITION, HISTORY, ITEM and ENTRY
    /// containers.
    #[test]
    fn covers_legacy_hardcoded_set() {
        for attr in ["content", "items", "events", "activities"] {
            assert!(
                is_multiple(attr),
                "model-driven multiplicity set must include the legacy member `{attr}`",
            );
        }
    }

    /// `data` stays single: `OBSERVATION.data` and `ADMIN_ENTRY.data` are single,
    /// and `DV_MULTIMEDIA.data` is bytes.
    #[test]
    fn data_stays_single() {
        assert!(
            !is_multiple("data"),
            "`data` must not be treated as multi-valued (OBSERVATION.data is a single HISTORY)",
        );
    }

    /// The derivation now reaches genuinely multi-valued structural attributes
    /// beyond the legacy four.
    #[test]
    fn covers_reachable_multi_valued_attributes() {
        assert!(is_multiple("other_participations"));
    }

    /// An unknown / non-container attribute is not multi-valued.
    #[test]
    fn rejects_single_and_unknown() {
        assert!(!is_multiple("value"));
        assert!(!is_multiple("not_a_real_attribute"));
    }
}
