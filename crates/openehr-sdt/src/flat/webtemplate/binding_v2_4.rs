// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The external terms the `v2_4` Web Template front end
//! ([`super::builder_v2_4`]) writes for a coded constraint whose at-codes the
//! template binds to a terminology.
//!
//! An operational template may write the target of a term binding in data in
//! place of the archetype's own at-code (ADL2
//! `master08-terminology_integration.adoc` §"From Constraints to Concrete
//! Codes in Data"; the `[atN@ttttt]` and `[acN@ttttt]` form of ADL2
//! `master04.5-cadl_primitive_types.adoc`), the substitution AOM2
//! `master04.2-constraint_model-semantics.adoc` §"Terminology Code
//! Resolution" names `value_set_substituted`. A binding target is a URI in the
//! IHTSDO form (AOM2 `master01-preface.adoc`), `http://openehr.org/id/433` for
//! the openEHR term `433`.

use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

/// The terminology an `@ttttt` operational binding names on a constraint,
/// split from the constraint's code: `ac1@snomed_ct` reads as `ac1` and
/// `snomed_ct`.
pub(super) fn split_operational(constraint: &str) -> (&str, Option<&str>) {
    match constraint.split_once('@') {
        Some((code, terminology)) => (code.trim(), Some(terminology.trim())),
        None => (constraint.trim(), None),
    }
}

/// The admitted at-codes written as the terms they are bound to: the
/// terminology and one bound code per at-code, in order.
///
/// Two bindings substitute: the one an `@ttttt` operational binding names
/// (ADL2 `master04.5-cadl_primitive_types.adoc`), and an `openehr` binding,
/// whose term the RM requires where it codes an RM attribute (RM
/// `COMPOSITION` invariant `Category_validity`, `EVENT_CONTEXT` invariant
/// `Setting_valid`). Any other binding keeps the local at-codes, the default
/// ADL2 `master08-terminology_integration.adoc` §"From Constraints to Concrete
/// Codes in Data" names, as does a binding that leaves an admitted code
/// unbound.
pub(super) fn bound_codes<'t>(
    term: &'t ArchetypeTerminology,
    operational: Option<&str>,
    codes: &[String],
) -> Option<(&'t str, Vec<String>)> {
    if codes.is_empty() {
        return None;
    }
    // NOTE: ADL2 master08 §"From Constraints to Concrete Codes in Data" makes
    // substitution a choice; only `@ttttt` and an RM-required `openehr` code take it.
    let named = operational.unwrap_or("openehr");
    let (terminology, targets) = term
        .term_bindings
        .as_ref()?
        .iter()
        .find(|(terminology, _)| terminology.eq_ignore_ascii_case(named))?;
    let bound = codes
        .iter()
        .map(|code| targets.get(code).and_then(|uri| bound_term_code(uri)))
        .collect::<Option<Vec<_>>>()?;
    Some((terminology.as_str(), bound))
}

/// The code a term-binding URI names: the last segment after `/id/`, as in
/// `http://snomed.info/id/406464007`.
///
/// `None` for a URI of another form, such as a value-set reference.
fn bound_term_code(uri: &str) -> Option<String> {
    let (_, code) = uri.trim().rsplit_once("/id/")?;
    (!code.is_empty() && !code.contains('/')).then(|| code.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn terminology(bindings: &[(&str, &[(&str, &str)])]) -> ArchetypeTerminology {
        ArchetypeTerminology {
            is_differential: false,
            original_language: "en".to_owned(),
            concept_code: "id1".to_owned(),
            term_definitions: BTreeMap::new(),
            value_sets: None,
            terminology_extracts: None,
            term_bindings: Some(
                bindings
                    .iter()
                    .map(|(name, targets)| {
                        let targets = targets
                            .iter()
                            .map(|(code, uri)| ((*code).to_owned(), (*uri).to_owned()))
                            .collect();
                        ((*name).to_owned(), targets)
                    })
                    .collect(),
            ),
        }
    }

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| (*c).to_owned()).collect()
    }

    #[test]
    fn an_openehr_binding_names_its_term() {
        let term = terminology(&[("openehr", &[("at1", "http://openehr.org/id/433")])]);
        assert_eq!(
            bound_codes(&term, None, &codes(&["at1"])),
            Some(("openehr", codes(&["433"])))
        );
    }

    #[test]
    fn a_partial_binding_keeps_the_local_codes() {
        let term = terminology(&[("openehr", &[("at1", "http://openehr.org/id/433")])]);
        assert_eq!(bound_codes(&term, None, &codes(&["at1", "at2"])), None);
    }

    #[test]
    fn a_binding_other_than_openehr_keeps_the_local_code_unless_named() {
        let term = terminology(&[("snomed_ct", &[("at1", "http://snomed.info/id/7")])]);
        let at1 = codes(&["at1"]);
        assert_eq!(bound_codes(&term, None, &at1), None);
        assert_eq!(
            bound_codes(&term, Some("snomed_ct"), &at1),
            Some(("snomed_ct", codes(&["7"])))
        );
        assert_eq!(bound_codes(&term, Some("loinc"), &at1), None);
    }

    #[test]
    fn an_operational_binding_wins_over_openehr() {
        let term = terminology(&[
            ("openehr", &[("at1", "http://openehr.org/id/433")]),
            ("snomed_ct", &[("at1", "http://snomed.info/id/7")]),
        ]);
        let at1 = codes(&["at1"]);
        assert_eq!(
            bound_codes(&term, None, &at1),
            Some(("openehr", codes(&["433"])))
        );
        assert_eq!(
            bound_codes(&term, Some("snomed_ct"), &at1),
            Some(("snomed_ct", codes(&["7"])))
        );
    }

    #[test]
    fn a_uri_without_an_id_segment_names_no_term() {
        assert_eq!(
            bound_term_code("http://snomed.info/id/406464007"),
            Some("406464007".to_owned())
        );
        assert_eq!(bound_term_code("http://example.org/valueset/12"), None);
        assert_eq!(bound_term_code("http://example.org/id/"), None);
    }

    #[test]
    fn an_operational_binding_splits_from_its_code() {
        assert_eq!(
            split_operational("ac1@snomed_ct"),
            ("ac1", Some("snomed_ct"))
        );
        assert_eq!(split_operational("at3"), ("at3", None));
    }
}
