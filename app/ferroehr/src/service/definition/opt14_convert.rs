// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT-1.4 → ADL2 conversion front end.
//!
//! A stored 1.4 operational template is a specialisation-flattened artefact
//! whose `definition` embeds each component archetype as a nested
//! `C_ARCHETYPE_ROOT` with its own at-code space, so the component code spaces
//! collide if the whole is fed to `openehr_adl::adl14::convert::convert`, which
//! takes one source archetype. This front end decomposes the OPT into one
//! 1.4-shaped `openehr_am::v2_4` source archetype per embedded root, replacing
//! each child by an `ARCHETYPE_SLOT` whose `include` assertion names it and
//! recording the parent-to-child fill edge in [`OptConversion::structure`].
//! Anything a decomposed root cannot express is reported in
//! `RESOURCE_DESCRIPTION.conversion_details`.
//!
//! NOTE: no openEHR spec governs 1.4 → 2 conversion — the decomposition
//! strategy, slot substitution and code allocation here are our own
//! design/extension, as is `openehr_adl::adl14` itself.

use std::collections::BTreeMap;

use openehr_adl::adl14::convert::{ConvertConfig, ConvertError, convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::hrid::parse_hrid;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_boolean::CBoolean;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_date::CDate;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_date_time::CDateTime;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_duration::CDuration;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_time::CTime;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::{
    Cardinality, Interval, Iso8601Date, Iso8601DateTime, Iso8601Duration, Iso8601Time,
    MultiplicityInterval, PointInterval, ProperInterval, ProperIntervalData, TerminologyCode, Uuid,
};
use openehr_its::opt14;

/// A 1.4 → 2 OPT-conversion failure.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OptConvertError {
    /// An embedded archetype root carried an unparseable `ARCHETYPE_ID`.
    #[error("OPT embedded root has an invalid archetype id {0:?}: {1}")]
    Hrid(String, String),
    /// The converter core rejected a decomposed source.
    #[error("converting decomposed source {0:?}: {1}")]
    Convert(String, ConvertError),
    /// The ADL2 serializer refused a converted source.
    #[error("printing converted source {0:?}: {1}")]
    Print(String, openehr_adl::print::PrintError),
}

/// A parent → child slot-fill edge recovered from the flattened OPT: the
/// composition structure that flattening erased. `parent_path` is the RM path
/// within the parent archetype to the substituted slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FillEdge {
    pub(crate) parent_archetype_id: String,
    pub(crate) parent_path: String,
    pub(crate) slot_node_id: String,
    pub(crate) child_archetype_id: String,
}

/// One converted source archetype: the source archetype id it came from and its
/// ADL2 source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConvertedRoot {
    pub(crate) archetype_id: String,
    pub(crate) adl2: String,
}

/// The result of converting a stored 1.4 OPT: one ADL2 source per embedded
/// archetype root (root first, then in document order), plus the recovered
/// composition structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptConversion {
    pub(crate) roots: Vec<ConvertedRoot>,
    pub(crate) structure: Vec<FillEdge>,
}

/// Convert a parsed OPT-1.4 into one ADL2 source per embedded archetype root.
///
/// Each embedded `C_ARCHETYPE_ROOT` is decomposed into a scoped 1.4-shaped
/// `v2_4` source archetype and run through the `openehr_adl::adl14` converter
/// core; embedded children are replaced by open slots in their parent and the
/// fill edges recorded in [`OptConversion::structure`].
///
/// # Errors
/// - [`OptConvertError::Hrid`] if an embedded root's archetype id does not parse.
/// - [`OptConvertError::Convert`] if the converter rejects a decomposed source.
/// - [`OptConvertError::Print`] if a converted source carries a node the ADL2
///   serializer has no syntax for.
pub(crate) fn convert_opt_to_adl2(
    opt: &opt14::types::OperationalTemplate,
) -> Result<OptConversion, OptConvertError> {
    let (archetypes, structure) = convert_opt_to_archetypes(opt)?;
    let roots = archetypes
        .into_iter()
        .map(|(archetype_id, art)| {
            let adl2 = openehr_adl::print::print(&art)
                .map_err(|e| OptConvertError::Print(archetype_id.clone(), e))?;
            Ok(ConvertedRoot { archetype_id, adl2 })
        })
        .collect::<Result<Vec<ConvertedRoot>, OptConvertError>>()?;
    Ok(OptConversion { roots, structure })
}

/// One decomposed source per embedded OPT root, keyed by its archetype id.
pub(crate) type ConvertedRoots = Vec<(String, Archetype)>;

/// The minimal valid `RESOURCE_DESCRIPTION` for a decomposed OPT root: VARD
/// (`master03` §Validity Rules) requires a description to be specified; the
/// lifecycle state uses the 1.4→2 converter's own mapping (`unmanaged`).
/// The conversion-report entries land in `conversion_details` — the AOM2
/// `RESOURCE_DESCRIPTION` field for conversion provenance.
fn minimal_description(conversion_details: BTreeMap<String, String>) -> ResourceDescription {
    ResourceDescription {
        title: None,
        original_author: BTreeMap::new(),
        original_namespace: None,
        original_publisher: None,
        other_contributors: openehr_base::containers::present(Vec::new()),
        lifecycle_state: "unmanaged".to_owned(),
        custodian_namespace: None,
        custodian_organisation: None,
        copyright: None,
        licence: None,
        ip_acknowledgements: None,
        references: None,
        resource_package_uri: None,
        conversion_details: (!conversion_details.is_empty()).then_some(conversion_details),
        details: None,
        other_details: None,
    }
}

/// Decomposes the OPT and converts each embedded root, returning the converted
/// `v2_4` archetypes and the recovered fill structure.
///
/// # Errors
/// As [`convert_opt_to_adl2`].
pub(crate) fn convert_opt_to_archetypes(
    opt: &opt14::types::OperationalTemplate,
) -> Result<(ConvertedRoots, Vec<FillEdge>), OptConvertError> {
    let mut dx = Decomposer {
        language: opt.language.code_string.clone(),
        root_ontology: opt.ontology.as_ref(),
        component_ontologies: &opt.component_ontologies,
        units: Vec::new(),
        edges: Vec::new(),
    };
    // Unit 0 is the OPT's top root; nested roots are appended in document order
    // as they are discovered.
    dx.process_root(&opt.definition, "", true);

    let cfg = ConvertConfig {
        // NOTE: a flattened OPT inlines `-`-specialised roots standalone with
        // no resolvable lineage, so they emit unspecialised at depth 0 (no
        // openEHR spec governs 1.4→2 conversion — our own design).
        collapse_specialised_codes: true,
        ..ConvertConfig::default()
    };
    let mut archetypes = Vec::with_capacity(dx.units.len());
    for unit in dx.units {
        let mut log = ConversionLog::new();
        let hrid = parse_hrid(&unit.archetype_id)
            .map_err(|e| OptConvertError::Hrid(unit.archetype_id.clone(), e))?;
        let data = AuthoredArchetypeData {
            parent_archetype_id: None,
            archetype_id: hrid,
            is_differential: false,
            definition: unit.definition,
            terminology: Box::new(unit.terminology),
            rules: openehr_base::containers::present(Vec::new()),
            rm_overlay: None,
            uid: None,
            original_language: term_code("ISO_639-1", &dx.language),
            description: Some(Box::new(minimal_description(unit.notes))),
            is_controlled: None,
            annotations: None,
            translations: None,
            adl_version: Some("1.4".to_owned()),
            build_uid: Uuid::new(uuid::Uuid::nil()),
            rm_release: String::new(),
            is_generated: true,
            other_meta_data: BTreeMap::new(),
        };
        // NOTE: `is_differential`, `adl_version`, `rm_release`, `is_generated`
        // above are the converter's starting point only — `convert` overrides
        // them from the absent parent and from `ConvertConfig`.
        let art =
            Archetype::AuthoredArchetype(Box::new(AuthoredArchetype::AuthoredArchetype(data)));
        let converted = convert(&art, &cfg, &mut log)
            .map_err(|e| OptConvertError::Convert(unit.archetype_id.clone(), e))?;
        archetypes.push((unit.archetype_id, converted));
    }
    Ok((archetypes, dx.edges))
}

/// One decomposed archetype root awaiting conversion.
struct RawUnit {
    archetype_id: String,
    definition: CComplexObject,
    terminology: ArchetypeTerminology,
    /// Conversion-report entries for this root (dropped bindings, carried-URI
    /// notes, fallbacks) — landed in the converted archetype's
    /// `RESOURCE_DESCRIPTION.conversion_details` (the AOM2 home for conversion
    /// provenance; rendered by the ADL2 printer).
    notes: BTreeMap<String, String>,
}

/// Per-root working state threaded through the definition walk: the slot
/// at-code allocator, the ac-code allocator + the ac codes the root's
/// flattened ontology defines (for `CONSTRAINT_REF` resolution), the extra
/// terminology entries minted en route (reference-set ac definitions and
/// bindings), and the conversion-report notes.
struct RootCx {
    slot_num: i64,
    next_ac: i64,
    defined_acs: std::collections::BTreeSet<String>,
    extra_terms: Vec<ArchetypeTerm>,
    /// `(terminology-key, code, uri)` term-binding entries.
    extra_bindings: Vec<(String, String, String)>,
    notes: BTreeMap<String, String>,
}

impl RootCx {
    /// Mint a fresh 1.4-space ac code (the converter core shifts it like every
    /// other code, keeping the constraint and its terminology entry aligned).
    fn alloc_ac(&mut self) -> String {
        let code = format!("ac{:04}", self.next_ac);
        self.next_ac += 1;
        code
    }

    fn note(&mut self, key: impl Into<String>, message: impl Into<String>) {
        self.notes.insert(key.into(), message.into());
    }
}

struct Decomposer<'a> {
    language: String,
    root_ontology: Option<&'a opt14::types::FlatArchetypeOntology>,
    component_ontologies: &'a [opt14::types::FlatArchetypeOntology],
    units: Vec<RawUnit>,
    edges: Vec<FillEdge>,
}

impl Decomposer<'_> {
    /// Decompose one `C_ARCHETYPE_ROOT` into a scoped 1.4-shaped source archetype
    /// (pushed onto `units`), recursing into any embedded child roots. `is_top`
    /// selects the OPT's top-level `ontology` vs a `component_ontologies` entry
    /// for the archetype's flattened terminology.
    fn process_root(&mut self, root: &opt14::types::CArchetypeRoot, path: &str, is_top: bool) {
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
            mapped_children.push(self.map_object(c, archetype_id, &attr_path, cx));
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
fn merge_ontology(
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
fn drop_out_of_scope_bindings(
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
fn max_at_num(attrs: &[opt14::types::CAttribute]) -> i64 {
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

fn max_at_num_obj(obj: &opt14::types::CObject) -> i64 {
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
/// parser ([`openehr_adl::codes::parse_code`], the
/// `org.openehr.am.aom2.adl_code_definitions` leader + `.`-separated numeric
/// segments), so an at-code is recognised exactly as the rest of the ADL layer
/// recognises one.
fn at_num(node_id: &str) -> i64 {
    match openehr_adl::codes::parse_code(node_id) {
        Some(code) if code.prefix == openehr_adl::codes::CodePrefix::At => code
            .segments
            .first()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

// ── object constructors (the 1.4-shaped `v2_4` common fields) ────────────────

fn complex(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    attributes: Vec<CAttribute>,
) -> CObject {
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name: rm_type_name.to_owned(),
        occurrences: Some(map_mult(occurrences)),
        node_id: node_id.to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        attributes: openehr_base::containers::present(attributes),
        attribute_tuples: openehr_base::containers::present(Vec::new()),
    }))
}

fn terminology_code(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    constraint: String,
) -> CObject {
    CObject::CTerminologyCode(CTerminologyCode {
        parent: None,
        soc_parent: None,
        rm_type_name: rm_type_name.to_owned(),
        occurrences: Some(map_mult(occurrences)),
        node_id: node_id.to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint,
        constraint_status: None,
    })
}

/// Map a `C_PRIMITIVE_OBJECT` (its wrapped `C_PRIMITIVE`) to the matching `v2_4`
/// primitive constraint node. The converter passes primitive nodes through
/// untouched; phase-1 (no RM repo) does not validate primitive-constraint
/// internals, so a faithful-but-minimal mapping is sufficient and safe.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per primitive C_* struct literal — a flat mapping table"
)]
fn map_primitive_object(c: &opt14::types::CPrimitiveObject, cx: &mut RootCx) -> CObject {
    let rm = c.rm_type_name.as_str();
    let node_id = c.node_id.as_str();
    let occ = &c.occurrences;
    let Some(item) = c.item.as_deref() else {
        // A primitive object with no inner constraint → an unconstrained string.
        return c_string(rm, node_id, occ, Vec::new(), None);
    };
    match item {
        opt14::types::CPrimitive::CBoolean(p) => {
            let mut constraint = Vec::new();
            if p.true_valid {
                constraint.push(true);
            }
            if p.false_valid {
                constraint.push(false);
            }
            CObject::CBoolean(CBoolean {
                parent: None,
                soc_parent: None,
                rm_type_name: rm.to_owned(),
                occurrences: Some(map_mult(occ)),
                node_id: node_id.to_owned(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: p.assumed_value,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(constraint),
            })
        }
        opt14::types::CPrimitive::CString(p) => {
            let constraint = if !p.list.is_empty() {
                p.list.clone()
            } else if let Some(pat) = &p.pattern {
                vec![pat.clone()]
            } else {
                Vec::new()
            };
            c_string(rm, node_id, occ, constraint, p.assumed_value.clone())
        }
        opt14::types::CPrimitive::CInteger(p) => {
            let constraint = openehr_base::containers::present(
                p.range.as_ref().map(int_interval).into_iter().collect(),
            );
            CObject::CInteger(CInteger {
                parent: None,
                soc_parent: None,
                rm_type_name: rm.to_owned(),
                occurrences: Some(map_mult(occ)),
                node_id: node_id.to_owned(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: p.assumed_value.map(f64::from),
                is_enumerated_type_constraint: None,
                constraint,
            })
        }
        opt14::types::CPrimitive::CReal(p) => {
            let constraint = openehr_base::containers::present(
                p.range.as_ref().map(real_interval).into_iter().collect(),
            );
            CObject::CReal(CReal {
                parent: None,
                soc_parent: None,
                rm_type_name: rm.to_owned(),
                occurrences: Some(map_mult(occ)),
                node_id: node_id.to_owned(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: p.assumed_value,
                is_enumerated_type_constraint: None,
                constraint,
            })
        }
        opt14::types::CPrimitive::CDate(p) => CObject::CDate(CDate {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p.assumed_value.clone().map(|value| Iso8601Date { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601Date { value },
            )),
            pattern_constraint: date_time_pattern(
                p.pattern.clone(),
                p.range.is_some(),
                rm,
                node_id,
                cx,
            ),
        }),
        opt14::types::CPrimitive::CDateTime(p) => CObject::CDateTime(CDateTime {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p
                .assumed_value
                .clone()
                .map(|value| Iso8601DateTime { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601DateTime { value },
            )),
            pattern_constraint: date_time_pattern(
                p.pattern.clone(),
                p.range.is_some(),
                rm,
                node_id,
                cx,
            ),
        }),
        opt14::types::CPrimitive::CDuration(p) => CObject::CDuration(CDuration {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p
                .assumed_value
                .clone()
                .map(|value| Iso8601Duration { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601Duration { value },
            )),
            pattern_constraint: p.pattern.clone(),
        }),
        opt14::types::CPrimitive::CTime(p) => CObject::CTime(CTime {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p.assumed_value.clone().map(|value| Iso8601Time { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601Time { value },
            )),
            pattern_constraint: date_time_pattern(
                p.pattern.clone(),
                p.range.is_some(),
                rm,
                node_id,
                cx,
            ),
        }),
    }
}

/// Date/time/date-time constraints carry a pattern XOR a range on the ADL2
/// surface — the mixed `pattern/interval` form is defined for durations only
/// (`ADL2/master04.5-cadl_primitive_types.adoc` §Mixed Pattern and Interval
/// sits under Duration Constraints). When a 1.4 node carries both, the range
/// (the value constraint) is kept — the safe, still-narrowing half — and the
/// dropped format pattern reported in `conversion_details`.
fn date_time_pattern(
    pattern: Option<String>,
    has_range: bool,
    rm: &str,
    node_id: &str,
    cx: &mut RootCx,
) -> Option<String> {
    match pattern {
        Some(p) if has_range => {
            cx.note(
                format!("temporal_pattern.{rm}.{node_id}"),
                format!(
                    "{rm} node {node_id:?} carries both an ISO8601 pattern ({p}) and a range; \
                     ADL2 defines the combined form for durations only — the range was kept, \
                     the pattern is recorded here"
                ),
            );
            None
        }
        other => other,
    }
}

/// Build the `Interval<Iso8601_*>` constraint list from a 1.4 temporal range
/// (string bounds), mirroring [`int_interval`]'s point/proper split.
#[expect(
    clippy::type_complexity,
    reason = "one tuple threading the six ADL 1.4 interval facets through a \
              single private helper"
)]
fn temporal_interval<T>(
    range: Option<(
        Option<String>,
        Option<String>,
        bool,
        bool,
        Option<bool>,
        Option<bool>,
    )>,
    wrap: impl Fn(String) -> T,
) -> Vec<Interval<T>> {
    let Some((lower, upper, lower_unbounded, upper_unbounded, lower_included, upper_included)) =
        range
    else {
        return Vec::new();
    };
    if lower == upper && lower.is_some() {
        let v = lower.clone().unwrap_or_default();
        return vec![Interval::PointInterval(PointInterval {
            lower: lower.map(&wrap),
            upper: Some(wrap(v)),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        })];
    }
    vec![Interval::ProperInterval(ProperInterval::ProperInterval(
        ProperIntervalData {
            lower: lower.map(&wrap),
            upper: upper.map(&wrap),
            lower_unbounded,
            upper_unbounded,
            lower_included: lower_included.unwrap_or(!lower_unbounded),
            upper_included: upper_included.unwrap_or(!upper_unbounded),
        },
    ))]
}

fn c_string(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    constraint: Vec<String>,
    assumed_value: Option<String>,
) -> CObject {
    CObject::CString(CString {
        parent: None,
        soc_parent: None,
        rm_type_name: rm_type_name.to_owned(),
        occurrences: Some(map_mult(occurrences)),
        node_id: node_id.to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(constraint),
    })
}

// ── slot assertions ──────────────────────────────────────────────────────────

/// A `C_CODE_REFERENCE` names an external reference set by URI. With no
/// inline code list, that is exactly the AOM2 ac-code term-binding pattern:
/// an ac-code constraint whose binding URI "will designate a ref-set or value
/// set" (`AOM2/master07-terminology_package.adoc` §Overview; keys per VTCBK) —
/// a minted ac-code + definition + binding is emitted, and the converter core
/// shifts all three consistently. When an inline code list is ALSO present
/// the (more concrete) list constraint wins and the URI is carried in
/// `conversion_details` — a dual-constrained node has no single ADL2 form (no
/// openEHR spec governs 1.4→2 conversion — our own design).
fn map_code_reference(c: &opt14::types::CCodeReference, cx: &mut RootCx) -> CObject {
    if c.referenceSetUri.is_empty() || !c.code_list.is_empty() {
        if !c.referenceSetUri.is_empty() {
            cx.note(
                format!("reference_set_uri.{}", c.node_id),
                format!(
                    "node {} carries both a code list and referenceSetUri {}; \
                     the code list was converted, the URI is recorded here",
                    c.node_id, c.referenceSetUri
                ),
            );
        }
        return terminology_code(
            &c.rm_type_name,
            &c.node_id,
            &c.occurrences,
            code_constraint(
                c.terminology_id.as_ref().map(|t| t.value.as_str()),
                &c.code_list,
                c.assumed_value.as_ref().map(|a| a.code_string.as_str()),
            ),
        );
    }
    let ac = cx.alloc_ac();
    cx.extra_terms.push(ArchetypeTerm {
        code: ac.clone(),
        text: "Reference set".to_owned(),
        description: format!("External reference set {}", c.referenceSetUri),
        other_items: None,
    });
    // NOTE: the 1.4 `referenceSetUri` carries no terminology name, so an
    // undeclared one keys under a generic bucket (no openEHR spec governs the
    // key choice — our own design).
    let term_key = c
        .terminology_id
        .as_ref()
        .map_or_else(|| "external".to_owned(), |t| t.value.clone());
    cx.extra_bindings
        .push((term_key, ac.clone(), c.referenceSetUri.clone()));
    terminology_code(&c.rm_type_name, &c.node_id, &c.occurrences, ac)
}

/// The `include` assertion naming the archetype that filled a slot: the
/// canonical `archetype_id/value matches {/…/}` form
/// (`org.openehr.am.aom2.archetype_slot.adoc`), built through the BEL slot
/// parser so the tree matches what parsing the printed ADL2 would yield.
/// A parse failure (unreachable for a valid `ARCHETYPE_HRID`) degrades to an
/// open slot, reported — the fill edge in [`OptConversion::structure`] still
/// carries the identity.
fn archetype_id_include(
    child_archetype_id: &str,
    slot_node_id: &str,
    cx: &mut RootCx,
) -> Vec<Assertion> {
    let text = format!(
        "archetype_id/value matches {{/{}/}}",
        regex_escape(child_archetype_id)
    );
    match openehr_adl::rules::parse_slot_assertions(&text) {
        Ok(list) if !list.is_empty() => list,
        _ => {
            cx.note(
                format!("slot_assertion.{slot_node_id}.include.fill"),
                format!(
                    "the include naming filled archetype {child_archetype_id} did not parse; \
                     the slot is emitted open (the fill edge still records the identity)"
                ),
            );
            Vec::new()
        }
    }
}

/// Escape an archetype id for use inside a slot-assertion regex (`.` is the
/// only regex metacharacter a valid `ARCHETYPE_HRID` contains).
fn regex_escape(id: &str) -> String {
    id.replace('.', "\\.")
}

/// Map the 1.4 slot `ASSERTION`s (`AOM1.4/master05-assertion_package.adoc`
/// expression trees) onto AOM2 `beom` assertions: the verbatim
/// `string_expression` is preferred (both generations share the
/// `archetype_id/value matches {/…/}` surface syntax), else the expression
/// tree is rendered to that syntax; the result re-parses through the BEL slot
/// parser. An assertion that cannot be rendered or parsed is dropped —
/// reported in `conversion_details` so the slot's weakening is visible.
fn map_slot_assertions(
    assertions: &[opt14::types::Assertion],
    node_id: &str,
    kind: &str,
    cx: &mut RootCx,
) -> Vec<Assertion> {
    let mut out = Vec::new();
    for (idx, a) in assertions.iter().enumerate() {
        let text = a
            .string_expression
            .clone()
            .or_else(|| render_expr(&a.expression));
        let parsed = text
            .as_deref()
            .and_then(|t| openehr_adl::rules::parse_slot_assertions(t).ok());
        match parsed {
            Some(mut list) if !list.is_empty() => out.append(&mut list),
            _ => {
                cx.note(
                    format!("slot_assertion.{node_id}.{kind}.{idx}"),
                    format!(
                        "slot {node_id} {kind} assertion could not be converted \
                         (source: {:?}); the slot is emitted without it",
                        text.unwrap_or_else(|| "unrenderable expression tree".to_owned())
                    ),
                );
            }
        }
    }
    out
}

/// Render a 1.4 `EXPR_ITEM` tree to the ADL slot-assertion surface syntax.
/// Returns `None` for a shape outside the supported forms (binary/unary
/// operators over attribute paths, constants and constraint leaves).
fn render_expr(e: &opt14::types::ExprItem) -> Option<String> {
    match e {
        opt14::types::ExprItem::ExprBinaryOperator(b) => {
            let l = render_expr(&b.left_operand)?;
            let r = render_expr(&b.right_operand)?;
            let op = binary_symbol(b.operator)?;
            Some(format!("{l} {op} {r}"))
        }
        opt14::types::ExprItem::ExprUnaryOperator(u) => {
            let inner = render_expr(&u.operand)?;
            let op = unary_symbol(u.operator)?;
            Some(format!("{op} ({inner})"))
        }
        opt14::types::ExprItem::ExprLeaf(leaf) => render_leaf(leaf),
    }
}

/// The ADL surface symbol of a binary `OPERATOR_KIND`, or `None` for a kind
/// with no infix rendering.
///
/// The relational, arithmetic and logical renderings are the textual column of
/// `LANG/docs/BEL/master03-language.adoc` §Operators; `matches` is the
/// archetype-slot constraint operator
/// (`AM/docs/ADL2/master07.10-adl_definition.adoc` §Archetype Slots). The two
/// quantifiers (`for_all`, `exists`) are not infix operators — their syntax is
/// `there_exists v : c | …` / `for_all v : c | …` (BEL master03 §Container
/// Operators) — so a tree that carries one as a binary operator has no surface
/// form here and is reported unconverted rather than rendered into text the
/// slot-assertion parser would refuse.
fn binary_symbol(op: opt14::types::OperatorKind) -> Option<&'static str> {
    Some(match op {
        opt14::types::OperatorKind::Equal => "=",
        opt14::types::OperatorKind::NotEqual => "!=",
        opt14::types::OperatorKind::LessThanOrEqual => "<=",
        opt14::types::OperatorKind::LessThan => "<",
        opt14::types::OperatorKind::GreaterThanOrEqual => ">=",
        opt14::types::OperatorKind::GreaterThan => ">",
        opt14::types::OperatorKind::Matches => "matches",
        opt14::types::OperatorKind::And => "and",
        opt14::types::OperatorKind::Or => "or",
        opt14::types::OperatorKind::Xor => "xor",
        opt14::types::OperatorKind::Implies => "implies",
        opt14::types::OperatorKind::Plus => "+",
        opt14::types::OperatorKind::Minus => "-",
        opt14::types::OperatorKind::Multiply => "*",
        opt14::types::OperatorKind::Divide => "/",
        opt14::types::OperatorKind::Exponent => "^",
        opt14::types::OperatorKind::Not
        | opt14::types::OperatorKind::ForAll
        | opt14::types::OperatorKind::Exists => return None,
    })
}

/// The ADL surface symbol of a prefix `OPERATOR_KIND`, or `None` for a kind
/// with no prefix rendering.
///
/// `not` "can be applied as a prefix operator to all operators returning a
/// Boolean result as well as a parenthesised Boolean expression"
/// (`LANG/docs/BEL/master03-language.adoc` §Logical Negation); the unary
/// arithmetic signs are the same chapter's §Operators table.
fn unary_symbol(op: opt14::types::OperatorKind) -> Option<&'static str> {
    match op {
        opt14::types::OperatorKind::Not => Some("not"),
        opt14::types::OperatorKind::Plus => Some("+"),
        opt14::types::OperatorKind::Minus => Some("-"),
        _ => None,
    }
}

/// Render a 1.4 `EXPR_LEAF`: an attribute path verbatim, a constant literal,
/// or a constraint (`C_STRING` pattern/list) as the `{/…/}`/`{"…"}` block.
fn render_leaf(leaf: &opt14::types::ExprLeaf) -> Option<String> {
    let item = &leaf.item;
    let text = item.text();
    match leaf.reference_type.to_lowercase().as_str() {
        "attribute" => (!text.is_empty()).then_some(text),
        "constant" => {
            if text.is_empty() {
                None
            } else if item.xsi_type() == Some("string") || text.trim().parse::<i64>().is_err() {
                Some(format!("{text:?}"))
            } else {
                Some(text.trim().to_owned())
            }
        }
        "constraint" => {
            // The XML leaf item is a C_STRING: a regex `pattern` or a literal
            // string list.
            if let Some(p) = item.child("pattern") {
                let p = p.text();
                let body = p.trim_matches('/');
                return Some(format!("{{/{body}/}}"));
            }
            let items: Vec<String> = item
                .children_named("list")
                .map(|v| format!("{:?}", v.text()))
                .collect();
            (!items.is_empty()).then(|| format!("{{{}}}", items.join(", ")))
        }
        _ => None,
    }
}

// ── domain-type attribute tuples ─────────────────────────────────────────────

/// The `C_ATTRIBUTE` definition member of an attribute tuple (empty children —
/// the constraints live in the tuple rows).
fn tuple_member(rm_attribute_name: &str) -> CAttribute {
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name: rm_attribute_name.to_owned(),
        existence: None,
        children: openehr_base::containers::present(Vec::new()),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// A tuple-row `C_INTEGER` member (point value, or unconstrained when `None`).
fn tuple_integer(value: Option<i32>, range: Option<Interval<i32>>) -> CPrimitiveObject {
    CPrimitiveObject::CInteger(CInteger {
        parent: None,
        soc_parent: None,
        rm_type_name: "Integer".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(
            value
                .map(|v| {
                    Interval::PointInterval(PointInterval {
                        lower: Some(v),
                        upper: Some(v),
                        lower_unbounded: false,
                        upper_unbounded: false,
                        lower_included: true,
                        upper_included: true,
                    })
                })
                .into_iter()
                .chain(range)
                .collect::<Vec<_>>(),
        ),
    })
}

/// A tuple-row `C_REAL` member (range, or unconstrained when `None`).
fn tuple_real(range: Option<Interval<f64>>) -> CPrimitiveObject {
    CPrimitiveObject::CReal(CReal {
        parent: None,
        soc_parent: None,
        rm_type_name: "Real".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(range.into_iter().collect()),
    })
}

/// A tuple-row `C_STRING` member (single-value list).
fn tuple_string(value: &str) -> CPrimitiveObject {
    CPrimitiveObject::CString(CString {
        parent: None,
        soc_parent: None,
        rm_type_name: "String".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: Some(vec![value.to_owned()]),
    })
}

/// A tuple-row `C_TERMINOLOGY_CODE` member carrying the 1.4
/// `terminology::code` encoding the converter core rewrites (shifting local
/// at-codes, minting external ones).
fn tuple_code(terminology: Option<&str>, code: &str) -> CPrimitiveObject {
    CPrimitiveObject::CTerminologyCode(CTerminologyCode {
        parent: None,
        soc_parent: None,
        rm_type_name: "CODE_PHRASE".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: code_constraint(terminology, std::slice::from_ref(&code.to_owned()), None),
        constraint_status: None,
    })
}

/// `DV_STATE`: the 1.4 constrainer carries a state machine; the vendored AM
/// defines no ADL2/AOM2 constraint form for it (the tuple mechanism covers
/// co-varying attribute values, not state machines) — a loose domain-typed
/// complex object is the honest valid constraint. No vendored openEHR spec
/// governs `DV_STATE` conversion — our own design; recorded in
/// `conversion_details`.
fn dv_state_loose(c: &opt14::types::CDvState, cx: &mut RootCx) -> CObject {
    cx.note(
        format!("dv_state.{}", c.node_id),
        format!(
            "C_DV_STATE {} state-machine constraint has no ADL2 form; emitted as an \
             unconstrained DV_STATE node",
            c.node_id
        ),
    );
    complex(&c.rm_type_name, &c.node_id, &c.occurrences, Vec::new())
}

/// A `CONSTRAINT_REF` names an ac-code whose definition lives in the
/// flattened ontology's `constraint_definitions` (AOM 1.4
/// `constraint_ref.adoc`); ADL2 folds those into
/// `term_definitions`/`term_bindings` (`master07.13` §Terminology section),
/// so the ac-code constraint resolves (VACDF/VTCBK). An ac-code the ontology
/// does NOT define would dangle — it stays an unconstrained node, reported in
/// `conversion_details`.
fn map_constraint_ref(r: &opt14::types::ConstraintRef, cx: &mut RootCx) -> CObject {
    if cx.defined_acs.contains(&r.reference) {
        terminology_code(
            &r.rm_type_name,
            &r.node_id,
            &r.occurrences,
            r.reference.clone(),
        )
    } else {
        cx.note(
            format!("constraint_ref.{}", r.node_id),
            format!(
                "CONSTRAINT_REF {} names {} which the flattened ontology does not define; \
                 emitted unconstrained",
                r.node_id, r.reference
            ),
        );
        terminology_code(&r.rm_type_name, &r.node_id, &r.occurrences, String::new())
    }
}

/// `C_DV_ORDINAL` → the AOM2 `[value, symbol]` attribute tuple
/// (`master04.4-cadl_second_order.adoc` §Tuple Constraints). A 1.4
/// `assumed_value` has no per-tuple AOM2 slot — recorded in
/// `conversion_details`.
fn ordinal_tuple(c: &opt14::types::CDvOrdinal, cx: &mut RootCx) -> CObject {
    if let Some(a) = &c.assumed_value {
        cx.note(
            format!("assumed_value.{}", c.node_id),
            format!(
                "C_DV_ORDINAL {} assumed_value (ordinal {}) has no AOM2 tuple representation; \
                 recorded here",
                c.node_id, a.value
            ),
        );
    }
    let tuples = c
        .list
        .iter()
        .map(|o| CPrimitiveTuple {
            // A `[value, symbol]` ordinal row always has two members, so the
            // `1..*` bound of `C_PRIMITIVE_TUPLE.members` holds by construction.
            members: {
                let mut row =
                    openehr_base::containers::NonEmptyVec::of(tuple_integer(Some(o.value), None));
                row.push(tuple_code(
                    Some(o.symbol.defining_code.terminology_id.value.as_str()),
                    &o.symbol.defining_code.code_string,
                ));
                row
            },
        })
        .collect::<Vec<_>>();
    let attribute_tuples = if tuples.is_empty() {
        Vec::new()
    } else {
        vec![CAttributeTuple {
            members: Some(vec![tuple_member("value"), tuple_member("symbol")]),
            tuples: openehr_base::containers::present(tuples),
        }]
    };
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name: c.rm_type_name.clone(),
        occurrences: Some(map_mult(&c.occurrences)),
        node_id: c.node_id.clone(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        attributes: openehr_base::containers::present(Vec::new()),
        attribute_tuples: openehr_base::containers::present(attribute_tuples),
    }))
}

/// `C_DV_QUANTITY` → a `property` terminology-code attribute plus the
/// `[units, magnitude(, precision)]` attribute tuple (`master04.4` §Tuple
/// Constraints). The precision member is included only when some item
/// constrains it (uniform tuple arity; rows without one carry an
/// unconstrained `C_INTEGER`). A 1.4 `assumed_value` has no per-tuple AOM2
/// slot — recorded in `conversion_details`.
fn quantity_tuple(c: &opt14::types::CDvQuantity, cx: &mut RootCx) -> CObject {
    if c.assumed_value.is_some() {
        cx.note(
            format!("assumed_value.{}", c.node_id),
            format!(
                "C_DV_QUANTITY {} assumed_value has no AOM2 tuple representation; recorded here",
                c.node_id
            ),
        );
    }
    let mut attributes = Vec::new();
    if let Some(p) = &c.property {
        attributes.push(CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: "property".to_owned(),
            existence: None,
            children: Some(vec![terminology_code(
                "CODE_PHRASE",
                "",
                &unbounded_occurrences(),
                code_constraint(
                    Some(p.terminology_id.value.as_str()),
                    std::slice::from_ref(&p.code_string),
                    None,
                ),
            )]),
            differential_path: None,
            cardinality: None,
            is_multiple: false,
        });
    }
    // Tuple arity must be uniform, so a tuple is emitted only when EVERY item
    // constrains the magnitude; a mixed set widens to the plain units list,
    // which never rejects valid data (no openEHR spec governs 1.4→2 conversion
    // — our own design).
    let all_magnitude = !c.list.is_empty() && c.list.iter().all(|i| i.magnitude.is_some());
    let all_precision = !c.list.is_empty() && c.list.iter().all(|i| i.precision.is_some());
    let some_dropped = c
        .list
        .iter()
        .any(|i| i.magnitude.is_some() || i.precision.is_some());
    let mut attribute_tuples = Vec::new();
    if all_magnitude {
        if !all_precision && c.list.iter().any(|i| i.precision.is_some()) {
            cx.note(
                format!("quantity_precision.{}", c.node_id),
                format!(
                    "C_DV_QUANTITY {}: precision constrained on only some units; widened \
                     (dropped) to keep tuple arity uniform",
                    c.node_id
                ),
            );
        }
        let tuples = c
            .list
            .iter()
            .map(|item| {
                // A `[units, magnitude]` (optionally `+ precision`) row always
                // has at least two members, so the `1..*` bound of
                // `C_PRIMITIVE_TUPLE.members` holds by construction.
                let mut members =
                    openehr_base::containers::NonEmptyVec::of(tuple_string(&item.units));
                members.push(tuple_real(item.magnitude.as_ref().map(real_interval)));
                if all_precision {
                    members.push(tuple_integer(
                        None,
                        item.precision.as_ref().map(int_interval),
                    ));
                }
                CPrimitiveTuple { members }
            })
            .collect::<Vec<_>>();
        let mut members = vec![tuple_member("units"), tuple_member("magnitude")];
        if all_precision {
            members.push(tuple_member("precision"));
        }
        attribute_tuples.push(CAttributeTuple {
            members: openehr_base::containers::present(members),
            tuples: openehr_base::containers::present(tuples),
        });
    } else if !c.list.is_empty() {
        attributes.push(widened_units_attribute(c, some_dropped, cx));
    }
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name: c.rm_type_name.clone(),
        occurrences: Some(map_mult(&c.occurrences)),
        node_id: c.node_id.clone(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        attributes: openehr_base::containers::present(attributes),
        attribute_tuples: openehr_base::containers::present(attribute_tuples),
    }))
}

/// The plain `units` list attribute a magnitude-less (or mixed) quantity
/// widens to — the reference-corpus form for units-only constraints; a mixed
/// set's dropped per-unit ranges are reported.
fn widened_units_attribute(
    c: &opt14::types::CDvQuantity,
    some_dropped: bool,
    cx: &mut RootCx,
) -> CAttribute {
    if some_dropped {
        cx.note(
            format!("quantity_magnitude.{}", c.node_id),
            format!(
                "C_DV_QUANTITY {}: magnitude/precision constrained on only some units; widened \
                 to the plain units list (the dropped per-unit ranges are unrepresentable \
                 without uniform tuple arity)",
                c.node_id
            ),
        );
    }
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name: "units".to_owned(),
        existence: None,
        children: Some(vec![c_string(
            "String",
            "",
            &unbounded_occurrences(),
            c.list.iter().map(|i| i.units.clone()).collect(),
            None,
        )]),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// The `0..*` occurrences a synthesized single-child attribute carries (the
/// converter core elides RM-default multiplicities).
fn unbounded_occurrences() -> opt14::types::Intervalofinteger {
    opt14::types::Intervalofinteger {
        lower_included: Some(true),
        upper_included: None,
        lower_unbounded: false,
        upper_unbounded: true,
        lower: Some(0),
        upper: None,
    }
}

/// An AOM2 term-binding target must be a `Uri` (`AOM2/master07`
/// §`term_bindings`)
/// and the ADL2 ODIN target a URI token; a 1.4 binding value that is a bare
/// code is wrapped in the converter core's fabricated URN form
/// (`urn:adl14:<terminology>:<code>` — `adl14::convert::external_at_code`'s
/// fallback; no openEHR spec governs 1.4→2 conversion, our own design).
fn binding_uri(terminology: &str, value: &str) -> String {
    if value.contains("://") || value.starts_with("urn:") {
        value.to_owned()
    } else {
        format!("urn:adl14:{terminology}:{value}")
    }
}

/// The first numeric segment of an ac-code (`ac0007` → 7), for allocator
/// seeding.
fn first_ac_num(code: &str) -> Option<i64> {
    code.strip_prefix("ac")?
        .split('.')
        .next()?
        .parse::<i64>()
        .ok()
}

// ── value helpers ────────────────────────────────────────────────────────────

/// Build the 1.4 `C_TERMINOLOGY_CODE.constraint` encoding
/// (`terminology::code[,code…][;assumed]`) the converter rewrites. An empty code
/// list yields an empty (unconstrained) string.
fn code_constraint(terminology: Option<&str>, codes: &[String], assumed: Option<&str>) -> String {
    if codes.is_empty() {
        return String::new();
    }
    let term = terminology.unwrap_or("local");
    let mut out = format!("{term}::{}", codes.join(","));
    if let Some(a) = assumed {
        out.push(';');
        out.push_str(a);
    }
    out
}

fn map_term(t: &opt14::types::ArchetypeTerm) -> ArchetypeTerm {
    let text = t.items.get("text").cloned().unwrap_or_default();
    let description = t.items.get("description").cloned().unwrap_or_default();
    let other_items: BTreeMap<String, String> = t
        .items
        .iter()
        .filter(|(k, _)| k.as_str() != "text" && k.as_str() != "description")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // Keep the map absent rather than empty for the common no-extras case.
    let other_items = (!other_items.is_empty()).then_some(other_items);
    ArchetypeTerm {
        code: t.code.clone(),
        text,
        description,
        other_items,
    }
}

fn map_mult(iv: &opt14::types::Intervalofinteger) -> MultiplicityInterval {
    MultiplicityInterval {
        lower: iv.lower,
        upper: iv.upper,
        lower_unbounded: iv.lower_unbounded,
        upper_unbounded: iv.upper_unbounded,
        lower_included: iv.lower_included.unwrap_or(!iv.lower_unbounded),
        upper_included: iv.upper_included.unwrap_or(!iv.upper_unbounded),
    }
}

fn map_cardinality(c: &opt14::types::Cardinality) -> Cardinality {
    Cardinality {
        interval: map_mult(&c.interval),
        is_ordered: c.is_ordered,
        is_unique: c.is_unique,
    }
}

fn int_interval(iv: &opt14::types::Intervalofinteger) -> Interval<i32> {
    if iv.lower == iv.upper && iv.lower.is_some() {
        let v = iv.lower.unwrap_or_default();
        return Interval::PointInterval(PointInterval {
            lower: Some(v),
            upper: Some(v),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        });
    }
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower: iv.lower,
        upper: iv.upper,
        lower_unbounded: iv.lower_unbounded,
        upper_unbounded: iv.upper_unbounded,
        lower_included: iv.lower_included.unwrap_or(!iv.lower_unbounded),
        upper_included: iv.upper_included.unwrap_or(!iv.upper_unbounded),
    }))
}

fn real_interval(iv: &opt14::types::Intervalofreal) -> Interval<f64> {
    if iv.lower == iv.upper && iv.lower.is_some() {
        let v = iv.lower.unwrap_or_default();
        return Interval::PointInterval(PointInterval {
            lower: Some(v),
            upper: Some(v),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        });
    }
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower: iv.lower,
        upper: iv.upper,
        lower_unbounded: iv.lower_unbounded,
        upper_unbounded: iv.upper_unbounded,
        lower_included: iv.lower_included.unwrap_or(!iv.lower_unbounded),
        upper_included: iv.upper_included.unwrap_or(!iv.upper_unbounded),
    }))
}

fn term_code(terminology_id: &str, code: &str) -> TerminologyCode {
    TerminologyCode {
        terminology_id: terminology_id.to_owned(),
        terminology_version: None,
        code_string: code.to_owned(),
        uri: None,
    }
}

#[cfg(test)]
mod tests {
    use openehr_adl::parse::Dialect;
    use openehr_adl::validate::catalogue::Severity;
    use openehr_adl::validate::validate_integrity;

    use super::*;

    /// Paths under `corpus/fixtures/service`, the vendored OPT corpus of real
    /// Ocean/EHRbase-generated operational templates. One minimal
    /// COMPOSITION+OBSERVATION, one minimal EVALUATION
    /// (carries a `C_DV_QUANTITY`), and a large multi-archetype template
    /// (`Vital Signs`: 12 embedded roots, ordinals/quantities/integers/reals/
    /// strings/booleans/code phrases).
    const MINIMAL_OBSERVATION: &str = "knowledge/opt/minimal_observation.opt";
    const MINIMAL_EVALUATION: &str = "knowledge/opt/minimal_evaluation.opt";
    const VITAL_SIGNS: &str = "knowledge/opt/Vital Signs Encounter (Composition).opt";

    fn parse_opt(rel: &str) -> opt14::types::OperationalTemplate {
        let path = format!(
            "{}/../../corpus/fixtures/service/{rel}",
            env!("CARGO_MANIFEST_DIR")
        );
        let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse OPT {rel}: {e:?}"))
    }

    /// Every converted root converts, is a plain authored archetype, and passes
    /// the AOM2 phase-1 catalogue clean — the converter's own oracle property
    /// (`crates/openehr-adl/tests/adl14_conversion.rs` `assert_structural_match`
    /// runs the same `validate_integrity(&got, None)` gate).
    fn assert_converts_clean(rel: &str, min_roots: usize) {
        let opt = parse_opt(rel);
        let (archetypes, structure) =
            convert_opt_to_archetypes(&opt).unwrap_or_else(|e| panic!("convert {rel}: {e}"));
        assert!(
            archetypes.len() >= min_roots,
            "{rel}: expected >= {min_roots} converted roots, got {}",
            archetypes.len()
        );
        for (id, art) in &archetypes {
            assert!(
                matches!(art, Archetype::AuthoredArchetype(_)),
                "{rel}/{id}: not a plain authored archetype"
            );
            let errors: Vec<&str> = validate_integrity(art, None)
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .map(|i| i.code.mnemonic())
                .collect();
            assert!(
                errors.is_empty(),
                "{rel}/{id}: converted output failed phase-1 validation: {errors:?}"
            );
        }
        // Every recorded fill edge names a real parent/child archetype id.
        let ids: std::collections::BTreeSet<&str> =
            archetypes.iter().map(|(id, _)| id.as_str()).collect();
        for edge in &structure {
            assert!(
                ids.contains(edge.parent_archetype_id.as_str()),
                "{rel}: fill edge parent {:?} is not a converted root",
                edge.parent_archetype_id
            );
            assert!(
                ids.contains(edge.child_archetype_id.as_str()),
                "{rel}: fill edge child {:?} is not a converted root",
                edge.child_archetype_id
            );
        }
    }

    #[test]
    fn minimal_observation_converts_clean() {
        // COMPOSITION root + one embedded OBSERVATION root = 2 sources, 1 edge.
        assert_converts_clean(MINIMAL_OBSERVATION, 2);
    }

    #[test]
    fn minimal_evaluation_converts_clean() {
        assert_converts_clean(MINIMAL_EVALUATION, 2);
    }

    #[test]
    fn vital_signs_converts_clean() {
        // A large multi-archetype template: 12 embedded C_ARCHETYPE_ROOTs, so
        // the COMPOSITION root plus its components.
        assert_converts_clean(VITAL_SIGNS, 2);
    }

    /// Two full conversions of the same OPT yield identical archetypes (each
    /// unit converts under a fresh, deterministic log) — the converter's
    /// idempotency oracle, at the OPT-front-end level.
    #[test]
    fn conversion_is_deterministic() {
        let opt = parse_opt(MINIMAL_OBSERVATION);
        let (first, first_edges) = convert_opt_to_archetypes(&opt).expect("first convert");
        let (second, second_edges) = convert_opt_to_archetypes(&opt).expect("second convert");
        assert_eq!(first, second, "OPT conversion is not deterministic");
        assert_eq!(
            first_edges, second_edges,
            "fill structure is not deterministic"
        );
    }

    /// EVERY vendored OPT converts phase-1-clean and its printed ADL2
    /// re-parses — the whole-corpus fidelity gate (exercises default values,
    /// slot assertions, tuples, bindings across the real template set).
    #[test]
    fn whole_opt_corpus_converts_and_reparses() {
        let dir = format!(
            "{}/../../corpus/fixtures/service/knowledge/opt",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut count = 0usize;
        for entry in std::fs::read_dir(&dir).expect("opt corpus dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("opt") {
                continue;
            }
            count += 1;
            let name = path.display().to_string();
            let xml = std::fs::read_to_string(&path).expect("read opt");
            let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse {name}: {e:?}"));
            let (archetypes, _) =
                convert_opt_to_archetypes(&opt).unwrap_or_else(|e| panic!("convert {name}: {e}"));
            for (id, art) in &archetypes {
                let errors: Vec<&str> = validate_integrity(art, None)
                    .iter()
                    .filter(|i| i.severity == Severity::Error)
                    .map(|i| i.code.mnemonic())
                    .collect();
                // No adjudicated exceptions: every decomposed source is
                // phase-1 clean — specialised embedded roots collapse to
                // depth-0 emission and reused 1.4 node codes re-mint
                // archetype-wide-unique ids in the converter core.
                assert!(errors.is_empty(), "{name}/{id}: phase-1 errors: {errors:?}");
                let printed = openehr_adl::print::print(art)
                    .unwrap_or_else(|e| panic!("{name}/{id}: printing refused: {e}"));
                openehr_adl::assemble::parse_artefact(&printed, Dialect::Adl2).unwrap_or_else(
                    |e| panic!("{name}/{id}: printed ADL2 does not re-parse: {e:?}\n{printed}"),
                );
            }
        }
        assert!(count >= 3, "the OPT corpus went missing ({count} files)");
    }

    /// Every converted root prints to ADL2 text that re-parses — the printed
    /// surface (tuples, slot assertions, defaults) is parser-valid, not just
    /// object-model-valid.
    #[test]
    fn printed_adl2_reparses() {
        for rel in [MINIMAL_OBSERVATION, MINIMAL_EVALUATION, VITAL_SIGNS] {
            let opt = parse_opt(rel);
            let conversion = convert_opt_to_adl2(&opt).expect("convert");
            for root in &conversion.roots {
                openehr_adl::assemble::parse_artefact(&root.adl2, Dialect::Adl2).unwrap_or_else(
                    |e| {
                        let line = e.first().map_or(0, |err| err.line);
                        let context = root
                            .adl2
                            .lines()
                            .enumerate()
                            .skip(line.saturating_sub(4))
                            .take(7)
                            .map(|(i, l)| format!("{:>4} | {l}", i + 1))
                            .collect::<Vec<_>>()
                            .join("\n");
                        panic!(
                            "{rel}/{}: printed ADL2 does not re-parse: {e:?}\n{context}",
                            root.archetype_id
                        )
                    },
                );
            }
        }
    }

    /// A filled slot carries the canonical include assertion naming the child
    /// (`archetype_id/value matches {/…/}` —
    /// `org.openehr.am.aom2.archetype_slot.adoc`), alongside the fill edge.
    #[test]
    fn filled_slots_carry_include_assertions() {
        let opt = parse_opt(MINIMAL_OBSERVATION);
        let conversion = convert_opt_to_adl2(&opt).expect("convert");
        let composition = conversion
            .roots
            .iter()
            .find(|r| r.archetype_id == "openEHR-EHR-COMPOSITION.minimal.v1")
            .expect("composition root");
        assert!(
            composition.adl2.contains("include"),
            "no include assertion in the filled slot:\n{}",
            composition.adl2
        );
        assert!(
            composition
                .adl2
                .contains("openEHR-EHR-OBSERVATION\\.minimal\\.v1"),
            "the include does not name the filling archetype:\n{}",
            composition.adl2
        );
    }

    /// The `C_DV_ORDINAL` domain constrainer becomes the AOM2
    /// `[value, symbol]` attribute tuple (`master04.4-cadl_second_order.adoc`
    /// §Tuple Constraints) — visible on the printed ADL2 surface of the
    /// Vital Signs conversion. (Its quantities are magnitude-less, so they
    /// widen to plain `units` lists — the reference-corpus form; the tuple
    /// path is pinned by [`quantity_tuple_emitted_when_magnitudes_present`].)
    #[test]
    fn domain_types_become_attribute_tuples() {
        let opt = parse_opt(VITAL_SIGNS);
        let conversion = convert_opt_to_adl2(&opt).expect("convert");
        let all = conversion
            .roots
            .iter()
            .map(|r| r.adl2.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("[value, symbol]"),
            "no ordinal tuple emitted anywhere in the Vital Signs conversion"
        );
        assert!(
            all.contains("[units, magnitude, precision]"),
            "the fully-constrained quantities must carry [units, magnitude, precision] tuples"
        );
    }

    /// `C_DV_QUANTITY` with per-unit magnitudes → the `[units, magnitude]`
    /// tuple (`master04.4` §Tuple Constraints); with none → a plain `units`
    /// attribute (the reference-corpus form for units-only constraints).
    #[test]
    fn quantity_tuple_emitted_when_magnitudes_present() {
        let item = |units: &str, magnitude: Option<(f64, f64)>| opt14::types::CQuantityItem {
            magnitude: magnitude.map(|(lo, hi)| opt14::types::Intervalofreal {
                lower_included: Some(true),
                upper_included: Some(true),
                lower_unbounded: false,
                upper_unbounded: false,
                lower: Some(lo),
                upper: Some(hi),
            }),
            precision: None,
            units: units.to_owned(),
        };
        let quantity = |list: Vec<opt14::types::CQuantityItem>| opt14::types::CDvQuantity {
            rm_type_name: "DV_QUANTITY".to_owned(),
            occurrences: unbounded_occurrences(),
            node_id: "at0004".to_owned(),
            assumed_value: None,
            property: None,
            list,
        };

        let mut cx = test_cx();
        let constrained = quantity_tuple(
            &quantity(vec![
                item("deg C", Some((0.0, 100.0))),
                item("deg F", Some((32.0, 212.0))),
            ]),
            &mut cx,
        );
        let CObject::CComplexObject(CComplexObject::CComplexObject(d)) = &constrained else {
            panic!("expected a complex object");
        };
        assert_eq!(
            d.attribute_tuples.as_ref().map_or(0, Vec::len),
            1,
            "one [units, magnitude] tuple"
        );
        assert_eq!(
            d.attribute_tuples
                .iter()
                .flatten()
                .next()
                .map(|t| t.tuples.as_ref().map_or(0, Vec::len)),
            Some(2),
            "one row per unit"
        );

        let widened = quantity_tuple(&quantity(vec![item("kg", None), item("mg", None)]), &mut cx);
        let CObject::CComplexObject(CComplexObject::CComplexObject(d)) = &widened else {
            panic!("expected a complex object");
        };
        assert!(
            d.attribute_tuples.as_ref().is_none_or(Vec::is_empty),
            "no tuple without magnitudes"
        );
        assert!(
            d.attributes
                .iter()
                .flatten()
                .any(|a| a.rm_attribute_name == "units"),
            "a plain units attribute instead"
        );
    }

    /// `map_code_reference` with a bare `referenceSetUri` mints the ac-code +
    /// definition + binding (`AOM2/master07-terminology_package.adoc`
    /// §Overview: an ac binding designates a ref-set / value set).
    #[test]
    fn reference_set_uri_becomes_ac_binding() {
        let mut cx = test_cx();
        let node = opt14::types::CCodeReference {
            rm_type_name: "CODE_PHRASE".to_owned(),
            occurrences: unbounded_occurrences(),
            node_id: "at0005".to_owned(),
            assumed_value: None,
            terminology_id: None,
            code_list: Vec::new(),
            referenceSetUri: "http://example.org/fhir/ValueSet/units".to_owned(),
        };
        let obj = map_code_reference(&node, &mut cx);
        let CObject::CTerminologyCode(tc) = obj else {
            panic!("expected a C_TERMINOLOGY_CODE, got {obj:?}");
        };
        assert_eq!(tc.constraint, "ac0001", "minted ac constraint");
        assert_eq!(cx.extra_terms.len(), 1, "one minted ac definition");
        assert_eq!(
            cx.extra_terms.first().map(|t| t.code.as_str()),
            Some("ac0001")
        );
        assert_eq!(
            cx.extra_bindings.first(),
            Some(&(
                "external".to_owned(),
                "ac0001".to_owned(),
                "http://example.org/fhir/ValueSet/units".to_owned()
            ))
        );
    }

    /// `map_constraint_ref`: a defined ac-code becomes the ac constraint; an
    /// undefined one stays unconstrained with a `conversion_details` report
    /// (never a dangling reference that would fail VACDF).
    #[test]
    fn constraint_ref_resolves_or_reports() {
        let mut cx = test_cx();
        cx.defined_acs.insert("ac0002".to_owned());
        let defined = opt14::types::ConstraintRef {
            rm_type_name: "CODE_PHRASE".to_owned(),
            occurrences: unbounded_occurrences(),
            node_id: "at0007".to_owned(),
            reference: "ac0002".to_owned(),
        };
        let CObject::CTerminologyCode(tc) = map_constraint_ref(&defined, &mut cx) else {
            panic!("expected a C_TERMINOLOGY_CODE");
        };
        assert_eq!(tc.constraint, "ac0002");
        assert!(cx.notes.is_empty(), "a resolved ref reports nothing");

        let dangling = opt14::types::ConstraintRef {
            reference: "ac0099".to_owned(),
            ..defined
        };
        let CObject::CTerminologyCode(tc) = map_constraint_ref(&dangling, &mut cx) else {
            panic!("expected a C_TERMINOLOGY_CODE");
        };
        assert!(
            tc.constraint.is_empty(),
            "a dangling ref stays unconstrained"
        );
        assert!(
            cx.notes.keys().any(|k| k.starts_with("constraint_ref.")),
            "the dangling ref must be reported: {:?}",
            cx.notes
        );
    }

    /// Temporal primitives carry BOTH the ISO8601 pattern and the range
    /// (`org.openehr.am.aom2.c_duration.adoc`: the combined `"PWD/|P0W..P50W|"`
    /// form), plus the assumed value.
    #[test]
    fn temporal_range_and_pattern_both_carried() {
        let node = opt14::types::CPrimitiveObject {
            rm_type_name: "DV_DURATION".to_owned(),
            occurrences: unbounded_occurrences(),
            node_id: String::new(),
            item: Some(Box::new(opt14::types::CPrimitive::CDuration(
                opt14::types::CDuration {
                    pattern: Some("PTH".to_owned()),
                    range: Some(opt14::types::Intervalofduration {
                        lower_included: Some(true),
                        upper_included: Some(true),
                        lower_unbounded: false,
                        upper_unbounded: false,
                        lower: Some("PT0H".to_owned()),
                        upper: Some("PT12H".to_owned()),
                    }),
                    assumed_value: Some("PT1H".to_owned()),
                },
            ))),
        };
        let mut cx = test_cx();
        let CObject::CDuration(d) = map_primitive_object(&node, &mut cx) else {
            panic!("expected a C_DURATION");
        };
        assert_eq!(d.pattern_constraint.as_deref(), Some("PTH"));
        assert_eq!(
            d.constraint.as_ref().map_or(0, Vec::len),
            1,
            "the range must be carried"
        );
        assert_eq!(d.assumed_value.map(|a| a.value), Some("PT1H".to_owned()));
        assert!(cx.notes.is_empty(), "durations carry both without a report");
    }

    /// A date carrying BOTH a pattern and a range keeps the range and drops
    /// the pattern with a report — the mixed `pattern/interval` ADL2 form is
    /// duration-only (`master04.5` §Mixed Pattern and Interval), so emitting
    /// it for a date would not re-parse.
    #[test]
    fn date_pattern_plus_range_keeps_range_and_reports() {
        let node = opt14::types::CPrimitiveObject {
            rm_type_name: "DV_DATE".to_owned(),
            occurrences: unbounded_occurrences(),
            node_id: String::new(),
            item: Some(Box::new(opt14::types::CPrimitive::CDate(
                opt14::types::CDate {
                    pattern: Some("yyyy-??-??".to_owned()),
                    timezone_validity: None,
                    range: Some(opt14::types::Intervalofdate {
                        lower_included: Some(true),
                        upper_included: Some(true),
                        lower_unbounded: false,
                        upper_unbounded: false,
                        lower: Some("2004-01-01".to_owned()),
                        upper: Some("2004-12-31".to_owned()),
                    }),
                    assumed_value: None,
                },
            ))),
        };
        let mut cx = test_cx();
        let CObject::CDate(d) = map_primitive_object(&node, &mut cx) else {
            panic!("expected a C_DATE");
        };
        assert!(d.pattern_constraint.is_none(), "the pattern must drop");
        assert_eq!(
            d.constraint.as_ref().map_or(0, Vec::len),
            1,
            "the range must be kept"
        );
        assert!(
            cx.notes.keys().any(|k| k.starts_with("temporal_pattern.")),
            "the dropped pattern must be reported: {:?}",
            cx.notes
        );
        // A pattern-only date keeps its pattern unreported.
        let pattern_only = opt14::types::CPrimitiveObject {
            item: Some(Box::new(opt14::types::CPrimitive::CDate(
                opt14::types::CDate {
                    pattern: Some("yyyy-??-??".to_owned()),
                    timezone_validity: None,
                    range: None,
                    assumed_value: None,
                },
            ))),
            ..node
        };
        let mut cx = test_cx();
        let CObject::CDate(d) = map_primitive_object(&pattern_only, &mut cx) else {
            panic!("expected a C_DATE");
        };
        assert_eq!(d.pattern_constraint.as_deref(), Some("yyyy-??-??"));
        assert!(cx.notes.is_empty());
    }

    /// Notes recorded during a walk land in the converted archetype's
    /// `RESOURCE_DESCRIPTION.conversion_details`.
    #[test]
    fn notes_land_in_conversion_details() {
        let mut notes = BTreeMap::new();
        notes.insert("k".to_owned(), "v".to_owned());
        let desc = minimal_description(notes);
        assert_eq!(
            desc.conversion_details
                .as_ref()
                .and_then(|m| m.get("k"))
                .map(String::as_str),
            Some("v")
        );
        assert!(
            minimal_description(BTreeMap::new())
                .conversion_details
                .is_none(),
            "an empty report stays absent"
        );
    }

    fn test_cx() -> RootCx {
        RootCx {
            slot_num: 1,
            next_ac: 1,
            defined_acs: std::collections::BTreeSet::new(),
            extra_terms: Vec::new(),
            extra_bindings: Vec::new(),
            notes: BTreeMap::new(),
        }
    }

    /// The embedded OBSERVATION root is decomposed into its own source and the
    /// COMPOSITION → OBSERVATION fill is recorded in the structure log.
    #[test]
    fn embedded_root_recorded_in_structure() {
        let opt = parse_opt(MINIMAL_OBSERVATION);
        let (archetypes, structure) = convert_opt_to_archetypes(&opt).expect("convert");
        assert!(
            archetypes
                .iter()
                .any(|(id, _)| id == "openEHR-EHR-OBSERVATION.minimal.v1"),
            "the embedded OBSERVATION was not decomposed into its own source"
        );
        assert!(
            structure.iter().any(|e| {
                e.parent_archetype_id == "openEHR-EHR-COMPOSITION.minimal.v1"
                    && e.child_archetype_id == "openEHR-EHR-OBSERVATION.minimal.v1"
            }),
            "the COMPOSITION → OBSERVATION fill edge was not recorded: {structure:?}"
        );
    }

    /// A rendered slot assertion is ADL surface syntax, i.e. it round-trips
    /// through the slot-assertion parser the converted archetype is read back
    /// with.
    ///
    /// The `OPERATOR_KIND` value the OPT carries is an XSD facet id (`2007` for
    /// `matches`), which is not an operator token in any grammar; only the
    /// textual rendering of `LANG/docs/BEL/master03-language.adoc` §Operators
    /// parses.
    #[test]
    fn rendered_operators_are_adl_surface_syntax() {
        let matches = binary_symbol(opt14::types::OperatorKind::Matches)
            .expect("`matches` has a surface rendering");
        let text =
            format!("archetype_id/value {matches} {{/openEHR-EHR-OBSERVATION\\.minimal\\.v1/}}");
        let parsed = openehr_adl::rules::parse_slot_assertions(&text)
            .unwrap_or_else(|e| panic!("rendered assertion {text:?} did not parse: {e:?}"));
        assert_eq!(parsed.len(), 1, "expected one assertion from {text:?}");

        // The facet id itself is not a token, so the pre-mapping rendering
        // could never have parsed.
        let raw = format!(
            "archetype_id/value {} {{/openEHR-EHR-OBSERVATION\\.minimal\\.v1/}}",
            opt14::types::OperatorKind::Matches.as_wire()
        );
        assert!(
            openehr_adl::rules::parse_slot_assertions(&raw).is_err(),
            "the raw XSD facet id must not be accepted as an operator"
        );
    }

    /// Every relational, arithmetic and logical `OPERATOR_KIND` renders to the
    /// textual form of `LANG/docs/BEL/master03-language.adoc` §Operators, and
    /// the two quantifiers — which have no infix syntax there — render to
    /// nothing rather than to unparseable text.
    #[test]
    fn every_infix_operator_kind_maps_to_its_textual_rendering() {
        for (kind, symbol) in [
            (opt14::types::OperatorKind::Equal, "="),
            (opt14::types::OperatorKind::NotEqual, "!="),
            (opt14::types::OperatorKind::LessThanOrEqual, "<="),
            (opt14::types::OperatorKind::LessThan, "<"),
            (opt14::types::OperatorKind::GreaterThanOrEqual, ">="),
            (opt14::types::OperatorKind::GreaterThan, ">"),
            (opt14::types::OperatorKind::Matches, "matches"),
            (opt14::types::OperatorKind::And, "and"),
            (opt14::types::OperatorKind::Or, "or"),
            (opt14::types::OperatorKind::Xor, "xor"),
            (opt14::types::OperatorKind::Implies, "implies"),
            (opt14::types::OperatorKind::Plus, "+"),
            (opt14::types::OperatorKind::Minus, "-"),
            (opt14::types::OperatorKind::Multiply, "*"),
            (opt14::types::OperatorKind::Divide, "/"),
            (opt14::types::OperatorKind::Exponent, "^"),
        ] {
            assert_eq!(binary_symbol(kind), Some(symbol), "{kind:?}");
        }
        for kind in [
            opt14::types::OperatorKind::ForAll,
            opt14::types::OperatorKind::Exists,
            opt14::types::OperatorKind::Not,
        ] {
            assert_eq!(binary_symbol(kind), None, "{kind:?}");
        }
        assert_eq!(
            unary_symbol(opt14::types::OperatorKind::Not),
            Some("not"),
            "`not` is the prefix negation of BEL master03 §Logical Negation"
        );
        assert_eq!(unary_symbol(opt14::types::OperatorKind::Matches), None);
    }
}
