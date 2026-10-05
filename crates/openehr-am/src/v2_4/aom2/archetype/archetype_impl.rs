// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `ARCHETYPE` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.archetype.adoc` §Functions and
//! `AM/docs/AOM2/master07-terminology_package.adoc` §Specialisation Depth.

use crate::v2_4::aom2::archetype::archetype::Archetype;
use crate::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use crate::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use crate::v2_4::aom2::constraint_model::c_object::CObject;
use crate::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

impl Archetype {
    /// Returns the root node of this archetype's definition.
    #[must_use]
    pub fn definition(&self) -> &CComplexObject {
        match self {
            Self::AuthoredArchetype(a) => match a.as_ref() {
                AuthoredArchetype::OperationalTemplate(t) => &t.definition,
                AuthoredArchetype::Template(t) => &t.definition,
                AuthoredArchetype::AuthoredArchetype(d) => &d.definition,
            },
            Self::TemplateOverlay(o) => &o.definition,
        }
    }

    /// Returns this archetype's terminology.
    #[must_use]
    pub fn terminology(&self) -> &ArchetypeTerminology {
        match self {
            Self::AuthoredArchetype(a) => match a.as_ref() {
                AuthoredArchetype::OperationalTemplate(t) => &t.terminology,
                AuthoredArchetype::Template(t) => &t.terminology,
                AuthoredArchetype::AuthoredArchetype(d) => &d.terminology,
            },
            Self::TemplateOverlay(o) => &o.terminology,
        }
    }

    /// Returns the identifier of this archetype's specialisation parent, if any.
    #[must_use]
    pub fn parent_archetype_id(&self) -> Option<&str> {
        match self {
            Self::AuthoredArchetype(a) => match a.as_ref() {
                AuthoredArchetype::OperationalTemplate(t) => t.parent_archetype_id.as_deref(),
                AuthoredArchetype::Template(t) => t.parent_archetype_id.as_deref(),
                AuthoredArchetype::AuthoredArchetype(d) => d.parent_archetype_id.as_deref(),
            },
            Self::TemplateOverlay(o) => o.parent_archetype_id.as_deref(),
        }
    }

    /// Returns the concept code of this archetype's root object.
    ///
    /// `concept_code` (`org.openehr.am.aom2.archetype.adoc` §Functions),
    /// post-condition `Result.is_equal (definition.node_id)`.
    #[must_use]
    pub fn concept_code(&self) -> &str {
        self.definition().node_id()
    }

    /// Returns true if this archetype specialises another.
    ///
    /// `is_specialised` (`org.openehr.am.aom2.archetype.adoc` §Functions),
    /// post-condition `Result implies parent_archetype_hrid /= Void` — read
    /// against the `parent_archetype_id` attribute the same page declares.
    #[must_use]
    pub fn is_specialised(&self) -> bool {
        self.parent_archetype_id().is_some_and(|id| !id.is_empty())
    }

    /// Returns the specialisation depth of this archetype.
    ///
    /// `specialisation_depth` (`org.openehr.am.aom2.archetype.adoc` §Functions),
    /// post-condition `Result = terminology.specialisation_depth`.
    #[must_use]
    pub fn specialisation_depth(&self) -> i32 {
        self.terminology().specialisation_depth()
    }

    /// Returns the language-independent paths of this archetype's definition.
    ///
    /// `physical_paths` (`org.openehr.am.aom2.archetype.adoc` §Functions):
    /// "Paths obey Xpath-like syntax and are formed from alternations of
    /// `C_OBJECT.node_id` and `C_ATTRIBUTE.rm_attribute_name` values." A node
    /// whose `node_id` is empty contributes the attribute segment alone.
    #[must_use]
    pub fn physical_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_paths(self.definition(), "", &mut out);
        out
    }

    /// Returns the language-dependent paths of this archetype's definition.
    ///
    /// `logical_paths` (`org.openehr.am.aom2.archetype.adoc` §Functions): "the
    /// same syntax as `physical_paths`, but with `node_ids` replaced by their
    /// meanings from the terminology" — the meaning being the term's `text`
    /// rubric in `lang`. A code with no definition in `lang` keeps its code, so
    /// a partially translated terminology still yields a complete path set.
    #[must_use]
    pub fn logical_paths(&self, lang: &str) -> Vec<String> {
        let terminology = self.terminology();
        let mut out = Vec::new();
        collect_paths(self.definition(), "", &mut out);
        out.into_iter()
            .map(|path| translate(&path, lang, terminology))
            .collect()
    }
}

/// Replaces each `[code]` predicate in `path` with the code's rubric in `lang`.
fn translate(path: &str, lang: &str, terminology: &ArchetypeTerminology) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('[') {
        let Some((head, tail)) = rest.split_at_checked(open) else {
            break;
        };
        out.push_str(head);
        let Some((code, remainder)) = tail.trim_start_matches('[').split_once(']') else {
            out.push_str(tail);
            return out;
        };
        let meaning = terminology
            .term_definition(lang, code)
            .map_or(code, |term| term.text.as_str());
        out.push('[');
        out.push_str(meaning);
        out.push(']');
        rest = remainder;
    }
    out.push_str(rest);
    out
}

/// Appends the physical path of every node below `root` (each prefixed by
/// `prefix`) to `out`.
fn collect_paths(root: &CComplexObject, prefix: &str, out: &mut Vec<String>) {
    for attribute in root.attributes().unwrap_or_default() {
        let attribute_path = format!("{prefix}/{}", attribute.rm_attribute_name);
        let children = attribute.children.as_deref().unwrap_or_default();
        if children.is_empty() {
            out.push(attribute_path);
            continue;
        }
        for child in children {
            let id = child.node_id();
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
    use crate::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
    use crate::v2_4::aom2::archetype::authored_archetype::AuthoredArchetypeData;
    use crate::v2_4::aom2::constraint_model::c_attribute::CAttribute;
    use crate::v2_4::aom2::constraint_model::c_complex_object::CComplexObjectData;
    use crate::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
    use openehr_base::v1_3::base_types::definitions::version_status::VersionStatus;
    use openehr_base::v1_3::foundation_types::terminology::terminology_code::TerminologyCode;

    fn object(node_id: &str, attributes: Option<Vec<CAttribute>>) -> CComplexObject {
        CComplexObject::CComplexObject(CComplexObjectData {
            parent: None,
            soc_parent: None,
            rm_type_name: "ELEMENT".to_owned(),
            occurrences: None,
            node_id: node_id.to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            attributes,
            attribute_tuples: None,
        })
    }

    fn attribute(name: &str, children: Vec<CObject>) -> CAttribute {
        CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: name.to_owned(),
            existence: None,
            children: Some(children),
            differential_path: None,
            cardinality: None,
            is_multiple: true,
        }
    }

    fn terminology(concept_code: &str) -> ArchetypeTerminology {
        ArchetypeTerminology {
            is_differential: false,
            original_language: "en".to_owned(),
            concept_code: concept_code.to_owned(),
            term_definitions: [(
                "en".to_owned(),
                [(
                    "id2".to_owned(),
                    ArchetypeTerm {
                        code: "id2".to_owned(),
                        text: "systolic".to_owned(),
                        description: "systolic pressure".to_owned(),
                        other_items: None,
                    },
                )]
                .into_iter()
                .collect(),
            )]
            .into_iter()
            .collect(),
            term_bindings: None,
            value_sets: None,
            terminology_extracts: None,
        }
    }

    fn archetype(
        definition: CComplexObject,
        concept_code: &str,
        parent: Option<&str>,
    ) -> Archetype {
        Archetype::AuthoredArchetype(Box::new(AuthoredArchetype::AuthoredArchetype(
            AuthoredArchetypeData {
                parent_archetype_id: parent.map(str::to_owned),
                archetype_id: ArchetypeHrid {
                    namespace: None,
                    rm_publisher: "openEHR".to_owned(),
                    rm_package: "EHR".to_owned(),
                    rm_class: "OBSERVATION".to_owned(),
                    concept_id: "blood_pressure".to_owned(),
                    release_version: "1.0.0".to_owned(),
                    version_status: VersionStatus::Released,
                    build_count: "0".to_owned(),
                },
                is_differential: false,
                definition,
                terminology: Box::new(terminology(concept_code)),
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
                is_generated: false,
                other_meta_data: std::collections::BTreeMap::new(),
            },
        )))
    }

    #[test]
    fn the_concept_code_is_the_root_node_id() {
        let a = archetype(object("id1", None), "id1", None);
        assert_eq!(a.concept_code(), "id1");
    }

    #[test]
    fn specialisation_is_the_presence_of_a_parent_identifier() {
        assert!(!archetype(object("id1", None), "id1", None).is_specialised());
        assert!(
            archetype(
                object("id1", None),
                "id1.1",
                Some("openEHR-EHR-OBSERVATION.bp.v1")
            )
            .is_specialised()
        );
    }

    #[test]
    fn the_depth_comes_from_the_terminology_concept_code() {
        assert_eq!(
            archetype(object("id1", None), "id1", None).specialisation_depth(),
            0
        );
        assert_eq!(
            archetype(object("id1.1", None), "id1.1", None).specialisation_depth(),
            1
        );
    }

    #[test]
    fn physical_paths_alternate_attribute_names_and_node_ids() {
        let definition = object(
            "id1",
            Some(vec![attribute(
                "data",
                vec![CObject::CComplexObject(object(
                    "id2",
                    Some(vec![attribute(
                        "items",
                        vec![CObject::CComplexObject(object("", None))],
                    )]),
                ))],
            )]),
        );
        let a = archetype(definition, "id1", None);
        assert_eq!(
            a.physical_paths(),
            vec!["/data[id2]".to_owned(), "/data[id2]/items".to_owned()]
        );
    }

    #[test]
    fn logical_paths_replace_codes_with_their_rubrics() {
        let definition = object(
            "id1",
            Some(vec![attribute(
                "data",
                vec![CObject::CComplexObject(object("id2", None))],
            )]),
        );
        let a = archetype(definition, "id1", None);
        assert_eq!(a.logical_paths("en"), vec!["/data[systolic]".to_owned()]);
    }

    #[test]
    fn an_untranslated_code_keeps_its_code_in_the_logical_path() {
        let definition = object(
            "id1",
            Some(vec![attribute(
                "data",
                vec![CObject::CComplexObject(object("id9", None))],
            )]),
        );
        let a = archetype(definition, "id1", None);
        assert_eq!(a.logical_paths("en"), vec!["/data[id9]".to_owned()]);
        assert_eq!(a.logical_paths("fr"), vec!["/data[id9]".to_owned()]);
    }
}
