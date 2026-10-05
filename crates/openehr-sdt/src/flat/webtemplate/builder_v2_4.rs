// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `v2_4` OPT2 → Web Template walk (the ADL2 front end).
//!
//! The Web Template layer is the dialect-neutral seam: `ITS-REST
//! simplified_formats master04-basic_concepts.adoc` §"Web Template Metadata"
//! defines a Web Template as "a processed representation of an openEHR
//! Operational Template", and a `v2_4` OPT2 is one.
//! [`build_web_template_v2_4`] walks the `v2_4` constraint tree into the SAME
//! [`WebTemplate`] model [`super::builder`] produces and hands it to the shared
//! `shape` and `id` passes, so the whole downstream works unchanged and no
//! parallel pipeline exists.
//!
//! The AOM2 constraint model differs from OPT 1.4: it has no
//! `C_DV_QUANTITY`/`C_DV_ORDINAL`/`C_CODE_PHRASE` classes — a DV leaf is a
//! `C_COMPLEX_OBJECT` whose RM attributes are constrained by `C_ATTRIBUTE`s and
//! co-varying `C_ATTRIBUTE_TUPLE`s — and coded constraints are
//! `C_TERMINOLOGY_CODE`. This module carries the `v2_4`-specific
//! `build`/`inputs` half; the tree shaping is shared.
//!
//! NOTE: no openEHR spec governs the Web Template model itself — our own
//! design/extension; the validation-only constraint fields it populates come
//! from `AM/docs/AOM2/master03-archetype_package.adoc` §C_ATTRIBUTE,
//! §ARCHETYPE_SLOT.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_boolean::CBoolean;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::rules::expr_constraint::ExprConstraint;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_am::v2_4::beom::core::expression::Expression;
use openehr_base::prelude::{Cardinality, Interval, MultiplicityInterval, ProperInterval};
use openehr_lang::v1_1::beom::core::operator_kind::OperatorKind;

use super::model::{
    WebTemplate, WebTemplateArchetypeSlot, WebTemplateBindingCodedValue, WebTemplateCardinality,
    WebTemplateClosedAttribute, WebTemplateCodedValue, WebTemplateExistence, WebTemplateInput,
    WebTemplateInputType, WebTemplateNode, WebTemplateRange, WebTemplateStructuralStub,
    WebTemplateValidation,
};
use super::shape;

/// Build a [`WebTemplate`] from a parsed `v2_4` operational template (OPT2).
///
/// The document shape (`tree`/`children`, `inputs`, `aqlPath`), the node-id
/// derivation, and the compaction are governed by `ITS-REST simplified_formats
/// master04-basic_concepts.adoc` (§"Web Template Metadata", §"Node ID Generation
/// Rules", §"Level Removal"). `version` is the format version string; `semVer`
/// carries the OPT2 release version (`master07.05` — an ADL2/OPT2 concept the
/// OPT 1.4 form lacks).
///
/// # Errors
/// [`crate::flat::error::FlatError::InvalidTemplate`] if the template lacks an
/// archetype id.
pub fn build_web_template_v2_4(
    opt: &OperationalTemplate,
) -> Result<WebTemplate, crate::flat::error::FlatError> {
    let template_id = template_id_of(&opt.archetype_id);
    if template_id.is_empty() {
        return Err(crate::flat::error::FlatError::InvalidTemplate(
            "archetype_id is mandatory".to_owned(),
        ));
    }
    let default_language = opt.original_language.code_string.clone();
    let ctx = Ctx {
        languages: collect_languages(opt, &default_language),
        default_language,
        components: opt.component_terminologies.as_ref(),
    };

    // The OPT definition root is a plain `C_COMPLEX_OBJECT` (rm type COMPOSITION,
    // node_id = the concept code); its rubrics come from the OPT's own
    // terminology (constituents' terminologies live in `component_terminologies`
    // and are switched in as `C_ARCHETYPE_ROOT`s are entered).
    let root_co = CObject::CComplexObject(opt.definition.clone());
    let mut tree = build_node(
        &ctx,
        &opt.terminology,
        None,
        &root_co,
        "",
        shape::Identity::Archetyped,
    );
    // master04 §"Web Template Metadata": the root `nodeId` is the archetype id
    // (interface form), not the internal concept code.
    tree.node_id = Some(interface_id_of_hrid(&opt.archetype_id));
    tree.min = Some(1);
    tree.max = 1;

    let mut tree = shape::compact(tree, 1).unwrap_or_else(shape::tree_placeholder);
    shape::synthesize_in_context(&mut tree);
    super::id::build_ids(&mut tree);
    // Parse the (empty, for this front end) archetype-conformance walk plan so
    // the WebTemplate is uniform with the OPT-1.4 form; the v2_4 example is
    // validated by RM invariants + terminology, not this walk (module NOTE).
    crate::flat::validation::prepare_walk(&mut tree);

    Ok(WebTemplate {
        template_id,
        // OPT2 carries a semantic version (`master07.05`); the interface form
        // strips it, so surface the full release version here.
        sem_ver: Some(opt.archetype_id.release_version.clone()),
        version: shape::CURRENT_VERSION.to_owned(),
        default_language: ctx.default_language.clone(),
        languages: ctx.languages.clone(),
        tree,
        other_details: IndexMap::new(),
    })
}

/// The template languages: the default language, then every language the OPT's
/// own terminology or any constituent terminology defines a `term_definitions`
/// block for, first-seen order.
fn collect_languages(opt: &OperationalTemplate, default_language: &str) -> Vec<String> {
    let mut langs = vec![default_language.to_owned()];
    let mut push = |term: &ArchetypeTerminology| {
        for lang in term.term_definitions.keys() {
            if !langs.contains(lang) {
                langs.push(lang.clone());
            }
        }
    };
    push(&opt.terminology);
    if let Some(components) = &opt.component_terminologies {
        for term in components.values() {
            push(term);
        }
    }
    langs
}

/// Build context shared across the `v2_4` walk.
struct Ctx<'a> {
    default_language: String,
    languages: Vec<String>,
    /// The OPT's constituent (filler) terminologies, keyed by full archetype id
    /// (OPT2 master03 §Terminology); scope is switched to these as the walk
    /// enters an inlined `C_ARCHETYPE_ROOT`.
    components: Option<&'a BTreeMap<String, ArchetypeTerminology>>,
}

impl Ctx<'_> {
    /// The constituent terminology for `archetype_ref`, if the OPT inlined one.
    fn scope(&self, archetype_ref: &str) -> Option<&ArchetypeTerminology> {
        self.components.and_then(|m| m.get(archetype_ref))
    }

    /// The localized rubric texts for `code` across every template language.
    fn localized(&self, term: &ArchetypeTerminology, code: &str) -> IndexMap<String, String> {
        let mut out = IndexMap::new();
        for lang in &self.languages {
            if let Some(t) = rubric_text(term, code, lang) {
                out.insert(lang.clone(), t);
            }
        }
        out
    }

    /// The localized `description` rubrics for `code` across every language.
    fn localized_descriptions(
        &self,
        term: &ArchetypeTerminology,
        code: &str,
    ) -> IndexMap<String, String> {
        let mut out = IndexMap::new();
        for lang in &self.languages {
            if let Some(d) = term
                .term_definitions
                .get(lang)
                .and_then(|m| m.get(code))
                .map(|t| t.description.clone())
                .filter(|s| !s.is_empty())
            {
                out.insert(lang.clone(), d);
            }
        }
        out
    }
}

/// The `text` rubric for `code` in `term` at `lang`.
fn rubric_text(term: &ArchetypeTerminology, code: &str, lang: &str) -> Option<String> {
    term.term_definitions
        .get(lang)?
        .get(code)
        .map(|t| t.text.clone())
        .filter(|s| !s.is_empty())
}

/// The external term bindings for `code` in `term` (master04 §"Web Template
/// Metadata": a `termBindings` map). `v2_4` `term_bindings` maps terminology →
/// code → target URI (`ARCHETYPE_TERMINOLOGY.term_bindings`); the bound URI is
/// surfaced as the coded value.
fn term_bindings_of(
    term: &ArchetypeTerminology,
    code: &str,
) -> IndexMap<String, WebTemplateBindingCodedValue> {
    let mut out = IndexMap::new();
    if code.is_empty() {
        return out;
    }
    let Some(bindings) = &term.term_bindings else {
        return out;
    };
    for (terminology, map) in bindings {
        if let Some(uri) = map.get(code)
            && !uri.trim().is_empty()
        {
            out.insert(
                terminology.clone(),
                WebTemplateBindingCodedValue {
                    value: uri.clone(),
                    terminology_id: terminology.clone(),
                },
            );
        }
    }
    out
}

// ── node build ───────────────────────────────────────────────────────────────

fn build_node(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    owner: Option<&CAttribute>,
    co: &CObject,
    parent_path: &str,
    identity: shape::Identity,
) -> WebTemplateNode {
    // A `C_ARCHETYPE_ROOT` switches the terminology scope to its constituent's
    // for its CHILDREN (OPT2 master03 §Terminology). The root node's OWN rubric
    // is the slot-level term the introducing artefact defines (ADL2 requires
    // the artefact that introduces a node id to define it — AOM2 master03
    // §Validity Rules), so it resolves in the OUTER scope first; the component
    // scope is only a last resort. Resolving the slot id in the component scope
    // first can false-positive on the constituent's own unrelated id codes.
    let child_term = match co {
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
            ctx.scope(&r.archetype_ref).unwrap_or(term)
        }
        _ => term,
    };
    let mut node = create_node(ctx, term, child_term, owner, co, parent_path, identity);
    build_children(ctx, child_term, co, &mut node);
    node
}

fn create_node(
    ctx: &Ctx,
    name_term: &ArchetypeTerminology,
    parent_term: &ArchetypeTerminology,
    owner: Option<&CAttribute>,
    co: &CObject,
    parent_path: &str,
    identity: shape::Identity,
) -> WebTemplateNode {
    let attr_name = owner.map(|a| a.rm_attribute_name.as_str());
    let archetyped = identity == shape::Identity::Archetyped;
    let rm_type = object_rm_type(co).to_owned();
    let arch_node_id = if archetyped {
        object_archetype_node_id(co)
    } else {
        String::new()
    };
    let (occ_min, max) = occurrences(object_occurrences(co));
    let min = meet_single_existence(occ_min, owner);

    let path = build_path(parent_path, attr_name, &arch_node_id);
    let mut node = WebTemplateNode::new(rm_type, path);
    node.node_id = if arch_node_id.is_empty() {
        None
    } else {
        Some(arch_node_id)
    };
    node.min = min;
    node.max = max;

    let code = if archetyped { object_node_id(co) } else { "" };
    if !code.is_empty() {
        // Resolve the rubric in the introducing artefact's scope first (for a
        // filler root that is the OUTER template terminology, which ADL2
        // obliges to define the slot code — AOM2 master03 §Validity Rules),
        // falling back to the component scope only when the outer one is
        // silent.
        node.name = rubric_text(name_term, code, &ctx.default_language)
            .or_else(|| rubric_text(parent_term, code, &ctx.default_language));
        node.localized_name.clone_from(&node.name);
        node.localized_names = ctx.localized(name_term, code);
        if node.localized_names.is_empty() {
            node.localized_names = ctx.localized(parent_term, code);
        }
        node.localized_descriptions = ctx.localized_descriptions(name_term, code);
        if node.localized_descriptions.is_empty() {
            node.localized_descriptions = ctx.localized_descriptions(parent_term, code);
        }
        node.name_code = Some(code.to_owned());
        node.term_bindings = term_bindings_of(name_term, code);
    }
    node
}

/// Builds the child nodes of one AOM2 constraint attribute into `children`.
///
/// The careflow-state alternatives of `ism_transition` collapse into the one
/// transition node master05 §ISM_TRANSITION maps, so they are built without
/// their at-code identity (see [`shape::MERGED_ATTRIBUTE`]) and the merged
/// node takes the ATTRIBUTE's occurrences — one required transition per ACTION
/// instance, not one per careflow state (AOM2 leaves `existence` unset unless
/// it overrides the RM, where `ACTION.ism_transition` is 1..1). An unfilled
/// slot or a residual proxy yields no node.
fn build_attribute_children(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    attr: &CAttribute,
    node: &WebTemplateNode,
    children: &mut Vec<WebTemplateNode>,
) {
    let merged = attr.rm_attribute_name == shape::MERGED_ATTRIBUTE;
    let identity = if merged {
        shape::Identity::AttributeOnly
    } else {
        shape::Identity::Archetyped
    };
    let mut built = Vec::new();
    for child_co in attr.children.iter().flatten() {
        if is_leaf_ignored(child_co) {
            continue;
        }
        built.push(build_node(
            ctx,
            term,
            Some(attr),
            child_co,
            &node.aql_path,
            identity,
        ));
    }
    if !merged {
        children.extend(built);
        return;
    }
    if let Some(mut transition) = shape::merge_alternatives(built) {
        let (min, max) = attr.existence.as_ref().map_or((Some(1), 1), occ);
        transition.min = min;
        transition.max = max;
        children.push(transition);
    }
}

fn build_children(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
    node: &mut WebTemplateNode,
) {
    let is_data_value = node.rm_type.starts_with("DV_");
    let recurse_attrs = !is_data_value || node.rm_type.starts_with("DV_INTERVAL");

    let mut children = Vec::new();
    if recurse_attrs {
        for attr in co_attributes(co) {
            // The name attribute names the node (master04 §"Field Identifiers").
            if attr.rm_attribute_name != "name" {
                build_attribute_children(ctx, term, attr, node, &mut children);
            }
        }
    }

    // A party node keeps its inputs even when it HAS children — master05's
    // three PARTY_PROXY tables put the party suffixes and the `/relationship` +
    // `/_identifier:i` sub-paths on the SAME node (the ADL 1.4 builder's rule,
    // verbatim). Every other leaf family loses its inputs to a child, which
    // means the node is a container rather than a datum.
    if has_inputs(&node.rm_type) && (children.is_empty() || is_party(&node.rm_type)) {
        let (built, ptypes) = build_inputs(ctx, term, &node.rm_type, co);
        node.inputs = built;
        node.proportion_types = ptypes;
    }

    node.cardinalities = cardinalities(co, &node.aql_path);
    node.card_all = all_cardinalities(co, &node.aql_path);
    // Existence / closed-attribute / structural-stub capture is meaningful only
    // for attribute-recursing (structural) nodes; a DV leaf's constraints are
    // handled by `inputs` (mirrors the OPT-1.4 front end's `recurse_attrs` guard).
    if recurse_attrs {
        node.existence = existence_constraints(co, &node.aql_path);
        node.closed_attributes = closed_attributes(co, &node.aql_path);
        if shape::is_entry_family(&node.rm_type) {
            node.structural_stubs = structural_stubs(ctx, term, co);
        }
    }
    node.children = children;
    shape::post_process(node);
}

/// Whether a child `C_OBJECT` yields no web-template node (an unfilled slot or a
/// residual proxy; OPT2 fillers/proxies are inlined by `create_opt`, so a
/// surviving one is a runtime extension point).
fn is_leaf_ignored(co: &CObject) -> bool {
    matches!(
        co,
        CObject::ArchetypeSlot(_) | CObject::CComplexObjectProxy(_)
    )
}

/// Whether a node of this RM type is a web-template LEAF — one that carries
/// `inputs` rather than child nodes. The `v2_4` twin of the ADL 1.4 builder's
/// rule, including the PARTY arm: master05 §§PARTY_SELF, PARTY_IDENTIFIED,
/// PARTY_RELATED each get their own mapping table and share the party suffix
/// rows, so a slot narrowed to `PARTY_RELATED` is a party leaf like the other
/// two — its `relationship` is a DV_CODED_TEXT sub-path (master05
/// §"PARTY_RELATED performer"), not a demotion to a container.
fn has_inputs(rm_type: &str) -> bool {
    rm_type.starts_with("DV_")
        || is_party(rm_type)
        || rm_type == "CODE_PHRASE"
        || rm_type == "TERMINOLOGY_CODE"
}

/// The `PARTY_PROXY` family (the `v2_4` twin of the ADL 1.4 builder's rule).
fn is_party(rm_type: &str) -> bool {
    matches!(
        rm_type,
        "PARTY_PROXY" | "PARTY_IDENTIFIED" | "PARTY_RELATED"
    )
}

// ── inputs (the v2_4 RM-type → inputs mapping) ───────────────────────────────

fn build_inputs(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    rm_type: &str,
    co: &CObject,
) -> (Vec<WebTemplateInput>, Vec<String>) {
    let base = rm_type.split('<').next().unwrap_or(rm_type);
    let mut proportion_types = Vec::new();
    let inputs = match base {
        "DV_TEXT" | "DV_PARAGRAPH" | "DV_URI" | "DV_EHR_URI" | "DV_MULTIMEDIA" => {
            vec![text_input(cstring_under(co, "value"), None)]
        }
        "DV_CODED_TEXT" | "DV_STATE" => coded_text_inputs(ctx, term, co),
        "CODE_PHRASE" | "TERMINOLOGY_CODE" => code_phrase_inputs(ctx, term, co),
        "DV_QUANTITY" => quantity_inputs(ctx, term, co),
        "DV_COUNT" => vec![count_input(co)],
        "DV_PROPORTION" => proportion_inputs(co, &mut proportion_types),
        "DV_ORDINAL" => vec![ordinal_input(ctx, term, co, false)],
        "DV_SCALE" => vec![ordinal_input(ctx, term, co, true)],
        "DV_BOOLEAN" => vec![boolean_input(co)],
        "DV_DATE" => vec![temporal_input(co, WebTemplateInputType::Date)],
        "DV_DATE_TIME" => vec![temporal_input(co, WebTemplateInputType::Datetime)],
        "DV_TIME" => vec![temporal_input(co, WebTemplateInputType::Time)],
        "DV_DURATION" => vec![duration_input(co)],
        "DV_IDENTIFIER" => ["id", "type", "issuer", "assigner"]
            .into_iter()
            .map(|s| text_input(cstring_under(co, s), Some(s)))
            .collect(),
        "DV_PARSABLE" => ["value", "formalism"]
            .into_iter()
            .map(|s| text_input(cstring_under(co, s), Some(s)))
            .collect(),
        // The three PARTY_PROXY subtype tables share these rows — master05
        // §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED.
        "PARTY_PROXY" | "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            ["id", "id_scheme", "id_namespace", "name"]
                .into_iter()
                .map(|s| text_input(cstring_under(co, s), Some(s)))
                .collect()
        }
        _ => Vec::new(),
    };
    (inputs, proportion_types)
}

fn text_input(cstring: Option<&CString>, suffix: Option<&str>) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Text, suffix);
    if let Some(cs) = cstring {
        for entry in cs.constraint.iter().flatten() {
            if let Some(regex) = delimited_regex(entry) {
                input.validation = Some(WebTemplateValidation {
                    pattern: Some(regex.to_owned()),
                    ..Default::default()
                });
            } else {
                input
                    .list
                    .push(WebTemplateCodedValue::new(entry, Some(entry.clone())));
            }
        }
        // An `/.../ ` regex or an open list leaves the list open; a closed literal
        // list (no regex) constrains it.
        input.list_open = Some(input.list.is_empty());
    }
    input
}

fn coded_text_inputs(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
) -> Vec<WebTemplateInput> {
    let Some(ctc) = terminology_code_under(co, "defining_code") else {
        // No `defining_code` constraint: a free coded-text pair (master04
        // §"Attribute Suffixes": `code`/`value`).
        return external_coded_inputs();
    };
    coded_inputs_from(ctx, term, ctc)
}

fn code_phrase_inputs(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
) -> Vec<WebTemplateInput> {
    // A CODE_PHRASE leaf is itself a `C_TERMINOLOGY_CODE`.
    match co {
        CObject::CTerminologyCode(ctc) => coded_inputs_from(ctx, term, ctc),
        _ => external_coded_inputs(),
    }
}

/// Build the `code` coded input from a `C_TERMINOLOGY_CODE` (an at-code, or an
/// ac-code resolving to an archetype-local value set). AOM2
/// `AM/docs/AOM2/master02` §`C_TERMINOLOGY_CODE`.
fn coded_inputs_from(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    ctc: &CTerminologyCode,
) -> Vec<WebTemplateInput> {
    let codes = expand_codes(term, &ctc.constraint);
    if codes.is_empty() {
        return external_coded_inputs();
    }
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = codes
        .iter()
        .map(|code| coded_value(ctx, term, code))
        .collect();
    // Local (archetype-defined) value sets have terminology `local` — left
    // implicit (the reference form omits `terminology: "local"`).
    vec![input]
}

/// The at-codes a `C_TERMINOLOGY_CODE.constraint` admits: an ac-code resolves to
/// its archetype-local value-set members; an at-code is itself the singleton;
/// an empty constraint admits nothing enumerable.
fn expand_codes(term: &ArchetypeTerminology, constraint: &str) -> Vec<String> {
    let c = constraint.trim();
    if c.is_empty() {
        return Vec::new();
    }
    if c.starts_with("ac") {
        return term
            .value_sets
            .as_ref()
            .and_then(|vs| vs.get(c))
            .map(|vs| vs.members.to_vec())
            .unwrap_or_default();
    }
    vec![c.to_owned()]
}

fn coded_value(ctx: &Ctx, term: &ArchetypeTerminology, code: &str) -> WebTemplateCodedValue {
    let label = rubric_text(term, code, &ctx.default_language).unwrap_or_else(|| code.to_owned());
    let mut cv = WebTemplateCodedValue::new(code, Some(label));
    cv.localized_labels = ctx.localized(term, code);
    cv.term_bindings = term_bindings_of(term, code);
    cv
}

/// The `code`/`value` free-text inputs of an unconstrained coded leaf (master04
/// §"Attribute Suffixes").
fn external_coded_inputs() -> Vec<WebTemplateInput> {
    vec![
        WebTemplateInput::new(WebTemplateInputType::Text, Some("code")),
        WebTemplateInput::new(WebTemplateInputType::Text, Some("value")),
    ]
}

/// Appends the units of the co-varying `[magnitude, units]` (and optional
/// `precision`) tuple rows (AOM2 `C_ATTRIBUTE_TUPLE`); each row fixes one unit
/// with its magnitude range and precision.
fn push_tuple_units(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
    units: &mut WebTemplateInput,
) {
    for tuple in co_attribute_tuples(co) {
        let names: Vec<&str> = tuple
            .members
            .iter()
            .flatten()
            .map(|m| m.rm_attribute_name.as_str())
            .collect();
        if !names.contains(&"units") {
            continue;
        }
        for row in tuple.tuples.iter().flatten() {
            let Some((unit, validation)) = tuple_row_unit(row, &names) else {
                continue;
            };
            let mut cv = WebTemplateCodedValue::new(unit, Some(unit.clone()));
            cv.localized_labels = ctx.localized(term, unit);
            if !validation.is_empty() {
                cv.validation = Some(validation);
            }
            units.list.push(cv);
        }
    }
}

/// Reads one `C_ATTRIBUTE_TUPLE` row as a unit plus the validation its
/// co-varying members carry.
///
/// `names` positions the row's members by RM attribute name. `None` when the
/// row's `units` member constrains no plain string (a delimited regex is a
/// pattern, not a unit).
fn tuple_row_unit<'a>(
    row: &'a CPrimitiveTuple,
    names: &[&str],
) -> Option<(&'a String, WebTemplateValidation)> {
    let mut unit_value = None;
    let mut validation = WebTemplateValidation::default();
    for (i, name) in names.iter().enumerate() {
        match (*name, row.members.get(i)) {
            ("units", Some(CPrimitiveObject::CString(cs))) => {
                unit_value = cs
                    .constraint
                    .iter()
                    .flatten()
                    .find(|s| delimited_regex(s).is_none());
            }
            ("magnitude", Some(CPrimitiveObject::CReal(cr))) => {
                validation.range = cr.constraint.iter().flatten().next().and_then(real_range);
            }
            ("precision", Some(CPrimitiveObject::CInteger(ci))) => {
                validation.precision = ci.constraint.iter().flatten().next().and_then(int_range);
            }
            _ => {}
        }
    }
    unit_value.map(|unit| (unit, validation))
}

fn quantity_inputs(ctx: &Ctx, term: &ArchetypeTerminology, co: &CObject) -> Vec<WebTemplateInput> {
    let mut magnitude = WebTemplateInput::new(WebTemplateInputType::Decimal, Some("magnitude"));
    let mut units = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("unit"));

    push_tuple_units(ctx, term, co, &mut units);

    // The plain (non-tuple) `units`/`magnitude` attribute forms.
    for cs in cstrings_under(co, "units") {
        for unit in cs
            .constraint
            .iter()
            .flatten()
            .filter(|s| delimited_regex(s).is_none())
        {
            if !units.list.iter().any(|cv| &cv.value == unit) {
                units
                    .list
                    .push(WebTemplateCodedValue::new(unit, Some(unit.clone())));
            }
        }
    }
    if let Some(cr) = creal_under(co, "magnitude")
        && let Some(range) = cr.constraint.iter().flatten().next().and_then(real_range)
    {
        magnitude.validation = Some(WebTemplateValidation {
            range: Some(range),
            ..Default::default()
        });
    }
    // A single allowed unit promotes its range onto the magnitude.
    if let [only] = units.list.as_slice()
        && magnitude.validation.is_none()
    {
        magnitude.validation.clone_from(&only.validation);
    }
    vec![magnitude, units]
}

fn count_input(co: &CObject) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Integer, None);
    if let Some(ci) = cinteger_under(co, "magnitude")
        && let Some(range) = ci.constraint.iter().flatten().next().and_then(int_range)
    {
        input.validation = Some(WebTemplateValidation {
            range: Some(range),
            ..Default::default()
        });
    }
    input
}

fn ordinal_input(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
    scale: bool,
) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, None);
    // The `[value, symbol]` tuple: `value` an Integer/Real ordinal, `symbol` a
    // terminology code (AOM2 `C_ATTRIBUTE_TUPLE`).
    for tuple in co_attribute_tuples(co) {
        let names: Vec<&str> = tuple
            .members
            .iter()
            .flatten()
            .map(|m| m.rm_attribute_name.as_str())
            .collect();
        if !names.contains(&"symbol") {
            continue;
        }
        for row in tuple.tuples.iter().flatten() {
            let Some((code, ordinal, scale_value)) = ordinal_row(term, &names, row) else {
                continue;
            };
            let mut cv = coded_value(ctx, term, &code);
            if scale {
                cv.scale = Some(scale_value);
            } else {
                cv.ordinal = Some(ordinal);
            }
            input.list.push(cv);
        }
    }
    input
}

/// One tuple row read into its symbol code and the two paired numeric readings.
///
/// `DV_ORDINAL.value` is an Integer and `DV_SCALE.value` a Real, so both are
/// tracked and no lossy `f64 as i32` cast is needed. A row with no symbol code
/// yields [`None`].
fn ordinal_row(
    term: &ArchetypeTerminology,
    names: &[&str],
    row: &CPrimitiveTuple,
) -> Option<(String, i32, f64)> {
    let mut code = None;
    let mut ordinal = 0_i32;
    let mut scale_value = 0_f64;
    for (i, name) in names.iter().enumerate() {
        match (*name, row.members.get(i)) {
            ("symbol", Some(CPrimitiveObject::CTerminologyCode(ctc))) => {
                code = expand_codes(term, &ctc.constraint).into_iter().next();
            }
            ("value", Some(CPrimitiveObject::CInteger(ci))) => {
                ordinal = ci
                    .constraint
                    .iter()
                    .flatten()
                    .next()
                    .and_then(point_i32)
                    .unwrap_or(0);
                scale_value = f64::from(ordinal);
            }
            ("value", Some(CPrimitiveObject::CReal(cr))) => {
                scale_value = cr
                    .constraint
                    .iter()
                    .flatten()
                    .next()
                    .and_then(point_f64)
                    .unwrap_or(0.0);
            }
            _ => {}
        }
    }
    Some((code?, ordinal, scale_value))
}

fn boolean_input(co: &CObject) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Boolean, None);
    if let Some(cb) = cboolean_under(co, "value") {
        let allows = |v: bool| cb.constraint.as_ref().is_some_and(|c| c.contains(&v));
        if allows(false) && !allows(true) {
            input.list.push(WebTemplateCodedValue::new(
                "false",
                Some("false".to_owned()),
            ));
        } else if allows(true) && !allows(false) {
            input
                .list
                .push(WebTemplateCodedValue::new("true", Some("true".to_owned())));
        }
    }
    input
}

fn temporal_input(co: &CObject, ty: WebTemplateInputType) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(ty, None);
    let pattern = match ty {
        WebTemplateInputType::Date => cdate_pattern(co),
        WebTemplateInputType::Time => ctime_pattern(co),
        _ => cdatetime_pattern(co),
    };
    if pattern.is_some() {
        input.validation = Some(WebTemplateValidation {
            pattern,
            ..Default::default()
        });
    }
    input
}

/// The `DV_DURATION` per-field inputs, gated by the `C_DURATION` pattern (a `P…`
/// string listing the allowed fields). No openEHR spec governs the per-field
/// split — our own design/extension, matching the OPT-1.4 front end.
fn duration_input(co: &CObject) -> WebTemplateInput {
    // The v2_4 `DV_DURATION` value is a single Duration input; the field split
    // the OPT-1.4 form uses is not reconstructed here (the example generator
    // honours `duration_range` when present, which v2_4 leaves on the leaf's
    // pattern). A single Duration input keeps the leaf committable.
    let mut input = WebTemplateInput::new(WebTemplateInputType::Duration, None);
    if let Some(pattern) = cduration_pattern(co) {
        input.validation = Some(WebTemplateValidation {
            pattern: Some(pattern),
            ..Default::default()
        });
    }
    input
}

fn proportion_inputs(co: &CObject, proportion_types: &mut Vec<String>) -> Vec<WebTemplateInput> {
    let type_codes: Vec<i32> = cinteger_under(co, "type")
        .map(|ci| {
            ci.constraint
                .iter()
                .flatten()
                .filter_map(point_i32)
                .collect()
        })
        .unwrap_or_default();
    let is_integral = cboolean_under(co, "is_integral").is_some_and(|b| {
        b.constraint.as_ref().is_some_and(|c| c.contains(&true))
            && !b.constraint.as_ref().is_some_and(|c| c.contains(&false))
    });

    *proportion_types = if type_codes.is_empty() {
        super::PROPORTION_KINDS
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    } else {
        type_codes
            .iter()
            .filter_map(|c| super::PROPORTION_KINDS.get(usize::try_from(*c).unwrap_or(usize::MAX)))
            .map(|s| (*s).to_owned())
            .collect()
    };

    let ty = if is_integral {
        WebTemplateInputType::Integer
    } else {
        WebTemplateInputType::Decimal
    };
    ["numerator", "denominator"]
        .into_iter()
        .map(|suffix| {
            let mut input = WebTemplateInput::new(ty, Some(suffix));
            if let Some(cr) = creal_under(co, suffix)
                && let Some(range) = cr.constraint.iter().flatten().next().and_then(real_range)
            {
                input.validation = Some(WebTemplateValidation {
                    range: Some(range),
                    ..Default::default()
                });
            }
            input
        })
        .collect()
}

// ── cardinalities ────────────────────────────────────────────────────────────

fn cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        if !attr.is_multiple {
            continue;
        }
        let Some(card) = &attr.cardinality else {
            continue;
        };
        if requires_cardinality(card, attr.children.as_ref().map_or(0, Vec::len)) {
            let (min, max) = occ(&card.interval);
            out.push(WebTemplateCardinality {
                min,
                max,
                ids: None,
                path: format!("{node_path}/{}", attr.rm_attribute_name),
            });
        }
    }
    out
}

/// Whether a container cardinality genuinely constrains the container beyond
/// what the child count already implies (mirrors the OPT-1.4 front end).
fn requires_cardinality(card: &Cardinality, children_count: usize) -> bool {
    let (min, max) = occ(&card.interval);
    let Some(min) = min else {
        return max != -1;
    };
    if min == 0 {
        return max != -1 && i32::try_from(children_count).unwrap_or(i32::MAX) > max;
    }
    let count = i32::try_from(children_count).unwrap_or(i32::MAX);
    if min == 1 && max == 1 && children_count == 1 {
        false
    } else {
        min > 1 || (max != -1 && max < count)
    }
}

// ── archetype-conformance constraint capture (validation-only fields) ─────────
//
// The v2_4 front end fills the same validation-only constraint fields the
// OPT-1.4 one does, so the archetype-conformance walk runs identically for both
// dialects. AOM2 expresses them as `C_ATTRIBUTE.existence`/`.cardinality`,
// node-identified `C_OBJECT` alternatives, and `ARCHETYPE_SLOT` (AOM2
// `AM/docs/AOM2/master03-archetype_package.adoc` §C_ATTRIBUTE,
// §ARCHETYPE_SLOT), captured before compaction hoists wrappers.

/// Capture EVERY constraining `C_ATTRIBUTE.cardinality` (AOM2 §C_ATTRIBUTE:
/// "Cardinality constraint of attribute, if a container attribute") for the
/// validation walk: any interval with a lower bound `>= 1` or a bounded upper
/// bound constrains the container. A superset of the serialized [`cardinalities`]
/// selection, so `0..1`/`1..1`/`1..*` container bounds are enforced from
/// [`WebTemplateNode::card_all`] too (mirrors the OPT-1.4 front end).
fn all_cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        let Some(card) = &attr.cardinality else {
            continue;
        };
        let (min, max) = occ(&card.interval);
        if min.unwrap_or(0) >= 1 || max != -1 {
            out.push(WebTemplateCardinality {
                min,
                max,
                ids: None,
                path: format!("{node_path}/{}", attr.rm_attribute_name),
            });
        }
    }
    out
}

/// Capture the AOM2 `C_ATTRIBUTE.existence` constraints (AOM2 §C_ATTRIBUTE:
/// existence "indicates whether its target object exists or not, i.e. is
/// mandatory or not") with a lower bound `>= 1`, keyed by the attribute's
/// absolute archetype path. `existence` is `Option` in AOM2 — "Only set if it
/// overrides the underlying reference model or parent archetype" (AOM2
/// §C_ATTRIBUTE) — so only an explicitly-mandated attribute is captured, biasing
/// toward confident violations exactly as the OPT-1.4 front end does (where the
/// RM-mandatory presence an AOM2 template leaves to the RM is still covered by the
/// RM-invariant pass + occurrences). `name` is excluded — it names the node
/// (master04 §"Field Identifiers").
fn existence_constraints(co: &CObject, node_path: &str) -> Vec<WebTemplateExistence> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        if attr.rm_attribute_name == "name" {
            continue;
        }
        let Some(ex) = &attr.existence else {
            continue;
        };
        let (min, max) = occ(ex);
        let min = min.unwrap_or(0);
        if min < 1 {
            continue;
        }
        // A container mandates the attribute's presence regardless of member
        // cardinality (existence and cardinality are orthogonal, AOM2
        // §C_ATTRIBUTE). A single-valued attribute counts when it is object-valued
        // (a navigable RM instance attribute) or a bare mandatory attribute with no
        // value constraint — a pure primitive-value constraint never appears as a
        // navigable instance attribute.
        let object_valued = attr.is_multiple
            || attr.children.as_ref().is_none_or(Vec::is_empty)
            || attr.children.iter().flatten().any(is_object_valued);
        if !object_valued {
            continue;
        }
        out.push(WebTemplateExistence {
            min,
            max,
            path: format!("{node_path}/{}", attr.rm_attribute_name),
        });
    }
    out
}

/// Whether a child `C_OBJECT` is an object-valued (navigable) constraint — a
/// complex object, an inlined archetype root, a proxy, or a slot — as opposed to
/// a primitive value constraint (`C_STRING`/`C_INTEGER`/… never appears as a
/// navigable RM instance attribute).
fn is_object_valued(co: &CObject) -> bool {
    matches!(
        co,
        CObject::CComplexObject(_) | CObject::CComplexObjectProxy(_) | CObject::ArchetypeSlot(_)
    )
}

/// Capture the closed-archetype constraints (the AOM2 closed-world direction):
/// per attribute carrying node-identified `C_OBJECT` alternatives and/or
/// `ARCHETYPE_SLOT`s, the admissible child identities keyed by the attribute's
/// absolute archetype path. An attribute carrying an unresolved
/// `C_COMPLEX_OBJECT_PROXY` is left OPEN (its target is not resolved in this front
/// end — matching the OPT-1.4 internal-ref handling), and a purely
/// primitive/unconstrained attribute is never closed. `name` is matched by
/// predicate, not closure (master04 §"Field Identifiers").
fn closed_attributes(co: &CObject, node_path: &str) -> Vec<WebTemplateClosedAttribute> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        if attr.rm_attribute_name == "name" {
            continue;
        }
        // An unresolved proxy makes the admissible set uncertain: leave OPEN
        // rather than risk over-rejecting.
        if attr
            .children
            .iter()
            .flatten()
            .any(|c| matches!(c, CObject::CComplexObjectProxy(_)))
        {
            continue;
        }
        let (allowed_ids, slots) = admissible_children(attr);
        if allowed_ids.is_empty() && slots.is_empty() {
            continue; // Open attribute (no node-id alternatives, no slot).
        }
        out.push(WebTemplateClosedAttribute {
            path: format!("{node_path}/{}", attr.rm_attribute_name),
            allowed_ids,
            slots,
        });
    }
    out
}

/// The admissible child identities of one attribute: its node-identified
/// LOCATABLE alternatives and its archetype slots.
///
/// An alternative is an at/id-coded `C_COMPLEX_OBJECT`, or an inlined
/// `C_ARCHETYPE_ROOT` carrying its interface archetype id; primitive value
/// constraints never participate in sibling closure.
fn admissible_children(attr: &CAttribute) -> (Vec<String>, Vec<WebTemplateArchetypeSlot>) {
    let mut allowed_ids: Vec<String> = Vec::new();
    let mut slots: Vec<WebTemplateArchetypeSlot> = Vec::new();
    for child in attr.children.iter().flatten() {
        match child {
            CObject::ArchetypeSlot(s) => slots.push(archetype_slot(s)),
            CObject::CComplexObject(_) => {
                let id = object_archetype_node_id(child);
                if !id.is_empty() {
                    allowed_ids.push(id);
                }
            }
            _ => {}
        }
    }
    (allowed_ids, slots)
}

/// The validation-only slot record from an AOM2 `ARCHETYPE_SLOT`: its constrained
/// RM type, occurrences bounds, and the archetype-id regexes lifted from the
/// include/exclude assertions (AOM2 §ARCHETYPE_SLOT: `includes`/`excludes` are
/// ASSERTIONs of the form `EXPR_ARCHETYPE_REF matches EXPR_ARCHETYPE_ID_CONSTRAINT`).
fn archetype_slot(s: &ArchetypeSlot) -> WebTemplateArchetypeSlot {
    let (min, max) = occurrences(s.occurrences.as_ref());
    WebTemplateArchetypeSlot {
        rm_type: s.rm_type_name.clone(),
        min: min.unwrap_or(0).max(0),
        max,
        includes: s
            .includes
            .iter()
            .flatten()
            .filter_map(slot_pattern)
            .collect(),
        excludes: s
            .excludes
            .iter()
            .flatten()
            .filter_map(slot_pattern)
            .collect(),
    }
}

/// The archetype-id regex of a slot `ASSERTION`, read from its EXPRESSION TREE.
///
/// `ASSERTION.expression` is the "Root of expression tree" and
/// `string_expression` only its "String form of expression"
/// (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package), so the
/// tree is the authority: the slot constraint's core expression is
/// `<reference> matches {/<regex>/}` (`ADL2/master04.3` §Slots based on Lexical
/// Archetype Identifiers), whose right operand carries one delimited regex in a
/// `C_STRING.constraint` (`AOM2/master04.5` §`C_STRING`). Any other assertion
/// shape (the §Slots based on other Constraints form, a literal-value list)
/// yields `None`.
fn slot_pattern(a: &Assertion) -> Option<String> {
    let Expression::ExprBinaryOperator(op) = a.expression.as_ref() else {
        return None;
    };
    if op.operator != OperatorKind::Matches {
        return None;
    }
    let cstring = match op.right_operand.as_ref() {
        Expression::ExprConstraint(ExprConstraint::ExprArchetypeIdConstraint(c)) => &c.item,
        Expression::ExprConstraint(ExprConstraint::ExprConstraint(c)) => match &c.item {
            CPrimitiveObject::CString(s) => s,
            _ => return None,
        },
        _ => return None,
    };
    match cstring.constraint.as_deref() {
        Some([one]) => delimited_regex_body(one).map(str::to_owned),
        _ => None,
    }
}

/// The body of a `/re/` or `^re^` delimited regex, or `None` for a literal
/// string.
///
/// `AOM2/master04.5` §`C_STRING` types `constraint` as literal strings and/or
/// regular expressions delimited by `/`; `ADL2/master04.5` §Regular Expression
/// admits `^…^` as the lexical alternative when the body contains `/`.
fn delimited_regex_body(entry: &str) -> Option<&str> {
    let trimmed = entry.trim();
    ['/', '^'].into_iter().find_map(|delimiter| {
        trimmed
            .strip_prefix(delimiter)
            .and_then(|rest| rest.strip_suffix(delimiter))
    })
}

/// The RM-mandatory structural attributes of an ENTRY whose value the FLAT/TDD
/// composition builder synthesises when the simplified form carries no content
/// under them (the ENTRY `data`/`state`/`protocol` plus `ACTION.description`;
/// RM `composition`).
const ENTRY_STRUCTURAL_ATTRS: [&str; 4] = ["data", "description", "protocol", "state"];

/// Capture the structural stubs for an ENTRY node (mirrors the OPT-1.4 front
/// end): for each RM-mandatory structural attribute the OPT constrains with a
/// node-identified structural child, record its RM type, archetype node id, and
/// rubric name. The compactor drops such a wrapper when it carries no leaf
/// content, so this is the only surviving record of the *constrained* identity —
/// the composition builder synthesises the empty attribute from it (AOM2
/// §C_ATTRIBUTE: a constrained attribute must be filled by a conforming value)
/// rather than a blind `at0001`/`id1` placeholder a closed-archetype walk rejects.
fn structural_stubs(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
) -> Vec<WebTemplateStructuralStub> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        if !ENTRY_STRUCTURAL_ATTRS.contains(&attr.rm_attribute_name.as_str()) {
            continue;
        }
        for child in attr.children.iter().flatten() {
            // Only a node-identified structural child gives a concrete identity to
            // stamp; a slot / proxy leaves the attribute to its placeholder.
            let CObject::CComplexObject(_) = child else {
                continue;
            };
            let node_id = object_archetype_node_id(child);
            if node_id.is_empty() {
                continue;
            }
            out.push(WebTemplateStructuralStub {
                attr: attr.rm_attribute_name.clone(),
                rm_type: object_rm_type(child).to_owned(),
                node_id,
                name: rubric_text(term, object_node_id(child), &ctx.default_language),
            });
            break; // first node-identified structural child under this attribute
        }
    }
    out
}

// ── v2_4 C_OBJECT navigation ─────────────────────────────────────────────────

fn co_attributes(co: &CObject) -> &[CAttribute] {
    match co {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => {
            d.attributes.as_deref().unwrap_or_default()
        }
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
            r.attributes.as_deref().unwrap_or_default()
        }
        _ => &[],
    }
}

fn co_attribute_tuples(co: &CObject) -> &[CAttributeTuple] {
    match co {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => {
            d.attribute_tuples.as_deref().unwrap_or_default()
        }
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
            r.attribute_tuples.as_deref().unwrap_or_default()
        }
        _ => &[],
    }
}

fn attr_children<'a>(co: &'a CObject, name: &str) -> impl Iterator<Item = &'a CObject> {
    co_attributes(co)
        .iter()
        .filter(move |a| a.rm_attribute_name == name)
        .flat_map(|a| a.children.iter().flatten())
}

fn cstring_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CString> {
    attr_children(co, name).find_map(|c| match c {
        CObject::CString(cs) => Some(cs),
        _ => None,
    })
}

fn cstrings_under<'a>(co: &'a CObject, name: &str) -> impl Iterator<Item = &'a CString> {
    attr_children(co, name).filter_map(|c| match c {
        CObject::CString(cs) => Some(cs),
        _ => None,
    })
}

fn creal_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CReal> {
    attr_children(co, name).find_map(|c| match c {
        CObject::CReal(cr) => Some(cr),
        _ => None,
    })
}

fn cinteger_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CInteger> {
    attr_children(co, name).find_map(|c| match c {
        CObject::CInteger(ci) => Some(ci),
        _ => None,
    })
}

fn cboolean_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CBoolean> {
    attr_children(co, name).find_map(|c| match c {
        CObject::CBoolean(cb) => Some(cb),
        _ => None,
    })
}

fn terminology_code_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CTerminologyCode> {
    attr_children(co, name).find_map(|c| match c {
        CObject::CTerminologyCode(ctc) => Some(ctc),
        _ => None,
    })
}

fn cdate_pattern(co: &CObject) -> Option<String> {
    attr_children(co, "value").find_map(|c| match c {
        CObject::CDate(cd) => cd.pattern_constraint.clone(),
        _ => None,
    })
}

fn ctime_pattern(co: &CObject) -> Option<String> {
    attr_children(co, "value").find_map(|c| match c {
        CObject::CTime(ct) => ct.pattern_constraint.clone(),
        _ => None,
    })
}

fn cdatetime_pattern(co: &CObject) -> Option<String> {
    attr_children(co, "value").find_map(|c| match c {
        CObject::CDateTime(cdt) => cdt.pattern_constraint.clone(),
        _ => None,
    })
}

fn cduration_pattern(co: &CObject) -> Option<String> {
    attr_children(co, "value").find_map(|c| match c {
        CObject::CDuration(cd) => cd.pattern_constraint.clone(),
        _ => None,
    })
}

// ── object metadata ────────────────────────────────────────────────────────

fn object_rm_type(co: &CObject) -> &str {
    match co {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => &d.rm_type_name,
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => &r.rm_type_name,
        CObject::CComplexObjectProxy(p) => &p.rm_type_name,
        CObject::ArchetypeSlot(s) => &s.rm_type_name,
        CObject::CBoolean(c) => &c.rm_type_name,
        CObject::CInteger(c) => &c.rm_type_name,
        CObject::CReal(c) => &c.rm_type_name,
        CObject::CString(c) => &c.rm_type_name,
        CObject::CTerminologyCode(c) => &c.rm_type_name,
        CObject::CDate(c) => &c.rm_type_name,
        CObject::CTime(c) => &c.rm_type_name,
        CObject::CDateTime(c) => &c.rm_type_name,
        CObject::CDuration(c) => &c.rm_type_name,
    }
}

fn object_node_id(co: &CObject) -> &str {
    match co {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => &d.node_id,
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => &r.node_id,
        CObject::CComplexObjectProxy(p) => &p.node_id,
        CObject::ArchetypeSlot(s) => &s.node_id,
        CObject::CBoolean(c) => &c.node_id,
        CObject::CInteger(c) => &c.node_id,
        CObject::CReal(c) => &c.node_id,
        CObject::CString(c) => &c.node_id,
        CObject::CTerminologyCode(c) => &c.node_id,
        CObject::CDate(c) => &c.node_id,
        CObject::CTime(c) => &c.node_id,
        CObject::CDateTime(c) => &c.node_id,
        CObject::CDuration(c) => &c.node_id,
    }
}

/// The RM `archetype_node_id`: the archetype id (interface form) at an inlined
/// archetype root, else the constraint node id (at/id-code).
fn object_archetype_node_id(co: &CObject) -> String {
    match co {
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
            interface_id(&r.archetype_ref)
        }
        _ => object_node_id(co).to_owned(),
    }
}

fn object_occurrences(co: &CObject) -> Option<&MultiplicityInterval> {
    match co {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d.occurrences.as_ref(),
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => r.occurrences.as_ref(),
        CObject::CComplexObjectProxy(p) => p.occurrences.as_ref(),
        CObject::ArchetypeSlot(s) => s.occurrences.as_ref(),
        CObject::CBoolean(c) => c.occurrences.as_ref(),
        CObject::CInteger(c) => c.occurrences.as_ref(),
        CObject::CReal(c) => c.occurrences.as_ref(),
        CObject::CString(c) => c.occurrences.as_ref(),
        CObject::CTerminologyCode(c) => c.occurrences.as_ref(),
        CObject::CDate(c) => c.occurrences.as_ref(),
        CObject::CTime(c) => c.occurrences.as_ref(),
        CObject::CDateTime(c) => c.occurrences.as_ref(),
        CObject::CDuration(c) => c.occurrences.as_ref(),
    }
}

/// The node's effective `min`: its own occurrences lower bound met with the
/// owning SINGLE attribute's `C_ATTRIBUTE.existence` lower bound.
///
/// ADL 2 `AM/docs/ADL2/master04.3-cadl_complex_types.adoc` §Occurrences: "the
/// occurrences of an object that is the value of a single-valued attribute can
/// only be `0..1` or `1..1`, and this is already defined by the attribute's
/// `existence`" — it is used there only "to exclude a possibility defined in a
/// parent archetype". So an optional single attribute never yields a mandatory
/// child, whatever occurrences the constraint carries, and a `0..0` exclusion
/// still lands at `0`.
///
/// AOM2 keeps `existence` optional "since [it is] only needed to override the
/// settings from the reference model"
/// (`AM/docs/AOM2/master04.2-constraint_model-semantics.adoc` §"Attribute
/// Nodes"), so an unset existence leaves occurrences untouched — the RM's own
/// bound is not knowable here. A container attribute is left alone for the same
/// reason as in the OPT-1.4 front end: there occurrences is the significant
/// constraint.
fn meet_single_existence(occurrences_min: Option<i32>, owner: Option<&CAttribute>) -> Option<i32> {
    let Some(existence) = owner
        .filter(|a| !a.is_multiple)
        .and_then(|a| a.existence.as_ref())
    else {
        return occurrences_min;
    };
    let existence_min = occ(existence).0.unwrap_or(0);
    occurrences_min.map(|m| m.min(existence_min))
}

/// `(min, max)` from an object's occurrences; an unset occurrences defaults to
/// `0..1` (a permitted, single, optional node — the safe default that never
/// forces a spurious mandatory node into the example skeleton).
fn occurrences(iv: Option<&MultiplicityInterval>) -> (Option<i32>, i32) {
    iv.map_or((Some(0), 1), occ)
}

fn occ(iv: &MultiplicityInterval) -> (Option<i32>, i32) {
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded {
        -1
    } else {
        iv.upper.unwrap_or_else(|| iv.lower.unwrap_or(0))
    };
    (min, max)
}

fn build_path(parent_path: &str, attr_name: Option<&str>, arch_node_id: &str) -> String {
    let Some(attr) = attr_name else {
        return String::new(); // root
    };
    let predicate = if arch_node_id.is_empty() {
        String::new()
    } else {
        format!("[{arch_node_id}]")
    };
    format!("{parent_path}/{attr}{predicate}")
}

// ── interval → range helpers ──────────────────────────────────────────────

fn json_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

/// A `WebTemplateRange` from a `C_REAL` interval (bounds + inclusivity ops).
fn real_range(iv: &Interval<f64>) -> Option<WebTemplateRange> {
    let (lo, hi, lo_inc, hi_inc, lo_unb, hi_unb) = interval_bounds(iv)?;
    let min = if lo_unb { None } else { lo };
    let max = if hi_unb { None } else { hi };
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op: min.map(|_| if lo_inc { ">=" } else { ">" }.to_owned()),
        min: min.map(json_f64),
        max_op: max.map(|_| if hi_inc { "<=" } else { "<" }.to_owned()),
        max: max.map(json_f64),
    })
}

/// A `WebTemplateRange` from a `C_INTEGER` interval (used for count/precision).
fn int_range(iv: &Interval<i32>) -> Option<WebTemplateRange> {
    let (lo, hi, lo_inc, hi_inc, lo_unb, hi_unb) = interval_bounds(iv)?;
    let min = if lo_unb { None } else { lo }.map(|v| if lo_inc { v } else { v + 1 });
    let max = if hi_unb { None } else { hi }.map(|v| if hi_inc { v } else { v - 1 });
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op: min.map(|_| ">=".to_owned()),
        min: min.map(serde_json::Value::from),
        max_op: max.map(|_| "<=".to_owned()),
        max: max.map(serde_json::Value::from),
    })
}

/// `(lower, upper, lower_included, upper_included, lower_unbounded,
/// upper_unbounded)` of an `Interval<T>` — both the point and proper forms.
#[expect(
    clippy::type_complexity,
    reason = "the six-element tuple IS the interval-bounds result named in the doc comment; a struct would only wrap it for one call site"
)]
fn interval_bounds<T: Clone>(
    iv: &Interval<T>,
) -> Option<(Option<T>, Option<T>, bool, bool, bool, bool)> {
    match iv {
        Interval::PointInterval(p) => Some((
            p.lower.clone(),
            p.lower.clone(),
            true,
            true,
            p.lower_unbounded,
            p.lower_unbounded,
        )),
        Interval::ProperInterval(ProperInterval::ProperInterval(p)) => Some((
            p.lower.clone(),
            p.upper.clone(),
            p.lower_included,
            p.upper_included,
            p.lower_unbounded,
            p.upper_unbounded,
        )),
        Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => None,
    }
}

fn point_i32(iv: &Interval<i32>) -> Option<i32> {
    match iv {
        Interval::PointInterval(p) => p.lower,
        Interval::ProperInterval(_) => None,
    }
}

fn point_f64(iv: &Interval<f64>) -> Option<f64> {
    match iv {
        Interval::PointInterval(p) => p.lower,
        Interval::ProperInterval(_) => None,
    }
}

/// The delimited-regex body of a `C_STRING` constraint entry (`/re/` or `^re^`),
/// or `None` for a literal value. A value that merely contains `/` (a unit like
/// `mmol/l`) is a literal, so both delimiters are required.
fn delimited_regex(s: &str) -> Option<&str> {
    if s.len() >= 2
        && ((s.starts_with('/') && s.ends_with('/')) || (s.starts_with('^') && s.ends_with('^')))
    {
        Some(s)
    } else {
        None
    }
}

// ── archetype id helpers ────────────────────────────────────────────────────

/// The `template_id` served for an `v2_4` OPT: the full HRID string
/// (`master07.05`).
fn template_id_of(h: &ArchetypeHrid) -> String {
    let base = format!(
        "{}-{}-{}.{}.v{}",
        h.rm_publisher, h.rm_package, h.rm_class, h.concept_id, h.release_version
    );
    match &h.namespace {
        Some(ns) => format!("{ns}::{base}"),
        None => base,
    }
}

/// The interface (major-version) archetype id used as a Web Template `nodeId` /
/// aqlPath predicate, from a full HRID (namespace dropped — an
/// `archetype_node_id` carries no namespace).
fn interface_id_of_hrid(h: &ArchetypeHrid) -> String {
    let major = h
        .release_version
        .split('.')
        .next()
        .unwrap_or(&h.release_version);
    format!(
        "{}-{}-{}.{}.v{}",
        h.rm_publisher, h.rm_package, h.rm_class, h.concept_id, major
    )
}

/// The interface (major-version) form of a full archetype-ref string
/// (`…​.vMAJOR.MINOR.PATCH[-status.build]` → `…​.vMAJOR`; namespace dropped).
fn interface_id(full: &str) -> String {
    let bare = full.split("::").last().unwrap_or(full);
    if let Some(vpos) = bare.rfind(".v")
        && let Some(head) = bare.get(..vpos + 2)
    {
        let major: String = bare
            .get(vpos + 2..)
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if major.is_empty() {
            bare.to_owned()
        } else {
            format!("{head}{major}")
        }
    } else {
        bare.to_owned()
    }
}
