// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `ARCHETYPE_ONTOLOGY` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.archetype_ontology.adoc`
//! §Attributes + §Functions, and `AM/docs/AOM1.4/master07-ontology_package.adoc`
//! §Overview.
//!
//! NOTE: the class page declares `term_definition`, `constraint_definition`,
//! `term_binding`, `constraint_binding` and `has_language` over a term/binding
//! store and a language list that its own §Attributes section never declares,
//! so those five have no state to read here.

use crate::v1_4::aom14::archetype::ontology::archetype_ontology::ArchetypeOntology;

impl ArchetypeOntology {
    /// Returns true if `a_code` is one of this ontology's term codes.
    ///
    /// `has_term_code` (`org.openehr.am.aom14.archetype_ontology.adoc`
    /// §Functions): "True if `term_codes` has `a_code`."
    #[must_use]
    pub fn has_term_code(&self, a_code: &str) -> bool {
        self.term_codes.iter().any(|c| c == a_code)
    }

    /// Returns true if `a_code` is one of this ontology's constraint codes.
    ///
    /// `has_constraint_code` (`org.openehr.am.aom14.archetype_ontology.adoc`
    /// §Functions): "True if `constraint_codes` has `a_code`."
    #[must_use]
    pub fn has_constraint_code(&self, a_code: &str) -> bool {
        self.constraint_codes.iter().any(|c| c == a_code)
    }

    /// Returns true if this ontology binds to terminology `a_terminology_id`.
    ///
    /// `has_terminology` (`org.openehr.am.aom14.archetype_ontology.adoc`
    /// §Functions): "True if terminology `a_terminology` is present in archetype
    /// ontology" — read against the `terminologies_available` attribute the same
    /// page declares as "List of terminologies to which term or constraint
    /// bindings exist in this terminology". An absent list binds nothing.
    #[must_use]
    pub fn has_terminology(&self, a_terminology_id: &str) -> bool {
        self.terminologies_available
            .as_ref()
            .is_some_and(|ids| ids.iter().any(|id| id == a_terminology_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::containers::NonEmptyVec;

    fn ontology(terminologies: Option<Vec<String>>) -> ArchetypeOntology {
        ArchetypeOntology {
            term_codes: NonEmptyVec::new(vec!["at0000".to_owned(), "at0001".to_owned()])
                .expect("two term codes are a non-empty vector"),
            constraint_codes: NonEmptyVec::new(vec!["ac0001".to_owned()])
                .expect("one constraint code is a non-empty vector"),
            terminologies_available: terminologies,
            specialisation_depth: 0,
            term_attribute_names: NonEmptyVec::new(vec![
                "text".to_owned(),
                "description".to_owned(),
            ])
            .expect("two attribute names are a non-empty vector"),
        }
    }

    #[test]
    fn term_and_constraint_codes_are_looked_up_in_their_own_lists() {
        let o = ontology(None);
        assert!(o.has_term_code("at0001"));
        assert!(!o.has_term_code("ac0001"));
        assert!(o.has_constraint_code("ac0001"));
        assert!(!o.has_constraint_code("at0001"));
    }

    #[test]
    fn a_code_that_is_a_prefix_of_a_defined_one_is_not_defined() {
        let o = ontology(None);
        assert!(!o.has_term_code("at000"));
        assert!(!o.has_term_code("at00010"));
    }

    #[test]
    fn an_absent_terminology_list_binds_nothing() {
        assert!(!ontology(None).has_terminology("SNOMED_CT"));
        assert!(!ontology(Some(Vec::new())).has_terminology("SNOMED_CT"));
        assert!(ontology(Some(vec!["SNOMED_CT".to_owned()])).has_terminology("SNOMED_CT"));
    }
}
