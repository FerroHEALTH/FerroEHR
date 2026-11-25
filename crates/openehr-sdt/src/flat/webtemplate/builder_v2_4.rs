// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `v2_4` OPT2 → Web Template walk (the ADL2 front end).
//!
//! A Web Template is "a processed representation of an openEHR Operational
//! Template" (`simplified_formats master04-basic_concepts.adoc` §"Web Template
//! Metadata"), so [`build_web_template_v2_4`] builds the same [`WebTemplate`] as
//! [`super::builder`] and shares its `shape` and `id` passes. AOM2 has no
//! `C_DV_*` classes: a DV leaf is a `C_COMPLEX_OBJECT` with `C_ATTRIBUTE`s and
//! `C_ATTRIBUTE_TUPLE`s, and codes are `C_TERMINOLOGY_CODE`.
//!
//! NOTE: no openEHR spec governs the Web Template model: our own design; the
//! validation fields come from `AOM2/master03-archetype_package.adoc` §C_ATTRIBUTE.

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
use openehr_base::prelude::MultiplicityInterval;
use openehr_lang::v1_1::beom::core::operator_kind::OperatorKind;

use super::binding_v2_4::{bound_codes, split_operational, term_bindings_of};
use super::cardinality_v2_4::{all_cardinalities, cardinalities};
use super::constraint::{
    WebTemplateArchetypeSlot, WebTemplateClosedAttribute, WebTemplateExistence,
    WebTemplateStructuralStub,
};
use super::facts::WebTemplateNodeFacts;
use super::implicit;
use super::inputs::{assumed_code, default_string};
use super::interval_v2_4::{
    admits, int_range, integer_default, occ, occurrences, point_f64, point_i32, point_list,
    range_union, real_default, real_range, sole_interval,
};
use super::model::{
    WebTemplate, WebTemplateCodedValue, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
    WebTemplateValidation,
};
use super::path_v2_4::{build_path, name_constraint, object_path, prohibited_under};
use super::shape;

/// Build a [`WebTemplate`] from a parsed `v2_4` operational template (OPT2).
///
/// `version` is the format version string; `semVer` carries the OPT2 release
/// version. `prohibited_attributes` names the attributes the OPT removed for
/// `existence matches {0}`, so a COMPOSITION that deletes `context` gets no
/// `/context` node.
///
/// # Errors
/// [`crate::flat::error::FlatError::InvalidTemplate`] if the template lacks an
/// archetype id.
pub fn build_web_template_v2_4(
    opt: &OperationalTemplate,
    prohibited_attributes: &[String],
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
        overlay_parents: overlay_parents(opt),
        prohibited: prohibited_attributes,
    };

    // The definition root takes its rubrics from the OPT's own terminology;
    // constituents' terminologies switch in at each `C_ARCHETYPE_ROOT`.
    let root_co = CObject::CComplexObject(opt.definition.clone());
    let mut tree = build_node(
        &ctx,
        &opt.terminology,
        None,
        &root_co,
        None,
        shape::Identity::Archetyped,
        "",
    );
    // master04 §"Web Template Metadata": the root `nodeId` is the archetype id
    // (interface form), not the internal concept code.
    tree.node_id = Some(opt.archetype_id.local_semantic_id());
    tree.archetype_ref = Some(template_id.clone());
    tree.rm_release = Some(opt.rm_release.clone());
    tree.facts = WebTemplateNodeFacts::of_v2_4_root(&opt.terminology, &ctx.default_language);
    tree.min = Some(1);
    tree.max = 1;

    let tree = shape::finish(tree);
    super::constraint::check_evaluable(&tree)?;

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
        archetype_refs: Some(archetype_refs(&root_co, &ctx.overlay_parents)),
    })
}

/// Every archetype reference the OPT inlined at a `C_ARCHETYPE_ROOT`, mapped to
/// the full interface id the node carries (AM OPT2 `master03-opt_raw.adoc`
/// §Archetype References).
fn archetype_refs(
    co: &CObject,
    overlay_parents: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![co];
    while let Some(co) = stack.pop() {
        if let Some(reference) = object_archetype_ref(co, overlay_parents) {
            let interface = interface_id(reference);
            if reference != interface {
                out.insert(reference.to_owned(), interface);
            }
        }
        for attr in co_attributes(co) {
            stack.extend(attr.children.iter().flatten());
        }
    }
    out
}

/// The `other_meta_data` key naming each inlined template overlay's parent, as
/// `,`-separated `<overlay full id>=<parent full id>` pairs
/// (`openehr_adl::opt::OVERLAY_PARENTS_KEY`).
pub const OVERLAY_PARENTS_KEY: &str = "template_overlay_parents";

/// The specialisation parent of every template overlay the OPT inlined, keyed
/// by the overlay's full id.
///
/// A node under an overlay takes its parent's archetype id, so committed data
/// stays queryable by AQL against the published archetype (no specification
/// governs this: our own design).
fn overlay_parents(opt: &OperationalTemplate) -> BTreeMap<String, String> {
    opt.other_meta_data
        .get(OVERLAY_PARENTS_KEY)
        .map(|pairs| {
            pairs
                .split(',')
                .filter_map(|pair| pair.split_once('='))
                .map(|(overlay, parent)| (overlay.trim().to_owned(), parent.trim().to_owned()))
                .collect()
        })
        .unwrap_or_default()
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
    /// The specialisation parent of each inlined template overlay, keyed by
    /// the overlay's full id, as `openehr-adl` records it in the OPT's
    /// `other_meta_data`.
    overlay_parents: BTreeMap<String, String>,
    /// The OPT paths of the attributes the OPT removed for `existence {0}`.
    prohibited: &'a [String],
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

// ── node build ───────────────────────────────────────────────────────────────

fn build_node(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    owner: Option<&CAttribute>,
    co: &CObject,
    parent: Option<&WebTemplateNode>,
    identity: shape::Identity,
    path: &str,
) -> WebTemplateNode {
    // A `C_ARCHETYPE_ROOT` switches the scope for its children (OPT2 master03
    // §Terminology); its own rubric resolves in the outer scope first, since the
    // introducing artefact defines it (AOM2 master03 §Validity Rules).
    let child_term = match co {
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
            ctx.scope(&r.archetype_ref).unwrap_or(term)
        }
        _ => term,
    };
    let mut node = create_node(ctx, term, child_term, owner, co, parent, identity);
    node.facts = WebTemplateNodeFacts::of_v2_4(co, child_term, &ctx.default_language);
    if let Some(name) = attr_children(co, "name").next() {
        node.name_rm_type = Some(object_rm_type(name).to_owned());
        node.name_code_input = (object_rm_type(name) == "DV_CODED_TEXT")
            .then(|| coded_text_inputs(ctx, child_term, name))
            .and_then(|inputs| code_input(inputs).map(Box::new));
    }
    build_children(ctx, child_term, co, &mut node, path);
    node
}

fn create_node(
    ctx: &Ctx,
    name_term: &ArchetypeTerminology,
    parent_term: &ArchetypeTerminology,
    owner: Option<&CAttribute>,
    co: &CObject,
    parent: Option<&WebTemplateNode>,
    identity: shape::Identity,
) -> WebTemplateNode {
    let attr_name = owner.map(|a| a.rm_attribute_name.as_str());
    let archetyped = identity == shape::Identity::Archetyped;
    let rm_type = object_rm_type(co).to_owned();
    let arch_node_id = if archetyped {
        object_archetype_node_id(co, &ctx.overlay_parents)
    } else {
        String::new()
    };
    let (occ_min, max) = occurrences(object_occurrences(co));
    let min = meet_single_existence(occ_min, owner);

    let name = if archetyped {
        name_constraint(co)
    } else {
        None
    };
    let path = build_path(parent, attr_name, &rm_type, &arch_node_id, name.as_deref());
    let mut node = WebTemplateNode::new(rm_type, path);
    node.node_id = (!arch_node_id.is_empty()).then_some(arch_node_id);
    if archetyped {
        node.archetype_ref = object_archetype_ref(co, &ctx.overlay_parents).map(str::to_owned);
    }
    node.min = min;
    node.max = max;

    let code = if archetyped { object_node_id(co) } else { "" };
    if !code.is_empty() {
        // The introducing artefact's scope first (AOM2 master03 §Validity Rules).
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
/// transition node master05 §ISM_TRANSITION maps, taking the attribute's
/// occurrences. An unfilled slot or a residual proxy yields no node.
fn build_attribute_children(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    attr: &CAttribute,
    node: &WebTemplateNode,
    children: &mut Vec<WebTemplateNode>,
    path: &str,
) {
    let attr_path = format!("{path}/{}", attr.rm_attribute_name);
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
            Some(node),
            identity,
            &object_path(&attr_path, object_node_id(child_co)),
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
    path: &str,
) {
    let is_data_value = node.rm_type.starts_with("DV_");
    let recurse_attrs = !is_data_value || node.rm_type.starts_with("DV_INTERVAL");

    let mut children = Vec::new();
    if recurse_attrs {
        for attr in co_attributes(co) {
            // The name attribute names the node (master04 §"Field Identifiers").
            if attr.rm_attribute_name != "name" {
                build_attribute_children(ctx, term, attr, node, &mut children, path);
            }
        }
        let declared = children.len();
        for attr in implicit::interval_parts_v2_4(&node.rm_type, co_attributes(co)) {
            build_attribute_children(ctx, term, &attr, node, &mut children, path);
        }
        children
            .iter_mut()
            .skip(declared)
            .for_each(|c| c.implicit = true);
    }

    // A party node keeps its inputs with children: master05's PARTY_PROXY tables
    // put the suffixes and the `/relationship` sub-path on the same node.
    if has_inputs(&node.rm_type) && (children.is_empty() || is_party(&node.rm_type)) {
        let (built, ptypes) = build_inputs(ctx, term, &node.rm_type, co);
        node.inputs = built;
        node.proportion_types = ptypes;
    }

    node.cardinalities = cardinalities(co, &node.aql_path);
    node.card_all = all_cardinalities(co, &node.aql_path);
    if !node.inputs.is_empty() {
        capture_leaf_constraints(co, node);
    }
    // Existence / closed-attribute / structural-stub capture is meaningful only
    // for attribute-recursing (structural) nodes; a DV leaf's constraints are
    // handled by `inputs` (mirrors the OPT-1.4 front end's `recurse_attrs` guard).
    if recurse_attrs {
        node.existence = existence_constraints(co, &node.aql_path);
        node.prohibited_attributes = prohibited_under(ctx.prohibited, path);
        node.closed_attributes = closed_attributes(co, &node.aql_path, &ctx.overlay_parents);
        node.structural_stubs = structural_stubs(ctx, term, co, &node.aql_path);
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

/// Whether a node of this RM type is a web-template leaf carrying `inputs`.
///
/// All three `PARTY_PROXY` subtypes are leaves (master05 §§PARTY_SELF,
/// PARTY_IDENTIFIED, PARTY_RELATED); `relationship` is a sub-path.
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
        input.default_value = cs
            .assumed_value
            .as_deref()
            .filter(|v| input.list.is_empty() || input.list.iter().any(|cv| cv.value == *v))
            .map(default_string);
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

/// The `code` input among a coded constraint's inputs.
fn code_input(inputs: Vec<WebTemplateInput>) -> Option<WebTemplateInput> {
    inputs
        .into_iter()
        .find(|i| i.suffix.as_deref() == Some("code"))
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
    if let Some(qualified) = Adl14Qualified::parse(&ctc.constraint) {
        return qualified.inputs(ctx, term);
    }
    let (constraint, operational) = split_operational(&ctc.constraint);
    let codes = expand_codes(term, constraint);
    let assumed = ctc.assumed_value.as_ref().map(|tc| tc.code_string.as_str());
    if let Some((terminology, bound)) = bound_codes(term, operational, &codes) {
        return vec![bound_input(ctx, term, terminology, &codes, &bound, assumed)];
    }
    if codes.is_empty() {
        let mut inputs = external_coded_inputs();
        if let Some(code) = inputs.first_mut() {
            code.default_value = assumed_code(assumed, &codes);
        }
        return inputs;
    }
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = codes
        .iter()
        .map(|code| coded_value(ctx, term, code))
        .collect();
    // An ac-code's assumed value is one at-code of its value set
    // (AM `UML/classes/org.openehr.am.aom2.c_terminology_code.adoc`).
    input.default_value = assumed_code(assumed, &codes);
    // Local (archetype-defined) value sets have terminology `local` — left
    // implicit (the reference form omits `terminology: "local"`).
    vec![input]
}

/// The `code` input of a constraint whose at-codes the template binds to
/// `terminology` ([`super::binding_v2_4`]).
fn bound_input(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    terminology: &str,
    codes: &[String],
    bound: &[String],
    assumed: Option<&str>,
) -> WebTemplateInput {
    let openehr = terminology.eq_ignore_ascii_case("openehr");
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = codes
        .iter()
        .zip(bound)
        .map(|(local, code)| {
            let mut cv = coded_value(ctx, term, local);
            cv.value.clone_from(code);
            if openehr {
                let rubrics = openehr_term::bundle::openehr();
                for (lang, label) in &mut cv.localized_labels {
                    if let Some(rubric) = rubrics.concept_rubric(code, lang) {
                        rubric.clone_into(label);
                    }
                }
                if let Some(rubric) = rubrics.concept_rubric(code, &ctx.default_language) {
                    cv.label = Some(rubric.to_owned());
                }
            }
            cv
        })
        .collect();
    input.terminology = Some(terminology.to_owned());
    input.default_value = assumed
        .and_then(|a| codes.iter().position(|code| code == a))
        .and_then(|i| bound.get(i))
        .map(default_string);
    input
}

/// An ADL 1.4 qualified term constraint kept verbatim in
/// `C_TERMINOLOGY_CODE.constraint`: `terminology::code[,code]*[;assumed]` (ADL
/// 1.4 `master09` §Custom Syntax).
///
/// NOTE: no openEHR specification governs reading it here: our own design, for
/// an ADL 2 template over unconverted at-coded archetypes.
#[derive(Debug, PartialEq, Eq)]
struct Adl14Qualified<'a> {
    terminology: &'a str,
    codes: Vec<&'a str>,
    assumed: Option<&'a str>,
}

impl<'a> Adl14Qualified<'a> {
    /// The parts of a verbatim qualified constraint, `None` for an ADL 2
    /// constraint (an at-code or an ac-code).
    fn parse(constraint: &'a str) -> Option<Self> {
        let (terminology, rest) = constraint.trim().split_once("::")?;
        let (list, assumed) = match rest.split_once(';') {
            Some((list, assumed)) => (list, Some(assumed.trim())),
            None => (rest, None),
        };
        let codes = list
            .split(',')
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .collect();
        Some(Self {
            terminology: terminology.trim(),
            codes,
            assumed: assumed.filter(|code| !code.is_empty()),
        })
    }

    /// The `code` input of the constraint, as the 1.4 builder writes it for a
    /// `C_CODE_PHRASE`.
    fn inputs(&self, ctx: &Ctx, term: &ArchetypeTerminology) -> Vec<WebTemplateInput> {
        let local = self.terminology.eq_ignore_ascii_case("local");
        let named = (!local).then(|| self.terminology.to_owned());
        if self.codes.is_empty() {
            let mut inputs = external_coded_inputs();
            for input in &mut inputs {
                input.terminology.clone_from(&named);
            }
            return inputs;
        }
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
        input.list = self
            .codes
            .iter()
            .map(|code| {
                if local {
                    coded_value(ctx, term, code)
                } else {
                    let label = self
                        .terminology
                        .eq_ignore_ascii_case("openehr")
                        .then(|| openehr_term::bundle::openehr().concept_rubric(code, "en"))
                        .flatten()
                        .map_or_else(|| (*code).to_owned(), str::to_owned);
                    WebTemplateCodedValue::new(*code, Some(label))
                }
            })
            .collect();
        input.terminology = named;
        input.default_value = self
            .assumed
            .filter(|code| self.codes.contains(code))
            .map(|code| default_string(&code));
        vec![input]
    }
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
    let mut cv = WebTemplateCodedValue::new(code, Some(label)).with_terminology("local");
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
/// co-varying members carry, `None` when `units` constrains no plain string.
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
                validation.range = sole_interval(cr.constraint.as_ref()).and_then(real_range);
            }
            ("precision", Some(CPrimitiveObject::CInteger(ci))) => {
                validation.precision = sole_interval(ci.constraint.as_ref()).and_then(int_range);
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
        && let Some(range) = sole_interval(cr.constraint.as_ref()).and_then(real_range)
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
    magnitude.default_value = creal_under(co, "magnitude").and_then(real_default);
    units.default_value = cstrings_under(co, "units")
        .find_map(|cs| cs.assumed_value.as_deref())
        .filter(|unit| units.list.is_empty() || units.list.iter().any(|cv| cv.value == *unit))
        .map(default_string);
    vec![magnitude, units]
}

fn count_input(co: &CObject) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Integer, None);
    if let Some(ci) = cinteger_under(co, "magnitude") {
        if let Some(range) = sole_interval(ci.constraint.as_ref()).and_then(int_range) {
            input.validation = Some(WebTemplateValidation {
                range: Some(range),
                ..Default::default()
            });
        }
        input.default_value = integer_default(ci);
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
        // The symbol member's assumed at-code, when it is one of the rows'.
        let listed: Vec<String> = input.list.iter().map(|cv| cv.value.clone()).collect();
        let assumed = tuple.members.iter().flatten().find_map(|member| {
            (member.rm_attribute_name == "symbol")
                .then(|| terminology_code_under_attribute(member))
                .flatten()
                .and_then(|ctc| ctc.assumed_value.as_ref())
                .map(|tc| tc.code_string.clone())
        });
        if input.default_value.is_none() {
            input.default_value = assumed_code(assumed.as_deref(), &listed);
        }
    }
    input
}

/// The `C_TERMINOLOGY_CODE` a tuple member attribute constrains, if any.
fn terminology_code_under_attribute(attribute: &CAttribute) -> Option<&CTerminologyCode> {
    attribute.children.iter().flatten().find_map(|c| match c {
        CObject::CTerminologyCode(ctc) => Some(ctc),
        _ => None,
    })
}

/// One tuple row read into its symbol code and the paired `DV_ORDINAL` integer
/// and `DV_SCALE` real values, [`None`] without a symbol code.
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
                // An ordinal of an archetype read in the ADL 1.4 dialect keeps its
                // symbol as `local::atNNNN` (`Adl14Qualified`).
                code = match Adl14Qualified::parse(&ctc.constraint) {
                    Some(qualified) => qualified.codes.first().map(|&code| code.to_owned()),
                    None => expand_codes(term, &ctc.constraint).into_iter().next(),
                };
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
        input.default_value = cb
            .assumed_value
            .filter(|v| {
                cb.constraint
                    .as_ref()
                    .is_none_or(|c| c.is_empty() || c.contains(v))
            })
            .map(|v| default_string(&v));
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
    input.default_value = temporal_default(co);
    input
}

/// The `defaultValue` of the date, time, date-time or duration constraint on
/// `value`: its assumed value when its interval constraint admits it.
fn temporal_default(co: &CObject) -> Option<serde_json::Value> {
    attr_children(co, "value").find_map(|c| match c {
        CObject::CDate(cd) => cd
            .assumed_value
            .as_ref()
            .filter(|v| admits(cd.constraint.as_ref(), *v))
            .map(|v| default_string(&v.value)),
        CObject::CDateTime(cdt) => cdt
            .assumed_value
            .as_ref()
            .filter(|v| admits(cdt.constraint.as_ref(), *v))
            .map(|v| default_string(&v.value)),
        CObject::CTime(ct) => ct
            .assumed_value
            .as_ref()
            .filter(|v| admits(ct.constraint.as_ref(), *v))
            .map(|v| default_string(&v.value)),
        CObject::CDuration(cd) => cd
            .assumed_value
            .as_ref()
            .filter(|v| admits(cd.constraint.as_ref(), *v))
            .map(|v| default_string(&v.value)),
        _ => None,
    })
}

/// The `DV_DURATION` per-field inputs, gated by the `C_DURATION` pattern (a `P…`
/// string listing the allowed fields). No openEHR spec governs the per-field
/// split — our own design/extension, matching the OPT-1.4 front end.
fn duration_input(co: &CObject) -> WebTemplateInput {
    // A single Duration input; the OPT-1.4 field split is not reconstructed.
    let mut input = WebTemplateInput::new(WebTemplateInputType::Duration, None);
    if let Some(pattern) = cduration_pattern(co) {
        input.validation = Some(WebTemplateValidation {
            pattern: Some(pattern),
            ..Default::default()
        });
    }
    input.default_value = temporal_default(co);
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
            if let Some(cr) = creal_under(co, suffix) {
                if let Some(range) = sole_interval(cr.constraint.as_ref()).and_then(real_range) {
                    input.validation = Some(WebTemplateValidation {
                        range: Some(range),
                        ..Default::default()
                    });
                }
                input.default_value = real_default(cr);
            }
            input
        })
        .collect()
}

// ── archetype-conformance constraint capture (validation-only fields) ─────────
//
// AOM2 `master03-archetype_package.adoc` §C_ATTRIBUTE, §ARCHETYPE_SLOT.

/// Capture the explicitly mandated AOM2 `C_ATTRIBUTE.existence` constraints,
/// keyed by absolute archetype path (AOM2 §C_ATTRIBUTE: existence is "Only set
/// if it overrides the underlying reference model"). `name` is excluded.
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
        // Existence and cardinality are orthogonal (AOM2 §C_ATTRIBUTE); a primitive
        // value constraint is never a navigable instance attribute.
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

/// Whether a child `C_OBJECT` is an object-valued (navigable) constraint rather
/// than a primitive value constraint.
fn is_object_valued(co: &CObject) -> bool {
    matches!(
        co,
        CObject::CComplexObject(_) | CObject::CComplexObjectProxy(_) | CObject::ArchetypeSlot(_)
    )
}

/// Capture the closed-archetype constraints, keyed by absolute archetype path.
///
/// An attribute with an unresolved `C_COMPLEX_OBJECT_PROXY` stays open, a
/// primitive or unconstrained one is never closed, and `name` is matched by
/// predicate (master04 §"Field Identifiers").
fn closed_attributes(
    co: &CObject,
    node_path: &str,
    overlay_parents: &BTreeMap<String, String>,
) -> Vec<WebTemplateClosedAttribute> {
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
        let (allowed, slots) = admissible_children(attr, overlay_parents);
        if allowed.is_empty() && slots.is_empty() {
            continue; // Open attribute (no node-id alternatives, no slot).
        }
        let (allowed_ids, allowed_rm_types) = allowed.into_iter().unzip();
        out.push(WebTemplateClosedAttribute {
            path: format!("{node_path}/{}", attr.rm_attribute_name),
            allowed_ids,
            allowed_rm_types,
            slots,
        });
    }
    out
}

/// The admissible child identities of one attribute: its node-identified
/// LOCATABLE alternatives with their RM types, and its archetype slots.
fn admissible_children(
    attr: &CAttribute,
    overlay_parents: &BTreeMap<String, String>,
) -> (Vec<(String, String)>, Vec<WebTemplateArchetypeSlot>) {
    let mut allowed: Vec<(String, String)> = Vec::new();
    let mut slots: Vec<WebTemplateArchetypeSlot> = Vec::new();
    for child in attr.children.iter().flatten() {
        match child {
            CObject::ArchetypeSlot(s) => slots.push(archetype_slot(s)),
            CObject::CComplexObject(_) => {
                let id = object_archetype_node_id(child, overlay_parents);
                if !id.is_empty() {
                    allowed.push((id, object_rm_type(child).to_owned()));
                }
            }
            _ => {}
        }
    }
    (allowed, slots)
}

/// The validation-only slot record from an AOM2 `ARCHETYPE_SLOT`, with the
/// archetype-id regexes of its include and exclude assertions (AOM2
/// §ARCHETYPE_SLOT).
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

/// The archetype-id regex of a slot `ASSERTION`, read from its expression tree
/// (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package).
///
/// The core expression is `<reference> matches {/<regex>/}` (`ADL2/master04.3`
/// §Slots based on Lexical Archetype Identifiers); any other shape yields `None`.
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
/// (`AOM2/master04.5` §`C_STRING`; `ADL2/master04.5` §Regular Expression).
fn delimited_regex_body(entry: &str) -> Option<&str> {
    let trimmed = entry.trim();
    ['/', '^'].into_iter().find_map(|delimiter| {
        trimmed
            .strip_prefix(delimiter)
            .and_then(|rest| rest.strip_suffix(delimiter))
    })
}

/// The structural attributes the FLAT builder and the example generator
/// synthesise when empty (RM `composition`, RM `data_structures`).
const STRUCTURAL_ATTRS: [&str; 4] = ["data", "description", "protocol", "state"];

/// Captures the structural stubs of a structural node.
///
/// The compactor drops an empty wrapper, so the stub keeps the constrained
/// identity the builder fills the attribute with (AOM2 §C_ATTRIBUTE).
fn structural_stubs(
    ctx: &Ctx,
    term: &ArchetypeTerminology,
    co: &CObject,
    node_path: &str,
) -> Vec<WebTemplateStructuralStub> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        if !STRUCTURAL_ATTRS.contains(&attr.rm_attribute_name.as_str()) {
            continue;
        }
        for child in attr.children.iter().flatten() {
            // Only a node-identified structural child gives a concrete identity to
            // stamp; a slot / proxy leaves the attribute to its placeholder.
            let CObject::CComplexObject(_) = child else {
                continue;
            };
            let node_id = object_archetype_node_id(child, &ctx.overlay_parents);
            if node_id.is_empty() {
                continue;
            }
            out.push(WebTemplateStructuralStub {
                attr: attr.rm_attribute_name.clone(),
                path: format!("{node_path}/{}", attr.rm_attribute_name),
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

pub(super) fn co_attributes(co: &CObject) -> &[CAttribute] {
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

pub(super) fn attr_children<'a>(co: &'a CObject, name: &str) -> impl Iterator<Item = &'a CObject> {
    co_attributes(co)
        .iter()
        .filter(move |a| a.rm_attribute_name == name)
        .flat_map(|a| a.children.iter().flatten())
}

pub(super) fn cstring_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CString> {
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
/// archetype root, the specialisation parent's for a template overlay (see
/// [`overlay_parents`]), else the constraint node id (at/id-code).
fn object_archetype_node_id(co: &CObject, overlay_parents: &BTreeMap<String, String>) -> String {
    object_archetype_ref(co, overlay_parents)
        .map_or_else(|| object_node_id(co).to_owned(), interface_id)
}

/// The complete identifier an inlined archetype root carries in data: the
/// template's reference, or its specialisation parent's for a template overlay
/// (see [`overlay_parents`]); `None` below a root.
fn object_archetype_ref<'a>(
    co: &'a CObject,
    overlay_parents: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    match co {
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => Some(
            overlay_parents
                .get(&r.archetype_ref)
                .unwrap_or(&r.archetype_ref)
                .as_str(),
        ),
        _ => None,
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

/// The node's effective `min`: its occurrences lower bound met with the owning
/// single attribute's existence lower bound.
///
/// ADL 2 `master04.3-cadl_complex_types.adoc` §Occurrences: a single-valued
/// attribute's value occurrences "is already defined by the attribute's
/// `existence`". An unset existence leaves occurrences untouched (AOM2
/// `master04.2-constraint_model-semantics.adoc` §"Attribute Nodes"), and a
/// container attribute is left alone.
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

/// Capture the numeric leaf constraints the `inputs` do not carry (AOM2
/// `master04.5-constraint_model-class_definitions.adoc` §"Conformance semantics:
/// C_ORDERED").
fn capture_leaf_constraints(co: &CObject, node: &mut WebTemplateNode) {
    for datum in ["magnitude", "value", "numerator", "denominator", "size"] {
        let ranges = if let Some(ci) = cinteger_under(co, datum) {
            if let Some(list) = point_list(ci.constraint.as_ref()) {
                node.numeric_lists
                    .push((datum.to_owned(), list.into_iter().map(f64::from).collect()));
            }
            // NOTE: ITS-REST simplified_formats master04 §"Web Template Metadata" gives an input
            // one `validation.range`, so a union of ranges is carried validation-only.
            let sole = sole_interval(ci.constraint.as_ref()).and_then(int_range);
            range_union(ci.constraint.as_ref(), int_range)
                .or_else(|| sole.filter(|_| datum == "size").map(|range| vec![range]))
        } else if let Some(cr) = creal_under(co, datum) {
            if let Some(list) = point_list(cr.constraint.as_ref()) {
                node.numeric_lists.push((datum.to_owned(), list));
            }
            range_union(cr.constraint.as_ref(), real_range)
        } else {
            None
        };
        let ranges = ranges.into_iter().flatten();
        node.numeric_ranges
            .extend(ranges.map(|range| (datum.to_owned(), range)));
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

/// The interface form a Web Template `nodeId` and aqlPath predicate carry for
/// the archetype root a full archetype-ref string names
/// ([`ArchetypeHrid::local_semantic_id`]).
fn interface_id(full: &str) -> String {
    match full.parse::<ArchetypeHrid>() {
        Ok(hrid) => hrid.local_semantic_id(),
        // NOTE: an OPT2 reference the HRID grammar refuses (ADL2 `master07.05-adl_identification.adoc`)
        // is no archetype id, so the node keeps the identifier as the template wrote it.
        Err(_) => full.to_owned(),
    }
}
