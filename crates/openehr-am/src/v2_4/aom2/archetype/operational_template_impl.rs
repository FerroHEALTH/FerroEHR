// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `OPERATIONAL_TEMPLATE` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.operational_template.adoc`
//! §Attributes + §Functions.

use crate::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use crate::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

impl OperationalTemplate {
    /// Returns the terminology of the component archetype identified by `an_id`.
    ///
    /// `component_terminology` (`org.openehr.am.aom2.operational_template.adoc`
    /// §Functions) reads the `component_terminologies` table the same page
    /// declares, keyed by component archetype identifier. A template carrying no
    /// such component yields `None`.
    #[must_use]
    pub fn component_terminology(&self, an_id: &str) -> Option<&ArchetypeTerminology> {
        self.component_terminologies.as_ref()?.get(an_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
    use crate::v2_4::aom2::constraint_model::c_complex_object::{
        CComplexObject, CComplexObjectData,
    };
    use openehr_base::v1_3::base_types::definitions::version_status::VersionStatus;
    use openehr_base::v1_3::foundation_types::terminology::terminology_code::TerminologyCode;

    fn terminology(concept_code: &str) -> ArchetypeTerminology {
        ArchetypeTerminology {
            is_differential: false,
            original_language: "en".to_owned(),
            concept_code: concept_code.to_owned(),
            term_definitions: std::collections::BTreeMap::new(),
            term_bindings: None,
            value_sets: None,
            terminology_extracts: None,
        }
    }

    fn template(
        components: Option<std::collections::BTreeMap<String, ArchetypeTerminology>>,
    ) -> OperationalTemplate {
        OperationalTemplate {
            parent_archetype_id: None,
            archetype_id: ArchetypeHrid {
                namespace: None,
                rm_publisher: "openEHR".to_owned(),
                rm_package: "EHR".to_owned(),
                rm_class: "COMPOSITION".to_owned(),
                concept_id: "report".to_owned(),
                release_version: "1.0.0".to_owned(),
                version_status: VersionStatus::Released,
                build_count: "0".to_owned(),
            },
            is_differential: false,
            definition: CComplexObject::CComplexObject(CComplexObjectData {
                parent: None,
                soc_parent: None,
                rm_type_name: "COMPOSITION".to_owned(),
                occurrences: None,
                node_id: "id1".to_owned(),
                alternative_ids: None,
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                attributes: None,
                attribute_tuples: None,
            }),
            terminology: terminology("id1"),
            rules: None,
            rm_overlay: None,
            uid: None,
            original_language: TerminologyCode {
                terminology_id: "ISO_639-1".to_owned(),
                terminology_version: None,
                code_string: "en".to_owned(),
                uri: None,
            },
            description: None,
            is_controlled: None,
            annotations: None,
            translations: None,
            adl_version: None,
            build_uid: "9b4b5b6e-0000-4000-8000-000000000000"
                .parse()
                .expect("a literal v4 UUID should parse"),
            rm_release: "1.2.0".to_owned(),
            is_generated: true,
            other_meta_data: std::collections::BTreeMap::new(),
            component_terminologies: components,
            terminology_extracts: None,
        }
    }

    #[test]
    fn a_known_component_id_resolves_to_its_terminology() {
        let components = [(
            "openEHR-EHR-OBSERVATION.blood_pressure.v1".to_owned(),
            terminology("id7"),
        )]
        .into_iter()
        .collect();
        let t = template(Some(components));
        assert_eq!(
            t.component_terminology("openEHR-EHR-OBSERVATION.blood_pressure.v1")
                .map(|c| c.concept_code.as_str()),
            Some("id7")
        );
    }

    #[test]
    fn an_unknown_or_absent_component_yields_nothing() {
        let t = template(None);
        assert!(
            t.component_terminology("openEHR-EHR-OBSERVATION.bp.v1")
                .is_none()
        );
        let empty = template(Some(std::collections::BTreeMap::new()));
        assert!(empty.component_terminology("anything").is_none());
    }
}
