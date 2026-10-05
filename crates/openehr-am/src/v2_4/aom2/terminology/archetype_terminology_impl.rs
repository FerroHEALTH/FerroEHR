// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `ARCHETYPE_TERMINOLOGY` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.archetype_terminology.adoc`
//! §Attributes + §Functions, `AM/docs/AOM2/master07-terminology_package.adoc`
//! §Overview + §Specialisation Depth, and
//! `AM/docs/UML/classes/org.openehr.am.aom2.adl_code_definitions.adoc`
//! §Constants (the `at`/`id`/`ac` leaders and the `.` separator).

use crate::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use crate::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use crate::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

impl ArchetypeTerminology {
    /// Returns the specialisation depth of the owning artefact.
    ///
    /// `specialisation_depth` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "Unspecialised artefacts have depth 0, with each additional
    /// level of specialisation adding 1". `master07-terminology_package.adoc`
    /// §Specialisation Depth makes the depth of a code its count of `.`
    /// specialisation separators (`at0004` → 0, `at0004.1` → 1,
    /// `at0004.0.1` → 2), and the same page states `concept_code` is the root
    /// code of the artefact, so the artefact's depth is that code's depth.
    #[must_use]
    pub fn specialisation_depth(&self) -> i32 {
        i32::try_from(
            self.concept_code
                .matches(AdlCodeDefinitionsData::SPECIALISATION_SEPARATOR)
                .count(),
        )
        .unwrap_or(i32::MAX)
    }

    /// Returns the node codes defined in this terminology.
    ///
    /// `node_codes` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "For at-coded archetypes: list of all at codes …; for
    /// id-coded archetypes: list of all id codes", i.e. the codes that appear
    /// as `C_OBJECT.node_id`. Which code space is in play is decided by
    /// `concept_code`, which the same page defines as "always used as the
    /// at-code (at-coded archetypes) or id-code (id-coded archetypes) on the
    /// root node".
    ///
    /// NOTE: `adl_code_definitions.adoc` §Constants sets `At_code_leader` and
    /// `Value_code_leader` to the same `"at"`, so in an at-coded archetype the
    /// node codes and the value codes are one indistinguishable set.
    #[must_use]
    pub fn node_codes(&self) -> Vec<&str> {
        let leader = if self
            .concept_code
            .starts_with(AdlCodeDefinitionsData::ID_CODE_LEADER)
        {
            AdlCodeDefinitionsData::ID_CODE_LEADER
        } else {
            AdlCodeDefinitionsData::AT_CODE_LEADER
        };
        self.codes_with_leader(leader)
    }

    /// Returns the value term codes defined in this terminology.
    ///
    /// `value_codes` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "the 'at' codes in an ADL archetype, which are used as
    /// possible values on terminological constrainer nodes" — the codes
    /// carrying `Value_code_leader` (`adl_code_definitions.adoc` §Constants).
    /// The declared result multiplicity is `0..1`, so an archetype defining no
    /// such code yields no list.
    #[must_use]
    pub fn value_codes(&self) -> Option<Vec<&str>> {
        let codes = self.codes_with_leader(AdlCodeDefinitionsData::VALUE_CODE_LEADER);
        (!codes.is_empty()).then_some(codes)
    }

    /// Returns the value-set codes defined in this terminology.
    ///
    /// `value_set_codes` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "These correspond to the 'ac' codes in an ADL archetype" —
    /// the codes carrying `Value_set_code_leader` (`adl_code_definitions.adoc`
    /// §Constants). The declared result multiplicity is `0..1`.
    #[must_use]
    pub fn value_set_codes(&self) -> Option<Vec<&str>> {
        let codes = self.codes_with_leader(AdlCodeDefinitionsData::VALUE_SET_CODE_LEADER);
        (!codes.is_empty()).then_some(codes)
    }

    /// Returns true if language `a_lang` is present in this terminology.
    ///
    /// `has_language` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions), read against `term_definitions`, whose "outer hash keys are
    /// language codes".
    #[must_use]
    pub fn has_language(&self, a_lang: &str) -> bool {
        self.term_definitions.contains_key(a_lang)
    }

    /// Returns the languages this terminology's terms are available in.
    ///
    /// `languages_available` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "List of languages in which terms in this terminology are
    /// available" — the outer keys of `term_definitions`.
    #[must_use]
    pub fn languages_available(&self) -> Vec<&str> {
        self.term_definitions.keys().map(String::as_str).collect()
    }

    /// Returns true if bindings to terminology `a_terminology_id` are present.
    ///
    /// `has_terminology` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions), read against `term_bindings`, whose "outer hash keys are
    /// terminology ids".
    #[must_use]
    pub fn has_terminology(&self, a_terminology_id: &str) -> bool {
        self.term_bindings
            .as_ref()
            .is_some_and(|b| b.contains_key(a_terminology_id))
    }

    /// Returns the terminologies this terminology binds to.
    ///
    /// `terminologies_available`
    /// (`org.openehr.am.aom2.archetype_terminology.adoc` §Functions): "List of
    /// terminologies to which term or constraint bindings exist in this
    /// terminology, computed from bindings."
    #[must_use]
    pub fn terminologies_available(&self) -> Vec<&str> {
        self.term_bindings
            .iter()
            .flat_map(|b| b.keys().map(String::as_str))
            .collect()
    }

    /// Returns true if code `a_code` is defined in this terminology.
    ///
    /// `has_term_code` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "True if code `a_code` defined in this terminology". The
    /// same page's VTLC rule ("all term codes and constraint codes exist in all
    /// languages", `master07-terminology_package.adoc` §Validity Rules) makes
    /// the per-language sets equal for a valid terminology, so any language
    /// defining the code answers the question.
    #[must_use]
    pub fn has_term_code(&self, a_code: &str) -> bool {
        self.term_definitions
            .values()
            .any(|codes| codes.contains_key(a_code))
    }

    /// Returns the term definition for `a_code` in `a_lang`, if defined.
    ///
    /// `term_definition` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions), whose precondition is `has_term_definition (a_lang,
    /// a_code)`; an undefined pair yields `None` rather than a panic.
    #[must_use]
    pub fn term_definition(&self, a_lang: &str, a_code: &str) -> Option<&ArchetypeTerm> {
        self.term_definitions.get(a_lang)?.get(a_code)
    }

    /// Returns the binding of `a_code` in `a_terminology`, if bound.
    ///
    /// `term_binding` (`org.openehr.am.aom2.archetype_terminology.adoc`
    /// §Functions): "Binding of constraint corresponding to `a_code` in target
    /// external terminology `a_terminology_id`". Its precondition is
    /// `has_term_binding (a_terminology_id, a_code)`; an unbound pair yields
    /// `None`.
    #[must_use]
    pub fn term_binding(&self, a_terminology: &str, a_code: &str) -> Option<&str> {
        self.term_bindings
            .as_ref()?
            .get(a_terminology)?
            .get(a_code)
            .map(String::as_str)
    }

    /// Returns true if an extract of terminology `a_terminology_id` is present.
    ///
    /// `has_terminology_extract`
    /// (`org.openehr.am.aom2.archetype_terminology.adoc` §Functions), read
    /// against `terminology_extracts`, whose "outer hash keys are terminology
    /// ids".
    #[must_use]
    pub fn has_terminology_extract(&self, a_terminology_id: &str) -> bool {
        self.terminology_extracts
            .as_ref()
            .is_some_and(|e| e.contains_key(a_terminology_id))
    }

    /// Returns the extract term for `a_code` in terminology `a_terminology_id`.
    ///
    /// `terminology_extract_term`
    /// (`org.openehr.am.aom2.archetype_terminology.adoc` §Functions), whose
    /// precondition is `has_terminology_extract (a_terminology_id) and
    /// has_terminology_extract_code (a_code)`; an absent pair yields `None`.
    #[must_use]
    pub fn terminology_extract_term(
        &self,
        a_terminology_id: &str,
        a_code: &str,
    ) -> Option<&ArchetypeTerm> {
        self.terminology_extracts
            .as_ref()?
            .get(a_terminology_id)?
            .get(a_code)
    }

    /// Every defined term code carrying `leader`, deduplicated across languages.
    fn codes_with_leader(&self, leader: &str) -> Vec<&str> {
        let mut codes: Vec<&str> = self
            .term_definitions
            .values()
            .flat_map(|by_code| by_code.keys().map(String::as_str))
            .filter(|code| code.starts_with(leader))
            .collect();
        codes.sort_unstable();
        codes.dedup();
        codes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn term(code: &str) -> ArchetypeTerm {
        ArchetypeTerm {
            code: code.to_owned(),
            text: format!("text for {code}"),
            description: format!("description for {code}"),
            other_items: None,
        }
    }

    fn terminology(concept_code: &str, codes: &[&str]) -> ArchetypeTerminology {
        let by_code: BTreeMap<String, ArchetypeTerm> =
            codes.iter().map(|c| ((*c).to_owned(), term(c))).collect();
        ArchetypeTerminology {
            is_differential: false,
            original_language: "en".to_owned(),
            concept_code: concept_code.to_owned(),
            term_definitions: [
                ("en".to_owned(), by_code.clone()),
                ("de".to_owned(), by_code),
            ]
            .into_iter()
            .collect(),
            term_bindings: Some(
                [(
                    "SNOMED_CT".to_owned(),
                    [(
                        "at0004".to_owned(),
                        "http://snomed.info/id/271649006".to_owned(),
                    )]
                    .into_iter()
                    .collect(),
                )]
                .into_iter()
                .collect(),
            ),
            value_sets: None,
            terminology_extracts: Some(
                [(
                    "ICD10".to_owned(),
                    [("I10".to_owned(), term("I10"))].into_iter().collect(),
                )]
                .into_iter()
                .collect(),
            ),
        }
    }

    #[test]
    fn the_depth_is_the_separator_count_of_the_concept_code() {
        assert_eq!(terminology("at0000", &[]).specialisation_depth(), 0);
        assert_eq!(terminology("at0000.1", &[]).specialisation_depth(), 1);
        assert_eq!(terminology("id1.1.1", &[]).specialisation_depth(), 2);
    }

    #[test]
    fn an_id_coded_archetype_separates_node_codes_from_value_codes() {
        let t = terminology("id1", &["id1", "id2", "at4", "ac1"]);
        assert_eq!(t.node_codes(), vec!["id1", "id2"]);
        assert_eq!(t.value_codes(), Some(vec!["at4"]));
        assert_eq!(t.value_set_codes(), Some(vec!["ac1"]));
    }

    #[test]
    fn an_at_coded_archetype_shares_one_code_space_for_nodes_and_values() {
        let t = terminology("at0000", &["at0000", "at0004", "ac1"]);
        assert_eq!(t.node_codes(), vec!["at0000", "at0004"]);
        assert_eq!(t.value_codes(), Some(vec!["at0000", "at0004"]));
    }

    #[test]
    fn absent_code_families_yield_no_list() {
        let t = terminology("at0000", &["at0000"]);
        assert_eq!(t.value_set_codes(), None);
    }

    #[test]
    fn languages_and_terminologies_come_from_their_own_tables() {
        let t = terminology("at0000", &["at0004"]);
        assert_eq!(t.languages_available(), vec!["de", "en"]);
        assert!(t.has_language("de"));
        assert!(!t.has_language("fr"));
        assert_eq!(t.terminologies_available(), vec!["SNOMED_CT"]);
        assert!(t.has_terminology("SNOMED_CT"));
        assert!(!t.has_terminology("ICD10"));
    }

    #[test]
    fn definitions_and_bindings_resolve_by_language_and_terminology() {
        let t = terminology("at0000", &["at0004"]);
        assert!(t.has_term_code("at0004"));
        assert!(!t.has_term_code("at0099"));
        assert_eq!(
            t.term_definition("en", "at0004").map(|d| d.text.as_str()),
            Some("text for at0004")
        );
        assert_eq!(t.term_definition("fr", "at0004"), None);
        assert_eq!(
            t.term_binding("SNOMED_CT", "at0004"),
            Some("http://snomed.info/id/271649006")
        );
        assert_eq!(t.term_binding("SNOMED_CT", "at0099"), None);
        assert_eq!(t.term_binding("LOINC", "at0004"), None);
    }

    #[test]
    fn terminology_extracts_answer_only_for_codes_they_carry() {
        let t = terminology("at0000", &["at0004"]);
        assert!(t.has_terminology_extract("ICD10"));
        assert!(!t.has_terminology_extract("SNOMED_CT"));
        assert_eq!(
            t.terminology_extract_term("ICD10", "I10")
                .map(|e| e.code.as_str()),
            Some("I10")
        );
        assert_eq!(t.terminology_extract_term("ICD10", "J45"), None);
    }
}
