// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decomposition walk: one scoped source archetype per embedded
//! `C_ARCHETYPE_ROOT`, with each embedded child replaced by a slot.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_its::opt14;

use super::primitive::{complex, map_primitive_object, terminology_code};
use super::slot::{archetype_id_include, map_code_reference, map_slot_assertions};
use super::tuple::{
    binding_uri, dv_state_loose, first_ac_num, map_constraint_ref, ordinal_tuple, quantity_tuple,
    single_valued,
};
use super::value::{code_constraint, map_cardinality, map_mult, map_term};
use super::{FillEdge, RawUnit, RootCx};

/// The decomposition state over one operational template.
pub(super) struct Decomposer<'a> {
    language: String,
    root_ontology: Option<&'a opt14::types::FlatArchetypeOntology>,
    component_ontologies: &'a [opt14::types::FlatArchetypeOntology],
    units: Vec<RawUnit>,
    edges: Vec<FillEdge>,
}

impl<'a> Decomposer<'a> {
    /// A decomposer over `opt`, with no root processed yet.
    pub(super) fn new(opt: &'a opt14::types::OperationalTemplate) -> Self {
        Self {
            language: opt.language.code_string.clone(),
            root_ontology: opt.ontology.as_ref(),
            component_ontologies: &opt.component_ontologies,
            units: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// The OPT's authoring language, the decomposed roots in discovery order,
    /// and the recovered fill edges.
    pub(super) fn finish(self) -> (String, Vec<RawUnit>, Vec<FillEdge>) {
        (self.language, self.units, self.edges)
    }
}

impl Decomposer<'_> {
    /// Decompose one `C_ARCHETYPE_ROOT` into a scoped 1.4-shaped source archetype
    /// (pushed onto `units`), recursing into any embedded child roots. `is_top`
    /// selects the OPT's top-level `ontology` vs a `component_ontologies` entry
    /// for the archetype's flattened terminology.
    pub(super) fn process_root(
        &mut self,
        root: &opt14::types::CArchetypeRoot,
        path: &str,
        is_top: bool,
    ) {
        let archetype_id = root.archetype_id.value.clone();
        // Slot at-codes and minted ac codes allocate strictly above this root's
        // own retained subtree and its flattened ontology, so a substituted slot
        // never collides with a real node in the parent's code space.
        let ontology = self.ontology_for(&archetype_id, is_top);
        let defined_acs: std::collections::BTreeSet<String> = ontology
            .iter()
            .flat_map(|o| &o.constraint_definitions)
            .flat_map(|set| &set.items)
            .map(|t| t.code.clone())
            .collect();
        let next_ac = defined_acs
            .iter()
            .filter_map(|c| first_ac_num(c))
            .max()
            .unwrap_or(0)
            + 1;
        let mut cx = RootCx {
            slot_num: max_at_num(&root.attributes) + 1,
            next_ac,
            defined_acs,
            extra_terms: Vec::new(),
            extra_bindings: Vec::new(),
            notes: BTreeMap::new(),
        };
        let attributes = self.map_attributes(&root.attributes, &archetype_id, path, &mut cx);
        let definition = CComplexObject::CComplexObject(CComplexObjectData {
            parent: None,
            soc_parent: None,
            rm_type_name: root.rm_type_name.clone(),
            // A source-archetype root declares no occurrences of its own.
            occurrences: None,
            node_id: root.node_id.clone(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            attributes: openehr_base::containers::present(attributes),
            attribute_tuples: openehr_base::containers::present(Vec::new()),
        });
        let terminology = self.build_terminology(&archetype_id, root, is_top, &mut cx);
        self.units.push(RawUnit {
            archetype_id,
            definition,
            terminology,
            notes: cx.notes,
        });
    }

    /// The flattened ontology slice for one archetype root: the OPT `ontology`
    /// for the top root, the matching `component_ontologies` entry for an
    /// embedded one.
    fn ontology_for(
        &self,
        archetype_id: &str,
        is_top: bool,
    ) -> Option<&'_ opt14::types::FlatArchetypeOntology> {
        if is_top {
            self.root_ontology
        } else {
            self.component_ontologies
                .iter()
                .find(|o| o.archetype_id == archetype_id)
        }
    }

    fn map_attributes(
        &mut self,
        attrs: &[opt14::types::CAttribute],
        archetype_id: &str,
        path: &str,
        cx: &mut RootCx,
    ) -> Vec<CAttribute> {
        let mut out = Vec::with_capacity(attrs.len());
        for attr in attrs {
            out.push(self.map_attribute(attr, archetype_id, path, cx));
        }
        out
    }

    fn map_attribute(
        &mut self,
        attr: &opt14::types::CAttribute,
        archetype_id: &str,
        path: &str,
        cx: &mut RootCx,
    ) -> CAttribute {
        let (rm_attribute_name, existence, children, cardinality, is_multiple) = match attr {
            opt14::types::CAttribute::CSingleAttribute(a) => (
                a.rm_attribute_name.clone(),
                &a.existence,
                &a.children,
                None,
                false,
            ),
            opt14::types::CAttribute::CMultipleAttribute(a) => (
                a.rm_attribute_name.clone(),
                &a.existence,
                &a.children,
                Some(map_cardinality(&a.cardinality)),
                true,
            ),
        };
        let attr_path = format!("{path}/{rm_attribute_name}");
        let mut mapped_children = Vec::with_capacity(children.len());
        for c in children {
            let capped = (!is_multiple)
                .then(|| single_valued(c, &attr_path, cx))
                .flatten();
            let child = capped.as_ref().unwrap_or(c);
            mapped_children.push(self.map_object(child, archetype_id, &attr_path, cx));
        }
        CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name,
            existence: Some(map_mult(existence)),
            children: openehr_base::containers::present(mapped_children),
            differential_path: None,
            cardinality,
            is_multiple,
        }
    }

    fn map_object(
        &mut self,
        obj: &opt14::types::CObject,
        archetype_id: &str,
        path: &str,
        cx: &mut RootCx,
    ) -> CObject {
        match obj {
            // An embedded root becomes its own source, leaving a slot in this
            // parent whose `include` names the archetype that filled it — the
            // `EXPR_ARCHETYPE_REF matches EXPR_ARCHETYPE_ID_CONSTRAINT` form of
            // `org.openehr.am.aom2.archetype_slot.adoc`.
            opt14::types::CObject::CArchetypeRoot(child) => {
                let slot_node_id = format!("at{}", cx.slot_num);
                cx.slot_num += 1;
                let child_id = child.archetype_id.value.clone();
                self.edges.push(FillEdge {
                    parent_archetype_id: archetype_id.to_owned(),
                    parent_path: format!("{path}[{slot_node_id}]"),
                    slot_node_id: slot_node_id.clone(),
                    child_archetype_id: child_id.clone(),
                });
                let includes = archetype_id_include(&child_id, &slot_node_id, cx);
                let slot = CObject::ArchetypeSlot(ArchetypeSlot {
                    parent: None,
                    soc_parent: None,
                    rm_type_name: child.rm_type_name.clone(),
                    occurrences: Some(map_mult(&child.occurrences)),
                    node_id: slot_node_id,
                    alternative_ids: openehr_base::containers::present(Vec::new()),
                    is_deprecated: None,
                    sibling_order: None,
                    includes: openehr_base::containers::present(includes),
                    excludes: openehr_base::containers::present(Vec::new()),
                    is_closed: false,
                });
                self.process_root(child, path, false);
                slot
            }
            opt14::types::CObject::CComplexObject(c) => {
                let attributes = self.map_attributes(&c.attributes, archetype_id, path, cx);
                complex(&c.rm_type_name, &c.node_id, &c.occurrences, attributes)
            }
            // A `T_COMPLEX_OBJECT` is a template-node complex object; its
            // `default_value` (a DATA_VALUE) is carried into the converted
            // source as `C_DEFINED_OBJECT.default_value` — legal in any
            // archetype and serialized by the printer as the `_default`
            // pseudo-attribute (`master06-default_values.adoc` §Syntax; the
            // intermediate is the canonical-JSON encoding).
            opt14::types::CObject::TComplexObject(c) => {
                let attributes = self.map_attributes(&c.attributes, archetype_id, path, cx);
                let mut obj = complex(&c.rm_type_name, &c.node_id, &c.occurrences, attributes);
                if let Some(dv) = &c.default_value
                    && let CObject::CComplexObject(CComplexObject::CComplexObject(d)) = &mut obj
                {
                    d.default_value = Some(openehr_its::json::to_canonical_value(dv));
                }
                obj
            }
            opt14::types::CObject::CDefinedObject(c) => {
                complex(&c.rm_type_name, &c.node_id, &c.occurrences, Vec::new())
            }
            opt14::types::CObject::ArchetypeInternalRef(r) => {
                CObject::CComplexObjectProxy(CComplexObjectProxy {
                    parent: None,
                    soc_parent: None,
                    rm_type_name: r.rm_type_name.clone(),
                    occurrences: Some(map_mult(&r.occurrences)),
                    node_id: r.node_id.clone(),
                    alternative_ids: openehr_base::containers::present(Vec::new()),
                    is_deprecated: None,
                    sibling_order: None,
                    target_path: r.target_path.clone(),
                })
            }
            opt14::types::CObject::ArchetypeSlot(s) => {
                let includes = map_slot_assertions(&s.includes, &s.node_id, "include", cx);
                let excludes = map_slot_assertions(&s.excludes, &s.node_id, "exclude", cx);
                CObject::ArchetypeSlot(ArchetypeSlot {
                    parent: None,
                    soc_parent: None,
                    rm_type_name: s.rm_type_name.clone(),
                    occurrences: Some(map_mult(&s.occurrences)),
                    node_id: s.node_id.clone(),
                    alternative_ids: openehr_base::containers::present(Vec::new()),
                    is_deprecated: None,
                    sibling_order: None,
                    includes: openehr_base::containers::present(includes),
                    excludes: openehr_base::containers::present(excludes),
                    is_closed: false,
                })
            }
            opt14::types::CObject::CCodePhrase(c) => terminology_code(
                &c.rm_type_name,
                &c.node_id,
                &c.occurrences,
                code_constraint(
                    c.terminology_id.as_ref().map(|t| t.value.as_str()),
                    &c.code_list,
                    c.assumed_value.as_ref().map(|a| a.code_string.as_str()),
                ),
            ),
            opt14::types::CObject::CCodeReference(c) => map_code_reference(c, cx),
            opt14::types::CObject::ConstraintRef(r) => map_constraint_ref(r, cx),
            opt14::types::CObject::CDvOrdinal(c) => ordinal_tuple(c, cx),
            opt14::types::CObject::CDvQuantity(c) => quantity_tuple(c, cx),
            opt14::types::CObject::CDvState(c) => dv_state_loose(c, cx),
            opt14::types::CObject::CPrimitiveObject(c) => map_primitive_object(c, cx),
        }
    }

    /// Assemble the 1.4-shaped terminology for one archetype root: the inline
    /// `C_ARCHETYPE_ROOT.term_definitions`/`term_bindings` (keyed under the OPT's
    /// language) merged with the matching flattened ontology
    /// (`ontology` for the top root, `component_ontologies[archetype_id]` for an
    /// embedded root). Extra (unused) codes are harmless — they raise at most a
    /// `WOUC` warning, never a phase-1 error.
    fn build_terminology(
        &self,
        archetype_id: &str,
        root: &opt14::types::CArchetypeRoot,
        is_top: bool,
        cx: &mut RootCx,
    ) -> ArchetypeTerminology {
        let mut term_definitions: BTreeMap<String, BTreeMap<String, ArchetypeTerm>> =
            BTreeMap::new();
        let mut term_bindings: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

        // Inline terms on the root node are language-less; key them under the
        // OPT's authoring language.
        {
            let inline = term_definitions.entry(self.language.clone()).or_default();
            for t in &root.term_definitions {
                inline.insert(t.code.clone(), map_term(t));
            }
        }
        for set in &root.term_bindings {
            let bucket = term_bindings.entry(set.terminology.clone()).or_default();
            for item in &set.items {
                bucket.insert(
                    item.code.clone(),
                    binding_uri(&set.terminology, &item.value.code_string),
                );
            }
        }

        if let Some(ont) = self.ontology_for(archetype_id, is_top) {
            merge_ontology(ont, &mut term_definitions, &mut term_bindings);
        }

        {
            let inline = term_definitions.entry(self.language.clone()).or_default();
            for t in &cx.extra_terms {
                inline.insert(t.code.clone(), t.clone());
            }
        }
        for (terminology, code, uri) in &cx.extra_bindings {
            term_bindings
                .entry(terminology.clone())
                .or_default()
                .insert(code.clone(), uri.clone());
        }

        drop_out_of_scope_bindings(&term_definitions, &mut term_bindings, cx);

        ArchetypeTerminology {
            is_differential: false,
            original_language: self.language.clone(),
            concept_code: root.node_id.clone(),
            term_definitions,
            term_bindings: (!term_bindings.is_empty()).then_some(term_bindings),
            value_sets: None,
            terminology_extracts: None,
        }
    }
}

/// Merges one flattened 1.4 ontology into the terminology being assembled.
///
/// The 1.4 `constraint_definitions`/`constraint_bindings` sections merge into
/// `term_definitions`/`term_bindings` — the ADL2 folding
/// (`ADL2/master07.13-adl_terminology.adoc` §Terminology section). AOM2
/// binding targets are URIs (`term_bindings: Hash<String, Hash<String,
/// Uri>>`), so a 1.4 target that is a bare code (`20081-6`) is wrapped in the
/// converter core's fabricated URN form and stays parseable as ODIN; a
/// constraint binding carries its target as a plain string (a URI or a
/// terminology query) and merges under the same terminology key, ac-code →
/// target (VTCBK keys).
pub(super) fn merge_ontology(
    ont: &opt14::types::FlatArchetypeOntology,
    term_definitions: &mut BTreeMap<String, BTreeMap<String, ArchetypeTerm>>,
    term_bindings: &mut BTreeMap<String, BTreeMap<String, String>>,
) {
    for set in ont
        .term_definitions
        .iter()
        .chain(&ont.constraint_definitions)
    {
        let bucket = term_definitions.entry(set.language.clone()).or_default();
        for t in &set.items {
            bucket.insert(t.code.clone(), map_term(t));
        }
    }
    for set in &ont.term_bindings {
        let bucket = term_bindings.entry(set.terminology.clone()).or_default();
        for item in &set.items {
            bucket.insert(
                item.code.clone(),
                binding_uri(&set.terminology, &item.value.code_string),
            );
        }
    }
    for set in &ont.constraint_bindings {
        let bucket = term_bindings.entry(set.terminology.clone()).or_default();
        for item in &set.items {
            bucket.insert(
                item.code.clone(),
                binding_uri(&set.terminology, &item.value),
            );
        }
    }
}

/// Drops every binding whose key is not a code DEFINED in this root's slice,
/// recording each drop in `conversion_details`.
///
/// Such a binding is unexpressible after decomposition (1.4 OPTs may bind path
/// keys or codes scoped to another embedded root) and would raise VTTBK
/// (`master03` §Validity Rules — binding keys must be defined codes). The
/// binding's home is the root that defines the code, which carries it in its
/// own slice.
pub(super) fn drop_out_of_scope_bindings(
    term_definitions: &BTreeMap<String, BTreeMap<String, ArchetypeTerm>>,
    term_bindings: &mut BTreeMap<String, BTreeMap<String, String>>,
    cx: &mut RootCx,
) {
    let defined: std::collections::BTreeSet<String> = term_definitions
        .values()
        .flat_map(|by_code| by_code.keys().cloned())
        .collect();
    for (terminology, by_key) in &mut *term_bindings {
        let terminology = terminology.clone();
        by_key.retain(|key, _| {
            let keep = defined.contains(key.as_str());
            if !keep {
                cx.notes.insert(
                    format!("dropped_term_binding.{terminology}.{key}"),
                    format!(
                        "term binding [{terminology}::{key}] is outside this root's code \
                         scope; dropped at OPT decomposition (its home is the root that \
                         defines the code)"
                    ),
                );
            }
            keep
        });
    }
    term_bindings.retain(|_, by_key| !by_key.is_empty());
}

/// The maximum at-code number among a root's own node ids (not descending into
/// embedded child roots — their codes belong to the child's space).
pub(super) fn max_at_num(attrs: &[opt14::types::CAttribute]) -> i64 {
    let mut max = 0;
    for attr in attrs {
        let children = match attr {
            opt14::types::CAttribute::CSingleAttribute(a) => &a.children,
            opt14::types::CAttribute::CMultipleAttribute(a) => &a.children,
        };
        for child in children {
            max = max.max(max_at_num_obj(child));
        }
    }
    max
}

pub(super) fn max_at_num_obj(obj: &opt14::types::CObject) -> i64 {
    match obj {
        // Do NOT descend into an embedded root: its at-codes are its own space.
        opt14::types::CObject::CArchetypeRoot(_) => 0,
        opt14::types::CObject::CComplexObject(c) => {
            at_num(&c.node_id).max(max_at_num(&c.attributes))
        }
        opt14::types::CObject::TComplexObject(c) => {
            at_num(&c.node_id).max(max_at_num(&c.attributes))
        }
        opt14::types::CObject::CDefinedObject(c) => at_num(&c.node_id),
        opt14::types::CObject::ArchetypeInternalRef(r) => at_num(&r.node_id),
        opt14::types::CObject::ArchetypeSlot(s) => at_num(&s.node_id),
        opt14::types::CObject::ConstraintRef(r) => at_num(&r.node_id),
        opt14::types::CObject::CCodePhrase(c) => at_num(&c.node_id),
        opt14::types::CObject::CCodeReference(c) => at_num(&c.node_id),
        opt14::types::CObject::CDvOrdinal(c) => at_num(&c.node_id),
        opt14::types::CObject::CDvQuantity(c) => at_num(&c.node_id),
        opt14::types::CObject::CDvState(c) => at_num(&c.node_id),
        opt14::types::CObject::CPrimitiveObject(c) => at_num(&c.node_id),
    }
}

/// The numeric value of an `atNNNN` node id's first segment (0 for a non-at
/// code, e.g. an empty id or an already-id code).
///
/// The code grammar is not restated here: the string goes through the AOM2 code
/// parser ([`crate::codes::parse_code`], the
/// `org.openehr.am.aom2.adl_code_definitions` leader + `.`-separated numeric
/// segments), so an at-code is recognised exactly as the rest of the ADL layer
/// recognises one.
pub(super) fn at_num(node_id: &str) -> i64 {
    match crate::codes::parse_code(node_id) {
        Some(code) if code.prefix == crate::codes::CodePrefix::At => code
            .segments
            .first()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        _ => 0,
    }
}
