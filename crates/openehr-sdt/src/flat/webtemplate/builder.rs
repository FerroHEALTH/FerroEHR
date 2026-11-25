// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT 1.4 → Web Template walk.
//!
//! ITS-REST simplified_formats master04 §"Web Template Metadata": build a node
//! per constraint object, compact per §"Level Removal", then assign ids per
//! §"Node ID Generation Rules".
//!
//! The web template mirrors the OPT's constraint tree, so RM-mandatory
//! attributes the OPT leaves unconstrained are not injected, and an internal
//! reference (`use_node`) is emitted as its own node. No openEHR spec governs
//! the shape of an unconstrained ELEMENT value: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::HashMap;

use indexmap::IndexMap;
use openehr_its::opt14::types::{
    ArchetypeTerm, Assertion, CArchetypeRoot, CObject, CPrimitive, Cardinality,
    Constraintbindingset, ExprItem, Intervalofinteger, OperationalTemplate, OperatorKind,
    TermBindingItem, Termbindingset, ValidityKind,
};

use super::constraint::{
    WebTemplateArchetypeSlot, WebTemplateClosedAttribute, WebTemplateCodeList,
    WebTemplateConstraintBinding, WebTemplateExistence, WebTemplateStructuralStub,
};
use super::existence::{existence_constraints, prohibited_attributes};
use super::implicit::{self, attribute_existence};
use super::inputs::{self, Labels};
use super::model::{
    CodedName, WebTemplate, WebTemplateBindingCodedValue, WebTemplateCardinality, WebTemplateNode,
};
use super::shape;

/// A rubric (`text`/`description`) for a code, resolved from an archetype ontology.
#[derive(Default)]
struct Rubric {
    text: Option<String>,
    description: Option<String>,
}

/// Ontology: `archetype_id → language → code → rubric`.
type Ontology = HashMap<String, HashMap<String, HashMap<String, Rubric>>>;

/// External term bindings per archetype, keyed by `archetype_id`: the archetype
/// root's `term_bindings`. A node inherits its owning archetype root's bindings.
type TermBindings = HashMap<String, Vec<Termbindingset>>;

/// External **constraint** bindings per archetype, keyed by `archetype_id`: the
/// flat ontology's `constraint_bindings` (ac-code → terminology-query URI,
/// `AM/docs/ADL1.4/master08-adl.adoc` §Constraint_bindings).
type ConstraintBindings = HashMap<String, Vec<Constraintbindingset>>;

/// The ontology scope of one `C_ARCHETYPE_ROOT` instance in the walk.
///
/// Each fill of an archetype is an overlay with its own terminology (AOM 2
/// `master10-templates.adoc` §Overview), so inline `term_definitions` belong to
/// the root instance; the bindings stay keyed by archetype id.
#[derive(Clone, Copy)]
struct Scope<'a> {
    archetype_id: &'a str,
    inline: &'a HashMap<String, Rubric>,
}

/// Build context shared across the walk.
struct Ctx {
    default_language: String,
    languages: Vec<String>,
    ontology: Ontology,
    term_bindings: TermBindings,
    constraint_bindings: ConstraintBindings,
}

impl Ctx {
    /// The rubric of `code` in `lang`: the flat ontology's entry for the
    /// scope's archetype, else, in the default language, the scope's own inline
    /// `term_definitions`.
    fn rubric<'s>(&'s self, scope: Scope<'s>, lang: &str, code: &str) -> Option<&'s Rubric> {
        self.ontology
            .get(scope.archetype_id)
            .and_then(|langs| langs.get(lang))
            .and_then(|codes| codes.get(code))
            .or_else(|| {
                (lang == self.default_language)
                    .then(|| scope.inline.get(code))
                    .flatten()
            })
    }

    fn text(&self, scope: Scope<'_>, code: &str, lang: &str) -> Option<String> {
        self.rubric(scope, lang, code).and_then(|r| r.text.clone())
    }

    /// The external term bindings for `code` within the scope's archetype, keyed by
    /// terminology, first match per terminology (master04 §"Web Template Metadata").
    fn term_bindings_for(
        &self,
        scope: Scope<'_>,
        code: &str,
    ) -> IndexMap<String, WebTemplateBindingCodedValue> {
        let mut out = IndexMap::new();
        if code.is_empty() {
            return out;
        }
        let Some(sets) = self.term_bindings.get(scope.archetype_id) else {
            return out;
        };
        for set in sets {
            if out.contains_key(&set.terminology) {
                continue;
            }
            if let Some(item) = set.items.iter().find(|it| it.code == code)
                && let Some(bcv) = binding_coded_value(item)
            {
                out.insert(set.terminology.clone(), bcv);
            }
        }
        out
    }

    /// The external constraint bindings of `ac_code` within the scope's archetype,
    /// one per terminology (`AM/docs/ADL1.4/master08-adl.adoc` §Constraint_bindings).
    /// An empty query URI is dropped.
    fn constraint_bindings_for(
        &self,
        scope: Scope<'_>,
        ac_code: &str,
        attr: &str,
    ) -> Vec<WebTemplateConstraintBinding> {
        let mut out = Vec::new();
        if ac_code.is_empty() {
            return out;
        }
        let Some(sets) = self.constraint_bindings.get(scope.archetype_id) else {
            return out;
        };
        for set in sets {
            for item in set.items.iter().filter(|it| it.code == ac_code) {
                let query_uri = item.value.trim();
                if query_uri.is_empty() {
                    continue;
                }
                out.push(WebTemplateConstraintBinding {
                    attr: attr.to_owned(),
                    ac_code: ac_code.to_owned(),
                    terminology: set.terminology.clone(),
                    query_uri: query_uri.to_owned(),
                });
            }
        }
        out
    }
}

/// Every `CONSTRAINT_REF` in force on `co`, on the node itself or a coded
/// attribute, resolved against the ontology's `constraint_bindings` (AOM 1.4
/// `master04-constraint_model_package.adoc` §Reference Objects).
fn constraint_bindings_of(
    ctx: &Ctx,
    co: &CObject,
    scope: Scope<'_>,
) -> Vec<WebTemplateConstraintBinding> {
    let mut out = Vec::new();
    if let CObject::ConstraintRef(cr) = co {
        out.extend(ctx.constraint_bindings_for(scope, &cr.reference, ""));
    }
    for attr in inputs::attributes(co) {
        let attr_name = inputs::attribute_name(attr);
        for child in inputs::attribute_children(attr) {
            if let CObject::ConstraintRef(cr) = child {
                out.extend(ctx.constraint_bindings_for(scope, &cr.reference, attr_name));
            }
        }
    }
    out
}

/// The bound code phrase as a `termBindings` value, `None` when its code is
/// blank.
fn binding_coded_value(item: &TermBindingItem) -> Option<WebTemplateBindingCodedValue> {
    let code_phrase = &item.value;
    if code_phrase.code_string.trim().is_empty() {
        return None;
    }
    let terminology_id = if code_phrase.terminology_id.value.is_empty() {
        item.code.clone()
    } else {
        code_phrase.terminology_id.value.clone()
    };
    Some(WebTemplateBindingCodedValue {
        value: code_phrase.code_string.clone(),
        terminology_id,
    })
}

/// A [`Labels`] view bound to the current archetype, for coded-value rubrics.
struct ArchetypeLabels<'a> {
    ctx: &'a Ctx,
    scope: Scope<'a>,
}

impl Labels for ArchetypeLabels<'_> {
    fn text(&self, terminology: &str, code: &str) -> Option<String> {
        // An external code's rubric is keyed `TERMINOLOGY::code` in the OPT ontology.
        self.ctx
            .text(self.scope, code, &self.ctx.default_language)
            .or_else(|| {
                self.ctx.text(
                    self.scope,
                    &format!("{terminology}::{code}"),
                    &self.ctx.default_language,
                )
            })
    }

    fn localized(&self, terminology: &str, code: &str) -> IndexMap<String, String> {
        let mut out = IndexMap::new();
        for lang in &self.ctx.languages {
            if let Some(t) = self.ctx.text(self.scope, code, lang).or_else(|| {
                self.ctx
                    .text(self.scope, &format!("{terminology}::{code}"), lang)
            }) {
                out.insert(lang.clone(), t);
            }
        }
        out
    }

    fn term_bindings(&self, code: &str) -> IndexMap<String, WebTemplateBindingCodedValue> {
        self.ctx.term_bindings_for(self.scope, code)
    }
}

/// Build a [`WebTemplate`] from a parsed OPT 1.4 operational template
/// (`simplified_formats master04-basic_concepts.adoc`). `version` is the format
/// version string.
///
/// # Errors
/// [`crate::flat::error::FlatError::InvalidTemplate`] if the template lacks a template id.
pub fn build_web_template(
    opt: &OperationalTemplate,
) -> Result<WebTemplate, crate::flat::error::FlatError> {
    let template_id = opt.template_id.value.clone();
    if template_id.is_empty() {
        return Err(crate::flat::error::FlatError::InvalidTemplate(
            "template_id is mandatory".to_owned(),
        ));
    }
    let default_language = opt.language.code_string.clone();
    let ctx = Ctx {
        languages: collect_languages(opt, &default_language),
        ontology: collect_ontology(opt),
        term_bindings: collect_term_bindings(opt),
        constraint_bindings: collect_constraint_bindings(opt),
        default_language,
    };

    let root_co = CObject::CArchetypeRoot(opt.definition.clone());
    let no_inline = HashMap::new();
    let outer = Scope {
        archetype_id: opt.definition.archetype_id.value.as_str(),
        inline: &no_inline,
    };
    let mut tree = build_node(
        &ctx,
        None,
        &root_co,
        "",
        outer,
        None,
        shape::Identity::Archetyped,
    );
    tree = shape::finish(tree);
    super::constraint::check_evaluable(&tree)?;

    Ok(WebTemplate {
        template_id,
        // NOTE: OPT 1.4 has no semantic-version field (semVer is an ADL2/OPT2
        // concept), so the 1.4 adapter always emits `null` — matching what stock
        // tooling produces for a 1.4 template. A value would only appear for OPT 2.
        sem_ver: None,
        version: shape::CURRENT_VERSION.to_owned(),
        default_language: ctx.default_language.clone(),
        languages: ctx.languages.clone(),
        tree,
        other_details: other_details(opt),
        archetype_refs: None,
    })
}

// ── node build ───────────────────────────────────────────────────────────────

fn build_node(
    ctx: &Ctx,
    owner: Option<&openehr_its::opt14::types::CAttribute>,
    co: &CObject,
    parent_path: &str,
    parent_scope: Scope<'_>,
    group: Option<&str>,
    identity: shape::Identity,
) -> WebTemplateNode {
    // A C_ARCHETYPE_ROOT switches the ontology scope to its own instance.
    let inline;
    let scope = match co {
        CObject::CArchetypeRoot(r) => {
            inline = inline_rubrics(r);
            Scope {
                archetype_id: r.archetype_id.value.as_str(),
                inline: &inline,
            }
        }
        _ => parent_scope,
    };
    let mut node = create_node(ctx, owner, co, parent_path, scope, identity);
    build_children(ctx, co, &mut node, scope, group);
    node
}

fn create_node(
    ctx: &Ctx,
    owner: Option<&openehr_its::opt14::types::CAttribute>,
    co: &CObject,
    parent_path: &str,
    scope: Scope<'_>,
    identity: shape::Identity,
) -> WebTemplateNode {
    let attr_name = owner.map(inputs::attribute_name);
    let archetyped = identity == shape::Identity::Archetyped;
    let rm_type = object_rm_type(co).to_owned();
    let arch_node_id = if archetyped {
        object_archetype_node_id(co)
    } else {
        String::new()
    };
    let (occ_min, occ_max) = occurrences(object_occurrences(co));
    let min = meet_single_existence(occ_min, owner);
    let max = cap_single_upper(occ_max, owner);
    let name_constraint = if archetyped {
        name_constraint(co)
    } else {
        None
    };

    let path = build_path(
        parent_path,
        attr_name,
        &arch_node_id,
        name_constraint.as_deref(),
    );
    let mut node = WebTemplateNode::new(rm_type, path);
    node.node_id = (!arch_node_id.is_empty()).then_some(arch_node_id);
    node.facts = super::facts::WebTemplateNodeFacts::of_v1_4(co);
    node.min = min;
    node.max = max;

    let name_code = if archetyped { object_node_id(co) } else { "" };
    // A `DV_CODED_TEXT` name constraint (RM common master03 §LOCATABLE) fixes the
    // display value and the `defining_code`, and wins over a plain name.
    let coded_name = archetyped
        .then(|| coded_name_constraint(co, ctx, scope, name_constraint.as_deref()))
        .flatten();
    let name_child = inputs::attr_children(co, "name").next();
    node.name_rm_type = name_child.map(|c| object_rm_type(c).to_owned());
    node.name_code_input = name_child
        .filter(|c| object_rm_type(c) == "DV_CODED_TEXT")
        .and_then(|c| {
            let labels = ArchetypeLabels { ctx, scope };
            inputs::build_inputs("DV_CODED_TEXT", c, &labels, None)
                .0
                .into_iter()
                .find(|i| i.suffix.as_deref() == Some("code"))
        })
        .map(Box::new);
    apply_node_naming(
        &mut node,
        ctx,
        scope,
        name_code,
        name_constraint.as_deref(),
        coded_name,
    );
    // Node-level external term bindings: the archetype root's `term_bindings`
    // whose item code matches this node's constraint node id (master04
    // §"Web Template Metadata": the node-level `termBindings` map).
    node.term_bindings = ctx.term_bindings_for(scope, name_code);
    // Constraint bindings need a terminology service, so they are recorded here
    // (BASE `master12-terminology.adoc` §"Binding Terminology Value-sets to
    // Archetypes").
    node.constraint_bindings = constraint_bindings_of(ctx, co, scope);
    node
}

/// Fills a node's name, localized names/descriptions and name code.
///
/// A coded name constraint wins over an explicit plain `name/value`, which wins
/// over the node rubric; only the rubric route carries per-language text.
fn apply_node_naming(
    node: &mut WebTemplateNode,
    ctx: &Ctx,
    scope: Scope<'_>,
    name_code: &str,
    name_constraint: Option<&str>,
    coded_name: Option<(String, CodedName)>,
) {
    if let Some((value, coded)) = coded_name {
        node.name = Some(value.clone());
        node.localized_name = Some(value);
        node.name_coded = Some(coded);
    } else if let Some(nc) = name_constraint {
        node.name = Some(nc.to_owned());
        node.localized_name = Some(nc.to_owned());
    } else if !name_code.is_empty() {
        node.name = ctx.text(scope, name_code, &ctx.default_language);
        node.localized_name.clone_from(&node.name);
        apply_localized_rubrics(node, ctx, scope, name_code);
    }
    node.name_code = if name_code.is_empty() {
        None
    } else {
        Some(name_code.to_owned())
    };
}

/// Records the node rubric's per-language text and description.
fn apply_localized_rubrics(
    node: &mut WebTemplateNode,
    ctx: &Ctx,
    scope: Scope<'_>,
    name_code: &str,
) {
    for lang in &ctx.languages {
        let Some(r) = ctx.rubric(scope, lang, name_code) else {
            continue;
        };
        if let Some(t) = &r.text {
            node.localized_names.insert(lang.clone(), t.clone());
        }
        if let Some(d) = &r.description {
            node.localized_descriptions.insert(lang.clone(), d.clone());
        }
    }
}

/// Builds the child nodes of one constraint attribute into `children`.
///
/// The careflow-state alternatives of `ism_transition` collapse into the one
/// transition node master05 §ISM_TRANSITION maps, taking the attribute's
/// occurrences. An unfilled slot or a constraint ref yields no node.
fn build_attribute_children(
    ctx: &Ctx,
    attr: &openehr_its::opt14::types::CAttribute,
    node: &WebTemplateNode,
    scope: Scope<'_>,
    children: &mut Vec<WebTemplateNode>,
) {
    let attr_name = inputs::attribute_name(attr);
    // The openEHR terminology group a child's coded value binds to, fixed by
    // (this node's RM type, the attribute) — used to resolve rubrics from the
    // correct group (SPECPR-51 code collisions; see `inputs::openehr_group`).
    let child_group = inputs::openehr_group(&node.rm_type, attr_name);
    let merged = attr_name == shape::MERGED_ATTRIBUTE;
    let identity = if merged {
        shape::Identity::AttributeOnly
    } else {
        shape::Identity::Archetyped
    };
    let mut built = Vec::new();
    for child_co in inputs::attribute_children(attr) {
        if matches!(
            child_co,
            CObject::ArchetypeSlot(_) | CObject::ConstraintRef(_)
        ) {
            continue;
        }
        built.push(build_node(
            ctx,
            Some(attr),
            child_co,
            &node.aql_path,
            scope,
            child_group,
            identity,
        ));
    }
    if !merged {
        children.extend(built);
        return;
    }
    if let Some(mut transition) = shape::merge_alternatives(built) {
        let (min, max) = occurrences(attribute_existence(attr));
        transition.min = min;
        transition.max = max;
        children.push(transition);
    }
}

fn build_children(
    ctx: &Ctx,
    co: &CObject,
    node: &mut WebTemplateNode,
    scope: Scope<'_>,
    group: Option<&str>,
) {
    let is_data_value = node.rm_type.starts_with("DV_");
    let recurse_attrs = !is_data_value || node.rm_type.starts_with("DV_INTERVAL");

    let mut children = Vec::new();
    if recurse_attrs {
        for attr in inputs::attributes(co) {
            // The `name` attribute is never a child node — it names the node
            // (master04 §"Field Identifiers": names generate the node id).
            if inputs::attribute_name(attr) != "name" {
                build_attribute_children(ctx, attr, node, scope, &mut children);
            }
        }
        let declared = children.len();
        let implicit = implicit::any_element_value(co, &node.rm_type).into_iter();
        for attr in implicit.chain(implicit::interval_parts_v1_4(co, &node.rm_type)) {
            build_attribute_children(ctx, &attr, node, scope, &mut children);
        }
        children
            .iter_mut()
            .skip(declared)
            .for_each(|c| c.implicit = true);
    }

    // A party node keeps its inputs with children: master05's PARTY_PROXY tables
    // put the suffixes and the `/relationship` sub-path on the same node.
    if has_inputs(&node.rm_type) && (children.is_empty() || is_party(&node.rm_type)) {
        let labels = ArchetypeLabels { ctx, scope };
        let (built, ptypes) = inputs::build_inputs(&node.rm_type, co, &labels, group);
        node.inputs = built;
        node.proportion_types = ptypes;
        capture_leaf_constraints(co, node);
    }
    node.string_constraint = inputs::string_constraint(co);

    node.cardinalities = cardinalities(co, &node.aql_path);
    node.card_all = all_cardinalities(co, &node.aql_path);
    // Existence is captured only for structural (attribute-recursing) nodes; a
    // DATA_VALUE leaf's constraints (`magnitude`, `is_integral`, `value`, …) are
    // handled by `inputs`/leaf checks, not attribute navigation.
    if recurse_attrs {
        node.existence = existence_constraints(co, &node.aql_path);
        node.closed_attributes = closed_attributes(co, &node.aql_path);
        node.prohibited_attributes = prohibited_attributes(co);
        node.structural_stubs = structural_stubs(ctx, co, scope, &node.aql_path);
    }
    node.children = children;
    shape::post_process(node);
}

/// Whether a node of this RM type is a web-template leaf carrying `inputs`.
///
/// All three `PARTY_PROXY` subtypes are leaves (master05 §§PARTY_SELF,
/// PARTY_IDENTIFIED, PARTY_RELATED); `relationship` is a sub-path.
fn has_inputs(rm_type: &str) -> bool {
    rm_type.starts_with("DV_") || is_party(rm_type) || rm_type == "CODE_PHRASE"
}

/// The `PARTY_PROXY` family: the abstract type an unnarrowed slot keeps, plus
/// the two concrete subtypes an OPT can narrow one to (`PARTY_SELF` carries no
/// constrainable attribute of its own, so no template names it).
fn is_party(rm_type: &str) -> bool {
    matches!(
        rm_type,
        "PARTY_PROXY" | "PARTY_IDENTIFIED" | "PARTY_RELATED"
    )
}

// ── cardinalities ────────────────────────────────────────────────────────────

fn cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        if let openehr_its::opt14::types::CAttribute::CMultipleAttribute(m) = attr
            && requires_cardinality(&m.cardinality, m.children.len())
        {
            let (min, max) = occurrences(&m.cardinality.interval);
            out.push(WebTemplateCardinality {
                min,
                max,
                ids: None,
                path: format!("{node_path}/{}", m.rm_attribute_name),
                unique: m.cardinality.is_unique,
            });
        }
    }
    out
}

/// Every constraining multiple-attribute cardinality, for the validation walk
/// (AOM 1.4 `master04-constraint_model_package.adoc` §cardinality). A superset
/// of the serialized [`requires_cardinality`] selection.
fn all_cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        if let openehr_its::opt14::types::CAttribute::CMultipleAttribute(m) = attr {
            let (min, max) = occurrences(&m.cardinality.interval);
            let unique = m.cardinality.is_unique;
            if min.unwrap_or(0) >= 1 || max != -1 || unique {
                out.push(WebTemplateCardinality {
                    min,
                    max,
                    ids: None,
                    path: format!("{node_path}/{}", m.rm_attribute_name),
                    unique,
                });
            }
        }
    }
    out
}

/// Capture the leaf value constraints the `inputs` mapping does not carry:
/// numeric lists, a `C_DURATION` range and the code lists of coded attributes
/// other than `defining_code` (AOM 1.4 §`C_INTEGER`, §`C_REAL`, §`C_DURATION`,
/// §`C_CODE_PHRASE`).
fn capture_leaf_constraints(co: &CObject, node: &mut WebTemplateNode) {
    // `size` covers `DV_MULTIMEDIA.size` (RM `data_types` §`DV_MULTIMEDIA`,
    // `size: Integer`), whose `C_INTEGER` list/range the `inputs` builder does
    // not model (the DV_MULTIMEDIA input is only the `value` text input).
    for datum in ["magnitude", "value", "numerator", "denominator", "size"] {
        match inputs::primitive_under(co, datum) {
            Some(CPrimitive::CInteger(ci)) if !ci.list.is_empty() => {
                node.numeric_lists.push((
                    datum.to_owned(),
                    ci.list.iter().map(|v| f64::from(*v)).collect(),
                ));
            }
            Some(CPrimitive::CReal(cr)) if !cr.list.is_empty() => {
                node.numeric_lists.push((datum.to_owned(), cr.list.clone()));
            }
            _ => {}
        }
    }
    capture_size_range(co, node);
    capture_leaf_existence(co, node);
    capture_duration_range(co, node);
    // C_TIME/C_DATE_TIME timezone_validity (VALIDITY_KIND: OPT 1.4 XSD 1001 =
    // mandatory, 1002 = optional, 1003 = disallowed). C_DATE has no timezone.
    node.tz_validity = match inputs::primitive_under(co, "value") {
        Some(CPrimitive::CTime(c)) => c.timezone_validity.map(validity_code),
        Some(CPrimitive::CDateTime(c)) => c.timezone_validity.map(validity_code),
        _ => None,
    };
    capture_quantity_property(co, node);
    capture_code_lists(co, node);
}

/// `C_DURATION.range` on `value` → [`WebTemplateNode::duration_range`] (AOM 1.4
/// §`C_DURATION`), keeping each bound's inclusivity.
fn capture_duration_range(co: &CObject, node: &mut WebTemplateNode) {
    let Some(CPrimitive::CDuration(d)) = inputs::primitive_under(co, "value") else {
        return;
    };
    let Some(range) = &d.range else { return };
    let min = if range.lower_unbounded {
        None
    } else {
        range.lower.clone()
    };
    let max = if range.upper_unbounded {
        None
    } else {
        range.upper.clone()
    };
    if min.is_none() && max.is_none() {
        return;
    }
    let min_strict = range.lower_included == Some(false);
    let max_strict = range.upper_included == Some(false);
    node.duration_range = Some(super::model::WebTemplateRange {
        min_op: min
            .as_ref()
            .map(|_| if min_strict { ">" } else { ">=" }.to_owned()),
        min: min.map(serde_json::Value::String),
        max_op: max
            .as_ref()
            .map(|_| if max_strict { "<" } else { "<=" }.to_owned()),
        max: max.map(serde_json::Value::String),
    });
}

/// `C_QUANTITY.property`, so the instance's `units` can be checked against the
/// property's unit set (`org.openehr.am.aom14.c_quantity.adoc` §C_QUANTITY).
///
/// Only an openEHR-terminology code counts; the placeholder "0" is no constraint.
fn capture_quantity_property(co: &CObject, node: &mut WebTemplateNode) {
    if let CObject::CDvQuantity(q) = co
        && let Some(property) = &q.property
        && property
            .terminology_id
            .value
            .eq_ignore_ascii_case("openehr")
        && !property.code_string.is_empty()
        && property.code_string != "0"
    {
        node.quantity_property = Some(property.code_string.clone());
    }
}

/// The `C_CODE_PHRASE` code lists the `inputs` mapping does not carry: the
/// explicit `local` scoping of `defining_code` and the lists on every other
/// coded attribute (AOM 1.4 §`C_CODE_PHRASE`; `aom14.c_coded_text.adoc`).
///
/// NOTE: `master04-constraint_model_package.adoc` §Reference Objects resolves a
/// CONSTRAINT_REF through an external query, so it is not captured here.
fn capture_code_lists(co: &CObject, node: &mut WebTemplateNode) {
    let defining_cp = match co {
        CObject::CCodePhrase(cp) => Some(cp),
        _ => inputs::attr_children(co, "defining_code").find_map(|c| match c {
            CObject::CCodePhrase(cp) => Some(cp),
            _ => None,
        }),
    };
    if let Some(cp) = defining_cp
        && !cp.code_list.is_empty()
        && cp
            .terminology_id
            .as_ref()
            .is_some_and(|t| t.value.eq_ignore_ascii_case("local"))
    {
        node.coded_terminology_local = true;
    }
    for attr in inputs::attributes(co) {
        let attr_name = inputs::attribute_name(attr);
        if attr_name == "defining_code" {
            continue; // Modelled by the coded-text `inputs`.
        }
        for child in inputs::attribute_children(attr) {
            if let CObject::CCodePhrase(cp) = child
                && !cp.code_list.is_empty()
            {
                node.code_lists.push(WebTemplateCodeList {
                    attr: attr_name.to_owned(),
                    terminology: cp
                        .terminology_id
                        .as_ref()
                        .map(|t| t.value.clone())
                        .filter(|t| !t.is_empty()),
                    codes: cp.code_list.clone(),
                });
            }
        }
    }
}

/// `DV_MULTIMEDIA.size` against a `C_INTEGER.range` (AOM 1.4 §`C_INTEGER`); the
/// other numeric data carry their range through `inputs`.
fn capture_size_range(co: &CObject, node: &mut WebTemplateNode) {
    let Some(CPrimitive::CInteger(ci)) = inputs::primitive_under(co, "size") else {
        return;
    };
    let Some(iv) = &ci.range else { return };
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded { None } else { iv.upper };
    if min.is_none() && max.is_none() {
        return;
    }
    let min_strict = iv.lower_included == Some(false);
    let max_strict = iv.upper_included == Some(false);
    node.numeric_ranges.push((
        "size".to_owned(),
        super::model::WebTemplateRange {
            min_op: min.map(|_| if min_strict { ">" } else { ">=" }.to_owned()),
            min: min.map(serde_json::Value::from),
            max_op: max.map(|_| if max_strict { "<" } else { "<=" }.to_owned()),
            max: max.map(serde_json::Value::from),
        },
    ));
}

/// AOM 1.4 `C_ATTRIBUTE.existence` on a DV leaf's data attributes (e.g.
/// `DV_IDENTIFIER.issuer`) that the OPT both mandates and constrains.
///
/// An attribute carrying only the tooling-default `{1..1}` existence with no
/// value constraint is left unenforced (see the NOTE on [`existence_constraints`]).
fn capture_leaf_existence(co: &CObject, node: &mut WebTemplateNode) {
    for attr in inputs::attributes(co) {
        let openehr_its::opt14::types::CAttribute::CSingleAttribute(s) = attr else {
            continue;
        };
        if s.rm_attribute_name == "name" {
            continue;
        }
        let constrained = node.inputs.iter().any(|i| {
            i.suffix.as_deref() == Some(s.rm_attribute_name.as_str())
                && ((!i.list.is_empty() && i.list_open != Some(true))
                    || i.validation.as_ref().is_some_and(|v| v.pattern.is_some()))
        });
        if !constrained {
            continue;
        }
        let (min, max) = occurrences(&s.existence);
        if min.unwrap_or(0) >= 1 {
            node.existence.push(WebTemplateExistence {
                min: min.unwrap_or(0),
                max,
                path: format!("{}/{}", node.aql_path, s.rm_attribute_name),
            });
        }
    }
}

// ── closed-archetype constraints (AOM2 closed-world direction) ────────────────

/// Capture the closed-archetype constraints: per attribute of `co` with
/// node-identified child alternatives or open slots, the admissible child
/// identities keyed by absolute archetype path.
///
/// An attribute whose children carry no `node_id` and has no slot stays open
/// (AOM 1.4 `master04-constraint_model_package.adoc` §`node_id`).
fn closed_attributes(co: &CObject, node_path: &str) -> Vec<WebTemplateClosedAttribute> {
    inputs::attributes(co)
        .iter()
        .filter_map(|attr| closed_attribute(attr, node_path))
        .collect()
}

/// The closure record for one constraint attribute, or [`None`] when it stays
/// open: the predicate-matched `name`, an unresolved reference, or no
/// node-id alternative and no slot.
fn closed_attribute(
    attr: &openehr_its::opt14::types::CAttribute,
    node_path: &str,
) -> Option<WebTemplateClosedAttribute> {
    let attr_name = inputs::attribute_name(attr);
    if attr_name == "name" {
        return None;
    }
    if inputs::attribute_children(attr).iter().any(|c| {
        matches!(
            c,
            CObject::ArchetypeInternalRef(_) | CObject::ConstraintRef(_)
        )
    }) {
        return None;
    }
    let (allowed, slots) = admissible_children(attr);
    if allowed.is_empty() && slots.is_empty() {
        return None;
    }
    let (allowed_ids, allowed_rm_types) = allowed.into_iter().unzip();
    Some(WebTemplateClosedAttribute {
        path: format!("{node_path}/{attr_name}"),
        allowed_ids,
        allowed_rm_types,
        slots,
    })
}

/// The `(archetype node id, rm_type_name)` alternatives and the slots one
/// attribute admits as children.
fn admissible_children(
    attr: &openehr_its::opt14::types::CAttribute,
) -> (Vec<(String, String)>, Vec<WebTemplateArchetypeSlot>) {
    let mut allowed: Vec<(String, String)> = Vec::new();
    let mut slots: Vec<WebTemplateArchetypeSlot> = Vec::new();
    for child in inputs::attribute_children(attr) {
        if let CObject::ArchetypeSlot(s) = child {
            slots.push(archetype_slot(s));
            continue;
        }
        let id = object_archetype_node_id(child);
        if !id.is_empty() {
            allowed.push((id, object_rm_type(child).to_owned()));
        }
    }
    (allowed, slots)
}

// ── structural stubs (constrained-but-content-less structural attrs) ─────────

/// The structural attributes the FLAT builder and the example generator
/// synthesise when empty (`org.openehr.rm.composition.action.adoc`,
/// `org.openehr.rm.data_structures.event.adoc`, `…composition.activity.adoc`).
const STRUCTURAL_ATTRS: [&str; 4] = ["data", "description", "protocol", "state"];

/// Captures the [`WebTemplateStructuralStub`]s of a structural node.
///
/// The compactor drops an empty wrapper, so the stub keeps the constrained
/// identity the builder fills the attribute with (AOM 1.4
/// `master04-constraint_model_package.adoc` §`Valid_value`). Without a stub the
/// attribute keeps the `at0001` "Any" placeholder (ADL 1.4 `master05-cadl.adoc`).
fn structural_stubs(
    ctx: &Ctx,
    co: &CObject,
    scope: Scope<'_>,
    node_path: &str,
) -> Vec<WebTemplateStructuralStub> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        let attr_name = inputs::attribute_name(attr);
        if !STRUCTURAL_ATTRS.contains(&attr_name) {
            continue;
        }
        for child in inputs::attribute_children(attr) {
            // An unresolved slot / reference gives no concrete identity to stamp;
            // leave the attribute to the spec-legal "Any" placeholder.
            if matches!(
                child,
                CObject::ArchetypeSlot(_)
                    | CObject::ConstraintRef(_)
                    | CObject::ArchetypeInternalRef(_)
            ) {
                continue;
            }
            let node_id = object_archetype_node_id(child);
            if node_id.is_empty() {
                continue;
            }
            out.push(WebTemplateStructuralStub {
                attr: attr_name.to_owned(),
                path: format!("{node_path}/{attr_name}"),
                rm_type: object_rm_type(child).to_owned(),
                node_id,
                name: ctx.text(scope, object_node_id(child), &ctx.default_language),
            });
            break; // first node-identified structural child under this attribute
        }
    }
    out
}

/// Build the validation-only slot record from an OPT `ARCHETYPE_SLOT`: its
/// constrained RM type, occurrences bounds, and the archetype-id regexes lifted
/// from the `includes`/`excludes` assertions (AOM 1.4 `ARCHETYPE_SLOT`).
fn archetype_slot(s: &openehr_its::opt14::types::ArchetypeSlot) -> WebTemplateArchetypeSlot {
    let (min, max) = occurrences(&s.occurrences);
    WebTemplateArchetypeSlot {
        rm_type: s.rm_type_name.clone(),
        min: min.unwrap_or(0).max(0),
        max,
        includes: s.includes.iter().filter_map(slot_pattern).collect(),
        excludes: s.excludes.iter().filter_map(slot_pattern).collect(),
    }
}

/// The archetype-id regex of an OPT-1.4 slot `ASSERTION`, read from its
/// expression tree, else the string form.
///
/// The tree is the authority (`org.openehr.am.aom14.assertion.adoc`); the slot
/// form is `archetype_id/value matches {/<regex>/}` (ADL 1.4 `master05-cadl.adoc`
/// §Defining Slots). Any other assertion yields `None`.
fn slot_pattern(a: &Assertion) -> Option<String> {
    expression_pattern(a).or_else(|| string_expression_pattern(a))
}

/// The `VALIDITY_KIND` facet value as the integer the Web Template carries
/// (ITS-XML `ALL/Archetype.xsd` §`VALIDITY_KIND`).
fn validity_code(kind: ValidityKind) -> i32 {
    match kind {
        ValidityKind::Mandatory => 1001,
        ValidityKind::Optional => 1002,
        ValidityKind::Disallowed => 1003,
    }
}

/// The archetype-id regex read from an assertion's expression tree.
fn expression_pattern(a: &Assertion) -> Option<String> {
    let ExprItem::ExprBinaryOperator(op) = a.expression.as_ref() else {
        return None;
    };
    // `op_matches` — ITS-XML `ALL/Archetype.xsd` §`OPERATOR_KIND`
    // `<xs:enumeration value="2007" id="matches"/>`.
    if op.operator != OperatorKind::Matches {
        return None;
    }
    let (ExprItem::ExprLeaf(left), ExprItem::ExprLeaf(right)) =
        (op.left_operand.as_ref(), op.right_operand.as_ref())
    else {
        return None;
    };
    if !is_archetype_id_reference(&left.item.text()) {
        return None;
    }
    let pattern = c_string_constraint(&right.item)?.child("pattern")?.text();
    let pattern = pattern.trim();
    (!pattern.is_empty()).then(|| pattern.to_owned())
}

/// The archetype-id regex parsed out of an assertion's `string_expression`;
/// archetype ids contain no `/`, so the last `/}` delimits it.
fn string_expression_pattern(a: &Assertion) -> Option<String> {
    let s = a.string_expression.as_deref()?;
    if !is_archetype_id_reference(s) {
        return None;
    }
    let (_, rest) = s.split_once("matches {/")?;
    let end = rest.rfind("/}")?;
    Some(rest.get(..end)?.to_owned())
}

/// Whether an assertion operand references the filler's `archetype_id` (ADL 1.4
/// `master05-cadl.adoc` §Archetype Slots).
fn is_archetype_id_reference(operand: &str) -> bool {
    let operand = operand.trim();
    operand == "archetype_id"
        || operand
            .strip_prefix("archetype_id")
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The `C_STRING` constraint of a `matches` right operand, direct or wrapped in
/// a `C_PRIMITIVE_OBJECT` (`aom14.expr_leaf.adoc`, `aom14.c_primitive_object.adoc`).
fn c_string_constraint(
    item: &openehr_its::xml::runtime::XmlAny,
) -> Option<&openehr_its::xml::runtime::XmlAny> {
    match item.xsi_type()? {
        "C_STRING" => Some(item),
        "C_PRIMITIVE_OBJECT" => item
            .child("item")
            .filter(|inner| inner.xsi_type() == Some("C_STRING")),
        _ => None,
    }
}

fn requires_cardinality(card: &Cardinality, children_count: usize) -> bool {
    let iv = &card.interval;
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded { None } else { iv.upper };
    if min.is_none() && max.is_none() {
        return false;
    }
    let Some(min) = min else { return false };
    if min == 0 {
        return false;
    }
    let count = i32::try_from(children_count).unwrap_or(i32::MAX);
    if min == 1 && max == Some(1) && children_count == 1 {
        false
    } else {
        min > 1 || max.is_some_and(|m| m < count)
    }
}

// ── ontology / languages / other_details ─────────────────────────────────────

fn collect_languages(opt: &OperationalTemplate, default_language: &str) -> Vec<String> {
    let mut langs = vec![default_language.to_owned()];
    let mut push = |flat: &openehr_its::opt14::types::FlatArchetypeOntology| {
        for set in &flat.term_definitions {
            if !langs.contains(&set.language) {
                langs.push(set.language.clone());
            }
        }
    };
    if let Some(o) = &opt.ontology {
        push(o);
    }
    for o in &opt.component_ontologies {
        push(o);
    }
    langs
}

fn collect_ontology(opt: &OperationalTemplate) -> Ontology {
    let mut ontology: Ontology = HashMap::new();

    let mut register_flat = |flat: &openehr_its::opt14::types::FlatArchetypeOntology| {
        let arch = ontology.entry(flat.archetype_id.clone()).or_default();
        for set in &flat.term_definitions {
            let lang = arch.entry(set.language.clone()).or_default();
            for term in &set.items {
                lang.entry(term.code.clone())
                    .or_insert_with(|| rubric_of(term));
            }
        }
    };
    if let Some(o) = &opt.ontology {
        register_flat(o);
    }
    for o in &opt.component_ontologies {
        register_flat(o);
    }
    ontology
}

/// The default-language rubrics of one archetype root's inline
/// `term_definitions`, first entry per code winning.
fn inline_rubrics(root: &CArchetypeRoot) -> HashMap<String, Rubric> {
    let mut out = HashMap::new();
    for term in &root.term_definitions {
        out.entry(term.code.clone())
            .or_insert_with(|| rubric_of(term));
    }
    out
}

/// Collect archetype roots at or below `co`, without descending past a root
/// (each found root handles its own subtree).
fn collect_nested_roots<'a>(co: &'a CObject, out: &mut Vec<&'a CArchetypeRoot>) {
    if let CObject::CArchetypeRoot(r) = co {
        out.push(r);
        return;
    }
    for attr in inputs::attributes(co) {
        for child in inputs::attribute_children(attr) {
            collect_nested_roots(child, out);
        }
    }
}

/// Collect every archetype root's inline `term_bindings`, keyed by archetype id;
/// the first root wins.
fn collect_term_bindings(opt: &OperationalTemplate) -> TermBindings {
    let mut out: TermBindings = HashMap::new();
    collect_root_bindings(&opt.definition, &mut out);
    out
}

fn collect_root_bindings(root: &CArchetypeRoot, out: &mut TermBindings) {
    if !root.term_bindings.is_empty() {
        out.entry(root.archetype_id.value.clone())
            .or_insert_with(|| root.term_bindings.clone());
    }
    // Recurse into nested archetype roots (each carries its own bindings).
    let mut nested = Vec::new();
    for attr in &root.attributes {
        for child in inputs::attribute_children(attr) {
            collect_nested_roots(child, &mut nested);
        }
    }
    for r in nested {
        collect_root_bindings(r, out);
    }
}

/// Collect the ontologies' `constraint_bindings`, keyed by archetype id
/// (`AM/docs/ADL1.4/master08-adl.adoc` §Constraint_bindings).
fn collect_constraint_bindings(opt: &OperationalTemplate) -> ConstraintBindings {
    let mut out: ConstraintBindings = HashMap::new();
    for onto in opt.ontology.iter().chain(opt.component_ontologies.iter()) {
        if onto.constraint_bindings.is_empty() {
            continue;
        }
        out.entry(onto.archetype_id.clone())
            .or_default()
            .extend(onto.constraint_bindings.iter().cloned());
    }
    out
}

fn rubric_of(term: &ArchetypeTerm) -> Rubric {
    Rubric {
        text: term.items.get("text").cloned(),
        description: term.items.get("description").cloned(),
    }
}

fn other_details(opt: &OperationalTemplate) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    if let Some(desc) = &opt.description
        && let Some(details) = &desc.other_details
    {
        // `otherDetails` keeps only `is_singleton`. No openEHR spec governs this
        // root field — our own design/extension.
        if let Some(v) = details.get("is_singleton") {
            out.insert("is_singleton".to_owned(), v.clone());
        }
    }
    out
}

// ── opt14 object metadata ────────────────────────────────────────────────────

fn object_rm_type(co: &CObject) -> &str {
    match co {
        CObject::CComplexObject(c) => &c.rm_type_name,
        CObject::CArchetypeRoot(c) => &c.rm_type_name,
        CObject::CCodePhrase(c) => &c.rm_type_name,
        CObject::CCodeReference(c) => &c.rm_type_name,
        CObject::CDvOrdinal(c) => &c.rm_type_name,
        CObject::CDvQuantity(c) => &c.rm_type_name,
        CObject::CDvState(c) => &c.rm_type_name,
        CObject::CPrimitiveObject(c) => &c.rm_type_name,
        CObject::CDefinedObject(c) => &c.rm_type_name,
        CObject::ArchetypeInternalRef(c) => &c.rm_type_name,
        CObject::ArchetypeSlot(c) => &c.rm_type_name,
        CObject::ConstraintRef(c) => &c.rm_type_name,
        CObject::TComplexObject(c) => &c.rm_type_name,
    }
}

fn object_node_id(co: &CObject) -> &str {
    match co {
        CObject::CComplexObject(c) => &c.node_id,
        CObject::CArchetypeRoot(c) => &c.node_id,
        CObject::CCodePhrase(c) => &c.node_id,
        CObject::CCodeReference(c) => &c.node_id,
        CObject::CDvOrdinal(c) => &c.node_id,
        CObject::CDvQuantity(c) => &c.node_id,
        CObject::CDvState(c) => &c.node_id,
        CObject::CPrimitiveObject(c) => &c.node_id,
        CObject::CDefinedObject(c) => &c.node_id,
        CObject::ArchetypeInternalRef(c) => &c.node_id,
        CObject::ArchetypeSlot(c) => &c.node_id,
        CObject::ConstraintRef(c) => &c.node_id,
        CObject::TComplexObject(c) => &c.node_id,
    }
}

/// The RM `archetype_node_id`: the archetype id at archetype roots, else the
/// constraint node id (`atNNNN`).
fn object_archetype_node_id(co: &CObject) -> String {
    match co {
        CObject::CArchetypeRoot(c) => c.archetype_id.value.clone(),
        _ => object_node_id(co).to_owned(),
    }
}

pub(super) fn object_occurrences(co: &CObject) -> &Intervalofinteger {
    match co {
        CObject::CComplexObject(c) => &c.occurrences,
        CObject::CArchetypeRoot(c) => &c.occurrences,
        CObject::CCodePhrase(c) => &c.occurrences,
        CObject::CCodeReference(c) => &c.occurrences,
        CObject::CDvOrdinal(c) => &c.occurrences,
        CObject::CDvQuantity(c) => &c.occurrences,
        CObject::CDvState(c) => &c.occurrences,
        CObject::CPrimitiveObject(c) => &c.occurrences,
        CObject::CDefinedObject(c) => &c.occurrences,
        CObject::ArchetypeInternalRef(c) => &c.occurrences,
        CObject::ArchetypeSlot(c) => &c.occurrences,
        CObject::ConstraintRef(c) => &c.occurrences,
        CObject::TComplexObject(c) => &c.occurrences,
    }
}

/// The node's effective `min`: its occurrences lower bound met with the owning
/// single attribute's existence lower bound.
///
/// ADL 1.4 `master05-cadl.adoc` §Occurrences: the occurrences of a single-valued
/// attribute's value "is already defined by the attribute `existence`". A
/// container attribute is left alone.
fn meet_single_existence(
    occurrences_min: Option<i32>,
    owner: Option<&openehr_its::opt14::types::CAttribute>,
) -> Option<i32> {
    let Some(openehr_its::opt14::types::CAttribute::CSingleAttribute(s)) = owner else {
        return occurrences_min;
    };
    let existence_min = occurrences(&s.existence).0.unwrap_or(0);
    occurrences_min.map(|m| m.min(existence_min))
}

/// The node's effective `max`: its occurrences upper bound capped at `1` under
/// a single attribute (ADL 1.4 `master05-cadl.adoc` §Occurrences, §Cardinality).
fn cap_single_upper(
    occurrences_max: i32,
    owner: Option<&openehr_its::opt14::types::CAttribute>,
) -> i32 {
    match owner {
        Some(openehr_its::opt14::types::CAttribute::CSingleAttribute(_))
            if occurrences_max == -1 || occurrences_max > 1 =>
        {
            1
        }
        _ => occurrences_max,
    }
}

/// `(min, max)` from an occurrences/cardinality interval; `max == -1` unbounded.
pub(super) fn occurrences(iv: &Intervalofinteger) -> (Option<i32>, i32) {
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded { None } else { iv.upper };
    (min, max.unwrap_or(-1))
}

/// A single fixed name constraint (`name/value` `C_STRING` with one value), used
/// for the `[atNNNN,'Name']` path predicate.
fn name_constraint(co: &CObject) -> Option<String> {
    for name_child in inputs::attr_children(co, "name") {
        if let Some(CPrimitive::CString(cs)) = inputs::primitive_under(name_child, "value")
            && cs.list.len() == 1
        {
            return cs.list.first().cloned();
        }
    }
    None
}

/// The `DV_CODED_TEXT` name constraint on `co`'s `name` attribute: display
/// value plus `defining_code`, or `None` for an unconstrained or plain name (RM
/// common `master03-archetyped_package.adoc` §"The `LOCATABLE` class").
///
/// With `explicit` the code is the `code_list` member whose rubric equals it,
/// else the sole member, else the first.
fn coded_name_constraint(
    co: &CObject,
    ctx: &Ctx,
    scope: Scope<'_>,
    explicit: Option<&str>,
) -> Option<(String, CodedName)> {
    let name_child =
        inputs::attr_children(co, "name").find(|c| object_rm_type(c) == "DV_CODED_TEXT")?;
    let code_phrase = inputs::attr_children(name_child, "defining_code").find_map(|c| match c {
        CObject::CCodePhrase(cp) => Some(cp),
        _ => None,
    })?;
    let (first, rest) = code_phrase.code_list.split_first()?;
    let terminology = code_phrase
        .terminology_id
        .as_ref()
        .map(|t| t.value.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "local".to_owned());
    // Prefer the candidate code whose archetype rubric equals the fixed name
    // value, so a same-`archetype_node_id` sibling differentiated by a renamed
    // `name/value` keeps a code consistent with that value.
    let matched = explicit.and_then(|v| {
        std::iter::once(first)
            .chain(rest)
            .find(|c| ctx.text(scope, c, &ctx.default_language).as_deref() == Some(v))
            .cloned()
    });
    // A fixed value with NO rubric-matching candidate (and more than one
    // candidate to choose from) is display/rubric-incoherent — see
    // [`CodedName::incoherent`].
    let incoherent = explicit.is_some() && matched.is_none() && !rest.is_empty();
    let code = matched.unwrap_or_else(|| first.clone());
    let value = explicit
        .map(str::to_owned)
        .or_else(|| ctx.text(scope, &code, &ctx.default_language))
        .unwrap_or_else(|| code.clone());
    Some((
        value,
        CodedName {
            terminology,
            code,
            incoherent,
        },
    ))
}

fn build_path(
    parent_path: &str,
    attr_name: Option<&str>,
    arch_node_id: &str,
    name_constraint: Option<&str>,
) -> String {
    let Some(attr) = attr_name else {
        return String::new(); // root
    };
    let predicate = if arch_node_id.is_empty() {
        String::new()
    } else if let Some(name) = name_constraint {
        format!("[{arch_node_id},'{name}']")
    } else {
        format!("[{arch_node_id}]")
    };
    format!("{parent_path}/{attr}{predicate}")
}
