// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `ARCHETYPE` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.archetype.adoc` §Functions +
//! §Invariants, and `AM/docs/AOM1.4/master07-ontology_package.adoc`
//! §Specialisation Depth.

use crate::v1_4::aom14::archetype::archetype::Archetype;
use crate::v1_4::aom14::archetype::constraint_model::c_complex_object::CComplexObject;
use crate::v1_4::aom14::archetype::constraint_model::c_object::CObject;

impl Archetype {
    /// Returns the short concept name of this archetype.
    ///
    /// `short_concept_name` (`org.openehr.am.aom14.archetype.adoc` §Functions):
    /// "The short concept name of the archetype extracted from the
    /// `archetype_id`" — the domain-concept segment, which
    /// `AM/docs/Identification/master03-artefact_source_id.adoc` §Concept
    /// Identifier calls the "'short' ontological identifier (known in ADL 1.4
    /// as the 'concept' or 'domain concept')", as opposed to the language-bound
    /// rubric `concept_name` returns.
    #[must_use]
    pub fn short_concept_name(&self) -> &str {
        self.archetype_id.domain_concept()
    }

    /// Returns the version of this archetype.
    ///
    /// `version` (`org.openehr.am.aom14.archetype.adoc` §Functions) carries no
    /// meaning text; the same page's `Inv_version_validity` invariant defines
    /// it: `version /= Void and then version.is_equal(archetype_id.version_id)`.
    #[must_use]
    pub fn version(&self) -> &str {
        self.archetype_id.version_id()
    }

    /// Returns true if this archetype specialises another.
    ///
    /// `is_specialised` (`org.openehr.am.aom14.archetype.adoc` §Functions),
    /// post-condition `Result implies parent_archetype_id /= Void`.
    #[must_use]
    pub fn is_specialised(&self) -> bool {
        self.parent_archetype_id.is_some()
    }

    /// Returns the specialisation depth of this archetype.
    ///
    /// `specialisation_depth` (`org.openehr.am.aom14.archetype.adoc`
    /// §Functions), post-condition `Result = terminology.specialisation_depth`
    /// — the `ontology` association in AOM 1.4.
    #[must_use]
    pub fn specialisation_depth(&self) -> i32 {
        self.ontology.specialisation_depth
    }

    /// Returns the language-independent paths of this archetype's definition.
    ///
    /// `physical_paths` (`org.openehr.am.aom14.archetype.adoc` §Functions):
    /// "Paths obey Xpath-like syntax and are formed from alternations of
    /// `C_OBJECT.node_id` and `C_ATTRIBUTE.rm_attribute_name` values." A node
    /// whose `node_id` is empty contributes the attribute segment alone, since
    /// there is no code to predicate on.
    #[must_use]
    pub fn physical_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_paths(&self.definition, "", &mut out);
        out
    }

    /// Returns true if every node id in the definition is defined in the
    /// ontology.
    ///
    /// `node_ids_valid` (`org.openehr.am.aom14.archetype.adoc` §Functions):
    /// "True if every `node_id` found on a `C_OBJECT` node is found in
    /// `ontology.term_codes`." A node carrying no code has no `node_id` to
    /// check.
    #[must_use]
    pub fn node_ids_valid(&self) -> bool {
        let mut nodes = Vec::new();
        collect_objects(&self.definition, &mut nodes);
        nodes
            .iter()
            .map(|o| node_id(o))
            .filter(|id| !id.is_empty())
            .all(|id| self.ontology.has_term_code(id))
    }

    /// Returns true if every constraint reference in the definition is defined
    /// in the ontology.
    ///
    /// `constraint_references_valid` (`org.openehr.am.aom14.archetype.adoc`
    /// §Functions): "True if every `CONSTRAINT_REF.reference` found on a
    /// `C_OBJECT` node in the archetype definition is found in
    /// `ontology.constraint_codes`."
    #[must_use]
    pub fn constraint_references_valid(&self) -> bool {
        let mut nodes = Vec::new();
        collect_objects(&self.definition, &mut nodes);
        nodes.iter().all(|o| match o {
            CObject::ConstraintRef(r) => self.ontology.has_constraint_code(&r.reference),
            _ => true,
        })
    }

    /// Returns true if every internal reference targets a node that exists.
    ///
    /// `internal_references_valid` (`org.openehr.am.aom14.archetype.adoc`
    /// §Functions): "True if every `ARCHETYPE_INTERNAL_REF.target_path` refers
    /// to a legitimate node in the archetype definition" — checked against the
    /// path set `physical_paths` builds from the same definition.
    #[must_use]
    pub fn internal_references_valid(&self) -> bool {
        let paths = self.physical_paths();
        let mut nodes = Vec::new();
        collect_objects(&self.definition, &mut nodes);
        nodes.iter().all(|o| match o {
            CObject::ArchetypeInternalRef(r) => paths.contains(&r.target_path),
            _ => true,
        })
    }

    /// Returns true if this archetype passes the structural checks the spec
    /// names.
    ///
    /// `is_valid` (`org.openehr.am.aom14.archetype.adoc` §Functions),
    /// post-condition `not (node_ids_valid and internal_references_valid and
    /// constraint_references_valid) implies not Result`. The spec states only
    /// that necessary condition and adds "various tests should be used,
    /// including" those three, so this returns exactly their conjunction: the
    /// post-condition holds and no unstated test is invented.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.node_ids_valid()
            && self.internal_references_valid()
            && self.constraint_references_valid()
    }
}

/// The `node_id` an AOM 1.4 `C_OBJECT` carries, whichever concrete form it takes.
fn node_id(object: &CObject) -> &str {
    match object {
        CObject::ArchetypeInternalRef(o) => &o.node_id,
        CObject::ArchetypeSlot(o) => &o.node_id,
        CObject::ConstraintRef(o) => &o.node_id,
        CObject::CCodedText(o) => &o.node_id,
        CObject::CComplexObject(o) => &o.node_id,
        CObject::COrdinal(o) => &o.node_id,
        CObject::CPrimitiveObject(o) => &o.node_id,
        CObject::CQuantity(o) => &o.node_id,
    }
}

/// Appends every `C_OBJECT` below `root`, in definition order, to `out`.
fn collect_objects<'a>(root: &'a CComplexObject, out: &mut Vec<&'a CObject>) {
    for attribute in root.attributes.iter().flatten() {
        for child in attribute.children().into_iter().flatten() {
            out.push(child);
            if let CObject::CComplexObject(complex) = child {
                collect_objects(complex, out);
            }
        }
    }
}

/// Appends the physical path of every node below `root` (each prefixed by
/// `prefix`) to `out`.
fn collect_paths(root: &CComplexObject, prefix: &str, out: &mut Vec<String>) {
    for attribute in root.attributes.iter().flatten() {
        let attribute_path = format!("{prefix}/{}", attribute.rm_attribute_name());
        let children = attribute.children().unwrap_or_default();
        if children.is_empty() {
            out.push(attribute_path);
            continue;
        }
        for child in children {
            let id = node_id(child);
            let path = if id.is_empty() {
                attribute_path.clone()
            } else {
                format!("{attribute_path}[{id}]")
            };
            out.push(path.clone());
            if let CObject::CComplexObject(complex) = child {
                collect_paths(complex, &path, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_4::aom14::archetype::constraint_model::archetype_internal_ref::ArchetypeInternalRef;
    use crate::v1_4::aom14::archetype::constraint_model::c_attribute::CAttribute;
    use crate::v1_4::aom14::archetype::constraint_model::c_multiple_attribute::CMultipleAttribute;
    use crate::v1_4::aom14::archetype::constraint_model::cardinality::Cardinality;
    use crate::v1_4::aom14::archetype::constraint_model::constraint_ref::ConstraintRef;
    use crate::v1_4::aom14::archetype::ontology::archetype_ontology::ArchetypeOntology;
    use openehr_base::containers::NonEmptyVec;
    use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::proper_interval::{
        ProperInterval, ProperIntervalData,
    };
    use openehr_base::v1_3::foundation_types::terminology::terminology_code::TerminologyCode;

    fn zero_to_many() -> Interval<i32> {
        Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
            lower: Some(0),
            upper: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower_included: true,
            upper_included: false,
        }))
    }

    fn complex(node_id: &str, attributes: Option<Vec<CAttribute>>) -> CComplexObject {
        CComplexObject {
            rm_type_name: "ELEMENT".to_owned(),
            occurrences: zero_to_many(),
            node_id: node_id.to_owned(),
            assumed_value: None,
            attributes,
        }
    }

    fn items(children: Vec<CObject>) -> CAttribute {
        CAttribute::CMultipleAttribute(CMultipleAttribute {
            rm_attribute_name: "items".to_owned(),
            existence: zero_to_many(),
            children: Some(children),
            cardinality: Cardinality {
                interval: zero_to_many(),
                is_ordered: true,
                is_unique: false,
            },
        })
    }

    fn archetype(definition: CComplexObject, parent: Option<&str>) -> Archetype {
        Archetype {
            uid: None,
            original_language: TerminologyCode {
                terminology_id: "ISO_639-1".to_owned(),
                code_string: "en".to_owned(),
                terminology_version: None,
                uri: None,
            },
            description: None,
            is_controlled: None,
            annotations: None,
            translations: None,
            definition,
            ontology: Box::new(ArchetypeOntology {
                term_codes: NonEmptyVec::new(vec!["at0000".to_owned(), "at0001".to_owned()])
                    .expect("two term codes are a non-empty vector"),
                constraint_codes: NonEmptyVec::new(vec!["ac0001".to_owned()])
                    .expect("one constraint code is a non-empty vector"),
                terminologies_available: None,
                specialisation_depth: 0,
                term_attribute_names: NonEmptyVec::new(vec!["text".to_owned()])
                    .expect("one attribute name is a non-empty vector"),
            }),
            adl_version: None,
            archetype_id: ArchetypeId {
                value: "openEHR-EHR-OBSERVATION.blood_pressure.v1".to_owned(),
            },
            concept: "at0000".to_owned(),
            parent_archetype_id: parent.map(|p| ArchetypeId {
                value: p.to_owned(),
            }),
            invariants: None,
        }
    }

    #[test]
    fn the_identifier_supplies_the_short_concept_name_and_the_version() {
        let a = archetype(complex("at0000", None), None);
        assert_eq!(a.short_concept_name(), "blood_pressure");
        assert_eq!(a.version(), "1");
    }

    #[test]
    fn specialisation_is_the_presence_of_a_parent_identifier() {
        assert!(!archetype(complex("at0000", None), None).is_specialised());
        assert!(
            archetype(
                complex("at0000", None),
                Some("openEHR-EHR-OBSERVATION.bp.v1")
            )
            .is_specialised()
        );
    }

    #[test]
    fn the_depth_comes_from_the_ontology() {
        assert_eq!(
            archetype(complex("at0000", None), None).specialisation_depth(),
            0
        );
    }

    #[test]
    fn physical_paths_alternate_attribute_names_and_node_ids() {
        let definition = complex(
            "at0000",
            Some(vec![items(vec![CObject::CComplexObject(complex(
                "at0001",
                Some(vec![items(vec![CObject::CComplexObject(complex(
                    "", None,
                ))])]),
            ))])]),
        );
        let a = archetype(definition, None);
        assert_eq!(
            a.physical_paths(),
            vec![
                "/items[at0001]".to_owned(),
                "/items[at0001]/items".to_owned()
            ]
        );
    }

    #[test]
    fn a_node_id_absent_from_the_ontology_invalidates_the_definition() {
        let good = archetype(
            complex(
                "at0000",
                Some(vec![items(vec![CObject::CComplexObject(complex(
                    "at0001", None,
                ))])]),
            ),
            None,
        );
        assert!(good.node_ids_valid());
        let bad = archetype(
            complex(
                "at0000",
                Some(vec![items(vec![CObject::CComplexObject(complex(
                    "at0099", None,
                ))])]),
            ),
            None,
        );
        assert!(!bad.node_ids_valid());
    }

    #[test]
    fn a_constraint_reference_outside_the_ontology_invalidates_the_definition() {
        let reference = |code: &str| {
            CObject::ConstraintRef(ConstraintRef {
                rm_type_name: "DV_CODED_TEXT".to_owned(),
                occurrences: zero_to_many(),
                node_id: "at0001".to_owned(),
                reference: code.to_owned(),
            })
        };
        let good = archetype(
            complex("at0000", Some(vec![items(vec![reference("ac0001")])])),
            None,
        );
        assert!(good.constraint_references_valid());
        let bad = archetype(
            complex("at0000", Some(vec![items(vec![reference("ac0099")])])),
            None,
        );
        assert!(!bad.constraint_references_valid());
    }

    #[test]
    fn an_internal_reference_must_target_an_existing_path() {
        let internal_ref = |target: &str| {
            CObject::ArchetypeInternalRef(ArchetypeInternalRef {
                rm_type_name: "CLUSTER".to_owned(),
                occurrences: zero_to_many(),
                node_id: "at0001".to_owned(),
                target_path: target.to_owned(),
            })
        };
        let good = archetype(
            complex(
                "at0000",
                Some(vec![items(vec![internal_ref("/items[at0001]")])]),
            ),
            None,
        );
        assert!(good.internal_references_valid());
        let bad = archetype(
            complex(
                "at0000",
                Some(vec![items(vec![internal_ref("/items[at0099]")])]),
            ),
            None,
        );
        assert!(!bad.internal_references_valid());
    }

    #[test]
    fn overall_validity_fails_as_soon_as_one_named_check_fails() {
        let bad = archetype(
            complex(
                "at0000",
                Some(vec![items(vec![CObject::CComplexObject(complex(
                    "at0099", None,
                ))])]),
            ),
            None,
        );
        assert!(!bad.node_ids_valid());
        assert!(!bad.is_valid());
    }
}
