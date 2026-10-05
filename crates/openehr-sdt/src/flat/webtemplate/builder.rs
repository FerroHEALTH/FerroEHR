// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT 1.4 → Web Template walk.
//!
//! Walks the OPT 1.4 constraint tree ([`openehr_its::opt14`]) into the Web
//! Template metadata document (`ITS-REST simplified_formats master04
//! §"Web Template Metadata"`):
//!
//! 1. **build** a node per constraint object (rm type, node id, aql path,
//!    occurrences, rubric names, term bindings), giving `DATA_VALUE`/PARTY leaves
//!    their `inputs` and running the per-rm post-processors;
//! 2. **compact** the tree per master04 §"Level Removal": elide the container
//!    attribute names, collapse the always-collapsed wrapper types
//!    (`ITEM_*`/`ITEM_STRUCTURE`/`HISTORY`) and a conditionally-collapsed single
//!    `EVENT`, promote an `ELEMENT`/`DATA_VALUE` single child, and drop empties;
//! 3. **assign ids** (see `id`, master04 §"Node ID Generation Rules").
//!
//! The web template mirrors the CONSTRAINT tree of the OPT, so RM structure the
//! operational template does not constrain is not synthesized here. Three
//! boundaries follow from that:
//!
//! * RM-mandatory attributes the OPT leaves unconstrained are not injected as
//!   nodes. The FLAT/TDD composition builders fill them on `RM ← FLAT/TDD` and
//!   [`crate::flat::validation`] enforces existence and occurrences, so the
//!   produced COMPOSITION is RM-valid without the builder duplicating the
//!   structure (cardinalities: openEHR RM `common` / `composition` /
//!   `data_structures`).
//! * An ELEMENT with no value constraint is emitted without an enumerated
//!   per-`DATA_VALUE` `inputs` expansion. No openEHR spec governs the shape of
//!   an unconstrained value node — our own design/extension.
//! * An archetype internal reference (`use_node`) is emitted as its own node
//!   rather than resolved to its target subtree.

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

use super::inputs::{self, Labels};
use super::model::{
    CodedName, WebTemplate, WebTemplateArchetypeSlot, WebTemplateBindingCodedValue,
    WebTemplateCardinality, WebTemplateClosedAttribute, WebTemplateCodeList,
    WebTemplateConstraintBinding, WebTemplateExistence, WebTemplateNode, WebTemplateStructuralStub,
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

/// Build context shared across the walk.
struct Ctx {
    default_language: String,
    languages: Vec<String>,
    ontology: Ontology,
    term_bindings: TermBindings,
    constraint_bindings: ConstraintBindings,
}

impl Ctx {
    fn rubric(&self, arch_id: &str, lang: &str, code: &str) -> Option<&Rubric> {
        self.ontology.get(arch_id)?.get(lang)?.get(code)
    }

    fn text(&self, arch_id: &str, code: &str, lang: &str) -> Option<String> {
        self.rubric(arch_id, lang, code)
            .and_then(|r| r.text.clone())
    }

    /// The external term bindings for `code` within `arch_id`'s archetype
    /// (master04 §"Web Template Metadata": a node/coded-value `termBindings` map):
    /// for every terminology whose binding set has an item matching `code`, the
    /// bound code phrase as a `{value, terminologyId}`. Keyed by terminology,
    /// first match per terminology wins, in binding-set order.
    fn term_bindings_for(
        &self,
        arch_id: &str,
        code: &str,
    ) -> IndexMap<String, WebTemplateBindingCodedValue> {
        let mut out = IndexMap::new();
        if code.is_empty() {
            return out;
        }
        let Some(sets) = self.term_bindings.get(arch_id) else {
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

    /// The external constraint bindings of `ac_code` within `arch_id`'s
    /// archetype: one entry per terminology whose `ConstraintBindingSet` binds
    /// the ac-code to a query URI (`AM/docs/ADL1.4/master08-adl.adoc`
    /// §Constraint_bindings). `attr` is the RM attribute of the leaf carrying
    /// the constrained code. An empty query URI is dropped — an unbound
    /// ac-code constrains nothing resolvable.
    fn constraint_bindings_for(
        &self,
        arch_id: &str,
        ac_code: &str,
        attr: &str,
    ) -> Vec<WebTemplateConstraintBinding> {
        let mut out = Vec::new();
        if ac_code.is_empty() {
            return out;
        }
        let Some(sets) = self.constraint_bindings.get(arch_id) else {
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

/// Every `CONSTRAINT_REF` (`ac`-code proxy) in force on `co`, resolved against
/// the archetype ontology's `constraint_bindings`: the node itself when it IS a
/// `CONSTRAINT_REF` (a bare `CODE_PHRASE` proxy), plus one per coded attribute
/// (`defining_code`, …) whose child is a `CONSTRAINT_REF`. AOM 1.4
/// `master04-constraint_model_package.adoc` §Reference Objects.
fn constraint_bindings_of(
    ctx: &Ctx,
    co: &CObject,
    arch_id: &str,
) -> Vec<WebTemplateConstraintBinding> {
    let mut out = Vec::new();
    if let CObject::ConstraintRef(cr) = co {
        out.extend(ctx.constraint_bindings_for(arch_id, &cr.reference, ""));
    }
    for attr in inputs::attributes(co) {
        let attr_name = inputs::attribute_name(attr);
        for child in inputs::attribute_children(attr) {
            if let CObject::ConstraintRef(cr) = child {
                out.extend(ctx.constraint_bindings_for(arch_id, &cr.reference, attr_name));
            }
        }
    }
    out
}

/// The bound code phrase's code string with its terminology id (falling back to
/// the binding item's own code when the code phrase carries no terminology).
/// `None` when the bound code string is blank. Feeds the `termBindings` map of
/// master04 §"Web Template Metadata".
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
    arch_id: &'a str,
}

impl Labels for ArchetypeLabels<'_> {
    fn text(&self, terminology: &str, code: &str) -> Option<String> {
        // An EXTERNAL code's rubric is keyed by the qualified
        // `TERMINOLOGY::code` form in the OPT ontology (OPT 1.4
        // `term_definitions code="SNOMED-CT::…"`); the bare-code key serves
        // the local at-codes. Try bare first (the overwhelmingly common
        // case), then qualified.
        self.ctx
            .text(self.arch_id, code, &self.ctx.default_language)
            .or_else(|| {
                self.ctx.text(
                    self.arch_id,
                    &format!("{terminology}::{code}"),
                    &self.ctx.default_language,
                )
            })
    }

    fn localized(&self, terminology: &str, code: &str) -> IndexMap<String, String> {
        let mut out = IndexMap::new();
        for lang in &self.ctx.languages {
            if let Some(t) = self.ctx.text(self.arch_id, code, lang).or_else(|| {
                self.ctx
                    .text(self.arch_id, &format!("{terminology}::{code}"), lang)
            }) {
                out.insert(lang.clone(), t);
            }
        }
        out
    }

    fn term_bindings(&self, code: &str) -> IndexMap<String, WebTemplateBindingCodedValue> {
        self.ctx.term_bindings_for(self.arch_id, code)
    }
}

/// Build a [`WebTemplate`] from a parsed OPT 1.4 operational template.
///
/// The document shape (field names, `tree`/`children`, `inputs`, `aqlPath`), the
/// node-id derivation, and the compaction are governed by `ITS-REST
/// simplified_formats master04-basic_concepts.adoc` (§"Web Template Metadata",
/// §"Node ID Generation Rules", §"Level Removal"). `version` is the format
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
        ontology: collect_ontology(opt, &default_language),
        term_bindings: collect_term_bindings(opt),
        constraint_bindings: collect_constraint_bindings(opt),
        default_language,
    };

    let root_arch_id = opt.definition.archetype_id.value.clone();
    let root_co = CObject::CArchetypeRoot(opt.definition.clone());
    let mut tree = build_node(
        &ctx,
        None,
        &root_co,
        "",
        &root_arch_id,
        None,
        shape::Identity::Archetyped,
    );
    tree = shape::compact(tree, 1).unwrap_or_else(shape::tree_placeholder);
    // Synthesize the in-context RM children the master04 example carries for the
    // structural attributes an OPT commonly leaves unconstrained (COMPOSITION
    // context/category/language/territory/composer, per-ENTRY language/encoding/
    // subject, per-EVENT time) BEFORE id assignment, so their ids go through the
    // normal sibling-uniqueness discipline (super::id).
    shape::synthesize_in_context(&mut tree);
    super::id::build_ids(&mut tree);
    // Parse the archetype-conformance walk's template-static constraint paths
    // ONCE now, so the validation walk never re-parses them per instance-node
    // visit.
    crate::flat::validation::prepare_walk(&mut tree);

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
    })
}

// ── node build ───────────────────────────────────────────────────────────────

fn build_node(
    ctx: &Ctx,
    owner: Option<&openehr_its::opt14::types::CAttribute>,
    co: &CObject,
    parent_path: &str,
    parent_arch_id: &str,
    group: Option<&str>,
    identity: shape::Identity,
) -> WebTemplateNode {
    // A C_ARCHETYPE_ROOT switches the ontology scope to its own archetype.
    let node_arch_id = match co {
        CObject::CArchetypeRoot(r) => r.archetype_id.value.as_str(),
        _ => parent_arch_id,
    };
    let mut node = create_node(ctx, owner, co, parent_path, node_arch_id, identity);
    build_children(ctx, co, &mut node, node_arch_id, group);
    node
}

fn create_node(
    ctx: &Ctx,
    owner: Option<&openehr_its::opt14::types::CAttribute>,
    co: &CObject,
    parent_path: &str,
    arch_id: &str,
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
    let (occ_min, max) = occurrences(object_occurrences(co));
    let min = meet_single_existence(occ_min, owner);
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
    node.node_id = if arch_node_id.is_empty() {
        None
    } else {
        Some(arch_node_id)
    };
    node.min = min;
    node.max = max;

    let name_code = if archetyped { object_node_id(co) } else { "" };
    // A `DV_CODED_TEXT` name constraint (RM common master03 §LOCATABLE — the
    // name is `DV_TEXT` *or* `DV_CODED_TEXT`) fixes both the display value and
    // the `defining_code` the composition builder must stamp; it takes
    // precedence over the plain `name/value` and node-rubric name.
    let coded_name = archetyped
        .then(|| coded_name_constraint(co, ctx, arch_id, name_constraint.as_deref()))
        .flatten();
    apply_node_naming(
        &mut node,
        ctx,
        arch_id,
        name_code,
        name_constraint.as_deref(),
        coded_name,
    );
    // Node-level external term bindings: the archetype root's `term_bindings`
    // whose item code matches this node's constraint node id (master04
    // §"Web Template Metadata": the node-level `termBindings` map).
    node.term_bindings = ctx.term_bindings_for(arch_id, name_code);
    // Archetype constraint bindings (ac-code → external terminology query) in
    // force on this node's coded value — resolvable only by a terminology
    // service, so recorded rather than checked here (BASE
    // `architecture_overview/master12-terminology.adoc` §"Binding Terminology
    // Value-sets to Archetypes").
    node.constraint_bindings = constraint_bindings_of(ctx, co, arch_id);
    node
}

/// Fills a node's name, localized names/descriptions and name code.
///
/// A coded name constraint wins over an explicit plain `name/value`, which wins
/// over the node rubric; only the rubric route carries per-language text.
fn apply_node_naming(
    node: &mut WebTemplateNode,
    ctx: &Ctx,
    arch_id: &str,
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
        node.name = ctx.text(arch_id, name_code, &ctx.default_language);
        node.localized_name.clone_from(&node.name);
        apply_localized_rubrics(node, ctx, arch_id, name_code);
    }
    node.name_code = if name_code.is_empty() {
        None
    } else {
        Some(name_code.to_owned())
    };
}

/// Records the node rubric's per-language text and description.
fn apply_localized_rubrics(node: &mut WebTemplateNode, ctx: &Ctx, arch_id: &str, name_code: &str) {
    for lang in &ctx.languages {
        let Some(r) = ctx.rubric(arch_id, lang, name_code) else {
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
/// transition node master05 §ISM_TRANSITION maps, so they are built without
/// their at-code identity (see [`shape::MERGED_ATTRIBUTE`]) and the merged
/// node takes the ATTRIBUTE's occurrences — one required transition per ACTION
/// instance, not one per careflow state. An unfilled slot or a constraint ref
/// yields no node.
fn build_attribute_children(
    ctx: &Ctx,
    attr: &openehr_its::opt14::types::CAttribute,
    node: &WebTemplateNode,
    arch_id: &str,
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
            arch_id,
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
    arch_id: &str,
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
                build_attribute_children(ctx, attr, node, arch_id, &mut children);
            }
        }
    }

    // A party node keeps its inputs even when it HAS children: master05's three
    // PARTY_PROXY tables put the `|name`/`|id`/`|id_scheme`/`|id_namespace`
    // suffixes and the `/relationship` + `/_identifier:i` SUB-PATHS on the same
    // node, so a template that narrows a party and constrains `relationship`
    // (the shape the vendored `Test constrained subject` / `MED - Perinatal
    // history Summary` templates use) still owns every party suffix. For every
    // other leaf family a child means the node is a container, not a datum.
    if has_inputs(&node.rm_type) && (children.is_empty() || is_party(&node.rm_type)) {
        let labels = ArchetypeLabels { ctx, arch_id };
        let (built, ptypes) = inputs::build_inputs(&node.rm_type, co, &labels, group);
        node.inputs = built;
        node.proportion_types = ptypes;
        capture_leaf_constraints(co, node);
    }

    node.cardinalities = cardinalities(co, &node.aql_path);
    node.card_all = all_cardinalities(co, &node.aql_path);
    // Existence is captured only for structural (attribute-recursing) nodes; a
    // DATA_VALUE leaf's constraints (`magnitude`, `is_integral`, `value`, …) are
    // handled by `inputs`/leaf checks, not attribute navigation.
    if recurse_attrs {
        node.existence = existence_constraints(co, &node.aql_path);
        node.closed_attributes = closed_attributes(co, &node.aql_path);
        if shape::is_entry_family(&node.rm_type) {
            node.structural_stubs = structural_stubs(ctx, co, arch_id);
        }
    }
    node.children = children;
    shape::post_process(node);
}

/// Whether a node of this RM type is a web-template LEAF — one that carries
/// `inputs` rather than child nodes.
///
/// The PARTY arm covers all three concrete `PARTY_PROXY` subtypes the
/// Simplified Formats spec gives their own mapping table: master05
/// §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED share the
/// `|id`/`|id_scheme`/`|id_namespace` rows and the latter two add `|name`, so a
/// slot an OPT narrows to `PARTY_RELATED` is the same party leaf as one left at
/// `PARTY_IDENTIFIED` — its extra `relationship` is a DV_CODED_TEXT SUB-PATH
/// (master05 §"PARTY_RELATED performer": "the `relationship` attribute is
/// emitted as a sub-path under the participation, with the standard
/// DV_CODED_TEXT suffixes"), not a reason to demote the party itself to a
/// container.
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
            });
        }
    }
    out
}

/// Capture **every** constraining multiple-attribute cardinality for the
/// validation walk (AOM 1.4 `master04-constraint_model_package.adoc`
/// §cardinality): any interval with a lower bound `>= 1` or a bounded upper
/// bound constrains the container. This is the superset of the serialized
/// [`requires_cardinality`] selection, which drops `0..1`/`1..1`/`1..*` — those
/// intervals are still real archetype constraints (master15/16 truth tables)
/// and are enforced from [`WebTemplateNode::card_all`], never serialized.
fn all_cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        if let openehr_its::opt14::types::CAttribute::CMultipleAttribute(m) = attr {
            let (min, max) = occurrences(&m.cardinality.interval);
            if min.unwrap_or(0) >= 1 || max != -1 {
                out.push(WebTemplateCardinality {
                    min,
                    max,
                    ids: None,
                    path: format!("{node_path}/{}", m.rm_attribute_name),
                });
            }
        }
    }
    out
}

/// Capture the leaf value constraints the `inputs` mapping does not carry, onto
/// the node's validation-only fields:
///
/// - `C_INTEGER.list` / `C_REAL.list` on a numeric datum (`magnitude`, `value`)
///   → [`WebTemplateNode::numeric_lists`] (AOM 1.4 §`C_INTEGER/§C_REAL`);
/// - `C_DURATION.range` on `value` → [`WebTemplateNode::duration_range`]
///   (AOM 1.4 §`C_DURATION`);
/// - `C_CODE_PHRASE` code lists on coded attributes other than
///   `defining_code` (e.g. `DV_MULTIMEDIA.media_type`) →
///   [`WebTemplateNode::code_lists`] (AOM 1.4 §`C_CODE_PHRASE`).
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
/// §`C_DURATION`).
///
/// Inclusivity comes from the AOM interval flags (BASE `foundation_types`
/// Interval: `lower_included`/`upper_included`) — an exclusive bound
/// (`> PT0S`) must not degrade to `>=`.
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

/// `C_QUANTITY.property` (an openEHR `property`-group code), captured so the
/// instance's `units` can be checked against the property's unit set
/// (`AM/docs/UML/classes/org.openehr.am.aom14.c_quantity.adoc` §C_QUANTITY:
/// `property` = "Name of physical property for Quantities being
/// constrained").
///
/// Only the openEHR-terminology property code is meaningful, and the
/// placeholder "0" (Ocean Template Designer's unconstrained property) is
/// treated as no constraint, matching the OPT-side `C_DV_QUANTITY` check.
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
/// explicit `local` scoping of `defining_code`, and the lists on every other
/// coded attribute (e.g. `DV_MULTIMEDIA.media_type`) → AOM 1.4
/// §`C_CODE_PHRASE`.
///
/// An explicitly `local`-scoped closed code list admits ONLY local codes, so a
/// foreign-terminology instance code violates it
/// (`AM/docs/UML/classes/org.openehr.am.aom14.c_coded_text.adoc`
/// §C_CODED_TEXT: `code_list` is "a list of codes FROM the terminology"). The
/// `wt+json` `inputs` mapping strips the implicit `local`, so the explicit
/// scoping is recorded on the node instead (validation-only); a
/// `C_CODE_PHRASE` naming no terminology is not flagged.
///
/// NOTE: `AM/docs/AOM1.4/master04-constraint_model_package.adoc` §Reference
/// Objects resolves a CONSTRAINT_REF through an external terminology query,
/// not a local code list, so it is not captured as a leaf constraint.
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

/// `DV_MULTIMEDIA.size` against a `C_INTEGER.range` (AOM 1.4
/// `master04-constraint_model_package.adoc` §`C_INTEGER`): the instance's `size`
/// must lie within the declared `IntervalOfInteger`. The other numeric data
/// (`magnitude`/`numerator`/…) already carry their range through the `inputs`
/// builders (`count_input`, `quantity_inputs`, `proportion_part`), so only `size`
/// needs a validation-only range captured here.
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

/// AOM 1.4 `C_ATTRIBUTE.existence` on a DV leaf's directly-navigable data
/// attributes (`master04-constraint_model_package.adoc` §existence): an OPT may
/// narrow an RM-optional leaf attribute (e.g. `DV_IDENTIFIER.issuer`,
/// `.assigner`, `.type` — RM `data_types` §`DV_IDENTIFIER`) to mandatory
/// (existence lower `>= 1`), so a committed value missing that attribute must be
/// rejected. The structural [`existence_constraints`] pass runs only for
/// attribute-recursing nodes, and its primitive-child carve-out excludes exactly
/// these string data attributes.
///
/// Captured **only** when the OPT actually CONSTRAINS the sub-attribute (a
/// `C_STRING` closed list or pattern, surfaced on the matching leaf input) as
/// well as mandating it — an attribute carrying only the tooling-default
/// existence (`{1..1}` serialised for every attribute; see the NOTE on
/// [`existence_constraints`]) with no value constraint is left unenforced, so a
/// real template's optional-in-intent identifier field is not over-rejected.
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

// ── existence (AOM 1.4 C_ATTRIBUTE.existence) ─────────────────────────────────

/// Capture the AOM 1.4 `C_ATTRIBUTE.existence` constraints for the mandatory
/// single-valued RM attributes of `co`, keyed by their absolute archetype path.
///
/// Scope: `C_SINGLE_ATTRIBUTE`s with an existence lower bound `>= 1` and at
/// least one object-valued constraint child (excluding pure function/primitive
/// constraints such as `is_integral`/`lower_included`, which never appear as
/// navigable instance attributes). `name` is excluded (it names the node —
/// master04 §"Field Identifiers" — and is matched by the archetype-node
/// predicate instead).
///
/// A node-identified structural child (e.g. `OBSERVATION.state → HISTORY[at0005]`,
/// `HISTORY.summary → ITEM_TREE[at0007]`) is captured here **too**, not only when
/// the child carries no `node_id`: existence and *occurrences* are orthogonal
/// (AOM 1.4 §existence — "indicates whether its target object exists or not, i.e.
/// is mandatory or not"), and a mandated structural wrapper with no leaf content
/// is dropped by master04 §"Level Removal" compaction, so it never becomes a
/// walkable node the occurrence check could visit — leaving existence as the only
/// enforcement that the mandatory attribute is present at all. When the child
/// does survive, existence and occurrences both fire only on absence (redundant,
/// never contradictory).
///
/// NOTE: AOM 1.4 (`master04-constraint_model_package.adoc` §existence) makes
/// existence "always required" with an unstated default of `{1..1}`; the OPT XML
/// always serialises it, and we honour the declared value (biasing toward
/// confident violations — an unstated/`{0..1}` existence is not enforced).
fn existence_constraints(co: &CObject, node_path: &str) -> Vec<WebTemplateExistence> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        // A CONTAINER attribute with existence lower >= 1 demands the
        // attribute's presence regardless of member cardinality (AOM 1.4
        // c_attribute `Existence_set`; cardinality then governs membership of
        // the present container — the two constraints are orthogonal).
        if let openehr_its::opt14::types::CAttribute::CMultipleAttribute(m) = attr {
            let (min, max) = occurrences(&m.existence);
            if min.unwrap_or(0) >= 1 {
                out.push(WebTemplateExistence {
                    min: min.unwrap_or(0),
                    max,
                    path: format!("{node_path}/{}", m.rm_attribute_name),
                });
            }
            continue;
        }
        let openehr_its::opt14::types::CAttribute::CSingleAttribute(s) = attr else {
            continue;
        };
        let (min, max) = occurrences(&s.existence);
        let min = min.unwrap_or(0);
        // Require one object-valued constraint child, which selects real
        // structural attributes and excludes function/primitive constraints
        // (`is_integral`, `lower_included`, …) that are never navigable instance
        // attributes. A CHILDLESS mandatory attribute also counts: AOM 1.4
        // `master04-constraint_model_package.adoc` §existence lets an archetype
        // demand presence without constraining the value.
        let object_valued = s.children.is_empty()
            || s.children
                .iter()
                .any(|c| !matches!(c, CObject::CPrimitiveObject(_)));
        if min >= 1 && s.rm_attribute_name != "name" && object_valued {
            out.push(WebTemplateExistence {
                min,
                max,
                path: format!("{node_path}/{}", s.rm_attribute_name),
            });
        }
    }
    out
}

// ── closed-archetype constraints (AOM2 closed-world direction) ────────────────

/// Capture the closed-archetype constraints for the walk: per
/// attribute of `co` that carries **archetype-node-identified** child
/// alternatives (a fixed at-code / archetype-id sibling set) and/or open
/// `ARCHETYPE_SLOT`s, record the admissible child identities keyed by the
/// attribute's absolute archetype path. Captured from the raw OPT `co` (before
/// the tree build drops slots and before compaction hoists wrappers), so no
/// alternative is lost.
///
/// An attribute whose constraint children carry **no** `node_id` and has no slot
/// is left OPEN — AOM 1.4 (`master04-constraint_model_package.adoc` §`node_id`
/// L44: a near-leaf with no same-attribute siblings "can safely have no
/// `node_id`") — matching the RM-metadata / plain-attribute carve-out (closed-world capture
/// rule 2): `name`/`value`/`category`/`context` etc. hold non-LOCATABLE values
/// that carry no `archetype_node_id` and so are never subject to sibling closure.
fn closed_attributes(co: &CObject, node_path: &str) -> Vec<WebTemplateClosedAttribute> {
    inputs::attributes(co)
        .iter()
        .filter_map(|attr| closed_attribute(attr, node_path))
        .collect()
}

/// The closure record for one constraint attribute, or [`None`] when the
/// attribute stays open.
///
/// An attribute stays open when it is the predicate-matched `name` (master04
/// §"Field Identifiers"), when an unresolved internal-ref / constraint-ref makes
/// the admissible set uncertain (target resolution is a documented builder scope
/// gap — leave it open rather than risk over-rejecting), or when it constrains
/// no node-id alternative and no slot.
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
    let (allowed_ids, slots) = admissible_children(attr);
    if allowed_ids.is_empty() && slots.is_empty() {
        return None;
    }
    Some(WebTemplateClosedAttribute {
        path: format!("{node_path}/{attr_name}"),
        allowed_ids,
        slots,
    })
}

/// The archetype node ids and slots one attribute admits as children.
fn admissible_children(
    attr: &openehr_its::opt14::types::CAttribute,
) -> (Vec<String>, Vec<WebTemplateArchetypeSlot>) {
    let mut allowed_ids: Vec<String> = Vec::new();
    let mut slots: Vec<WebTemplateArchetypeSlot> = Vec::new();
    for child in inputs::attribute_children(attr) {
        if let CObject::ArchetypeSlot(s) = child {
            slots.push(archetype_slot(s));
            continue;
        }
        let id = object_archetype_node_id(child);
        if !id.is_empty() {
            allowed_ids.push(id);
        }
    }
    (allowed_ids, slots)
}

// ── structural stubs (constrained-but-content-less ENTRY structural attrs) ────

/// The RM-mandatory / structural attributes of an ENTRY whose value the
/// FLAT/TDD composition builder synthesises when the simplified form carries no
/// content under them (`RM/docs/UML/classes/org.openehr.rm.composition.*`): the
/// ENTRY `data`/`state`/`protocol` structural attributes plus `ACTION.description`
/// (existence `1..1`, `RM/docs/UML/classes/org.openehr.rm.composition.action.adoc`).
const ENTRY_STRUCTURAL_ATTRS: [&str; 4] = ["data", "description", "protocol", "state"];

/// Capture the [`WebTemplateStructuralStub`]s for an ENTRY node: for each
/// structural attribute (`data`/`description`/`protocol`/`state`) the OPT
/// constrains with a node-identified structural child, record its RM type,
/// archetype node id, and rubric name. When the attribute has no leaf content the
/// compactor drops the wrapper, so this is the only surviving record of the
/// *constrained* identity — the composition builder synthesises the empty
/// attribute from it (AOM 1.4 `master04-constraint_model_package.adoc`
/// §`Valid_value`: a constrained attribute must be filled by a conforming value),
/// instead of a blind `at0001` placeholder that a closed-archetype walk rejects.
///
/// The first node-identified, resolvable structural child per attribute is
/// recorded; unresolved refs / slots yield no stub (the attribute then keeps its
/// spec-legal `at0001` "Any" placeholder — ADL 1.4 `master05-cadl.adoc` §"Any"
/// Constraints).
fn structural_stubs(ctx: &Ctx, co: &CObject, arch_id: &str) -> Vec<WebTemplateStructuralStub> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        let attr_name = inputs::attribute_name(attr);
        if !ENTRY_STRUCTURAL_ATTRS.contains(&attr_name) {
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
                rm_type: object_rm_type(child).to_owned(),
                node_id,
                name: ctx.text(arch_id, object_node_id(child), &ctx.default_language),
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
/// EXPRESSION TREE, falling back to the string form.
///
/// `ASSERTION.expression` is the "Root of expression tree" and carries the
/// constraint; `string_expression` is only its optional "String form of
/// expression" (`AM UML/classes/org.openehr.am.aom14.assertion.adoc`
/// §ASSERTION Class), so the tree is both the authority and the only datum
/// present in most templates. The slot form is
/// `archetype_id/value matches {/<regex>/}` (ADL 1.4 `master05-cadl.adoc`
/// §Defining Slots on the basis of Archetype Identifiers and Concepts), whose
/// right operand carries the regex in `C_STRING.pattern`
/// (`…aom14.c_string.adoc` §C_STRING Class). Any other assertion — a different
/// reference, a literal-value list, the §Using Other Constraints in Slots form —
/// yields `None` rather than an invented archetype-id regex.
fn slot_pattern(a: &Assertion) -> Option<String> {
    expression_pattern(a).or_else(|| string_expression_pattern(a))
}

/// The `VALIDITY_KIND` facet value as the integer the Web Template carries.
///
/// The Web Template `tz_validity` field is a number on its own wire, while the
/// OPT-1.4 XML encodes the same fact as the `xs:enumeration` facet value
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

/// The archetype-id regex parsed out of an assertion's `string_expression`.
///
/// The surface form is `archetype_id/value matches {/<regex>/}` (ADL 1.4
/// `master05-cadl.adoc` §Defining Slots on the basis of Archetype Identifiers
/// and Concepts); archetype ids contain no `/`, so the last `/}` delimits the
/// regex.
fn string_expression_pattern(a: &Assertion) -> Option<String> {
    let s = a.string_expression.as_deref()?;
    if !is_archetype_id_reference(s) {
        return None;
    }
    let (_, rest) = s.split_once("matches {/")?;
    let end = rest.rfind("/}")?;
    Some(rest.get(..end)?.to_owned())
}

/// Whether an assertion operand references the filler archetype's identifier
/// rather than another `ARCHETYPE` property or an archetype path (ADL 1.4
/// `master05-cadl.adoc` §Archetype Slots lists `archetype_id`,
/// `parent_archetype_id`, `short_concept_name` and definition paths as the
/// admissible references).
fn is_archetype_id_reference(operand: &str) -> bool {
    let operand = operand.trim();
    operand == "archetype_id"
        || operand
            .strip_prefix("archetype_id")
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The `C_STRING` constraint of a `matches` right operand.
///
/// `EXPR_LEAF.item` is typed `Any` — "for the right-hand side of a 'matches'
/// node, a constraint, often a `C_PRIMITIVE_OBJECT`"
/// (`AM UML/classes/org.openehr.am.aom14.expr_leaf.adoc` §EXPR_LEAF Class), and
/// `C_PRIMITIVE_OBJECT.item` is the `C_PRIMITIVE` "actually defining the
/// constraint" (`…aom14.c_primitive_object.adoc` §C_PRIMITIVE_OBJECT Class), so
/// both the direct and the wrapped spelling resolve here.
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

fn collect_ontology(opt: &OperationalTemplate, default_language: &str) -> Ontology {
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

    // Inline C_ARCHETYPE_ROOT term_definitions are the flattened default-language
    // rubrics; register them (without overwriting an explicit ontology entry).
    collect_inline_terms(&opt.definition, default_language, &mut ontology);
    ontology
}

fn collect_inline_terms(root: &CArchetypeRoot, default_language: &str, ontology: &mut Ontology) {
    let arch = ontology.entry(root.archetype_id.value.clone()).or_default();
    let lang = arch.entry(default_language.to_owned()).or_default();
    for term in &root.term_definitions {
        lang.entry(term.code.clone())
            .or_insert_with(|| rubric_of(term));
    }
    // Recurse into nested archetype roots (each registers its own subtree's
    // rubrics under its own archetype id). Walk by reference — no cloning.
    let mut nested = Vec::new();
    for attr in &root.attributes {
        for child in inputs::attribute_children(attr) {
            collect_nested_roots(child, &mut nested);
        }
    }
    for r in nested {
        collect_inline_terms(r, default_language, ontology);
    }
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

/// Collect every archetype root's inline `term_bindings`, keyed by archetype id
/// (a node inherits its owning archetype root's bindings, surfaced as the
/// `termBindings` map of master04 §"Web Template Metadata"). First root wins for a
/// repeated archetype id.
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

/// Collect the flat ontologies' `constraint_bindings`, keyed by archetype id —
/// the ac-code → terminology-query URI map a `CONSTRAINT_REF` resolves through
/// (`AM/docs/ADL1.4/master08-adl.adoc` §Constraint_bindings). The root
/// `ontology` and every `component_ontologies` entry carries its own
/// `archetype_id`.
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

fn object_occurrences(co: &CObject) -> &Intervalofinteger {
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

/// The `C_ATTRIBUTE.existence` interval of `attr` (AOM 1.4
/// `master04-constraint_model_package.adoc` §existence).
fn attribute_existence(attr: &openehr_its::opt14::types::CAttribute) -> &Intervalofinteger {
    match attr {
        openehr_its::opt14::types::CAttribute::CSingleAttribute(s) => &s.existence,
        openehr_its::opt14::types::CAttribute::CMultipleAttribute(m) => &m.existence,
    }
}

/// The node's effective `min`: its own occurrences lower bound met with the
/// owning SINGLE attribute's `C_ATTRIBUTE.existence` lower bound.
///
/// ADL 1.4 `AM/docs/ADL1.4/master05-cadl.adoc` §Occurrences: occurrences "only
/// has significance for objects which are children of a container attribute,
/// since by definition, the occurrences of an object which is the value of a
/// single valued attribute can only be `0..1` or `1..1`, and this is already
/// defined by the attribute `existence`". Existence and occurrences are
/// orthogonal (AOM 1.4 `master04-constraint_model_package.adoc` §"Attribute Node
/// Types": existence "indicates whether an object will be found in a given
/// attribute field"), so an optional single attribute (`existence {0..1}`)
/// carrying a `1..1`-occurrences constraint — the shape OPT tooling emits for
/// `ISM_TRANSITION.careflow_step`, whose `master05-rm_mapping.adoc`
/// §`ISM_TRANSITION` row is Required "no" — yields an OPTIONAL child, not a
/// mandatory one.
///
/// A container attribute is left alone: there occurrences is the significant
/// constraint, and the attribute's own existence is reported separately by
/// [`existence_constraints`].
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

/// `(min, max)` from an occurrences/cardinality interval; `max == -1` unbounded.
fn occurrences(iv: &Intervalofinteger) -> (Option<i32>, i32) {
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

/// The `DV_CODED_TEXT` name constraint on `co`'s `name` attribute, when the
/// template constrains the runtime `LOCATABLE.name` as a coded text: the display
/// value plus the `defining_code` `(terminology, code)` the composition builder
/// must stamp. Returns `None` for an unconstrained name or a plain `DV_TEXT`
/// name constraint. Spec: RM common `master03-archetyped_package.adoc` §"The
/// `LOCATABLE` class" (name is `DV_TEXT` or `DV_CODED_TEXT`); AOM 1.4
/// `master04-constraint_model_package.adoc` (a `C_ATTRIBUTE` on `name`
/// constrains the whole coded name — `defining_code` + optional `value`).
///
/// `explicit` is the fixed `name/value` `C_STRING` value (from
/// [`name_constraint`]) when the template also narrows the coded name's text.
/// The code is chosen as the `code_list` member whose archetype rubric equals
/// `explicit` (so a name-differentiated sibling keeps its intended code), else
/// the sole member of a single-code list, else the first candidate. The display
/// value is `explicit` when present, else the chosen code's archetype rubric.
fn coded_name_constraint(
    co: &CObject,
    ctx: &Ctx,
    arch_id: &str,
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
            .find(|c| ctx.text(arch_id, c, &ctx.default_language).as_deref() == Some(v))
            .cloned()
    });
    // A fixed value with NO rubric-matching candidate (and more than one
    // candidate to choose from) is display/rubric-incoherent — see
    // [`CodedName::incoherent`].
    let incoherent = explicit.is_some() && matched.is_none() && !rest.is_empty();
    let code = matched.unwrap_or_else(|| first.clone());
    let value = explicit
        .map(str::to_owned)
        .or_else(|| ctx.text(arch_id, &code, &ctx.default_language))
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
