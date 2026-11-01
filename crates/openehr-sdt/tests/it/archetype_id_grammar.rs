// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM-instance validation applies the BASE 1.3.0 `ARCHETYPE_ID`
//! `Value_valid` grammar to `archetype_details.archetype_id` (#3835, #3845).
//!
//! BASE `docs/base_types/master05-identification_package.adoc` §Syntaxes: the
//! legacy ADL 1.4 form and the namespaced ADL 2 form are valid, with the
//! recorded relaxation for a namespaceless full-version identifier (AM 2.4.0
//! `ARCHETYPE_HRID.namespace` is optional, upstream conflict #3844); anything
//! else the grammar refuses is reported at the attribute that carries it.

use openehr_sdt::rm_instance::validate_rm_and_terminology;
use serde_json::{Value, json};

/// A minimal COMPOSITION whose root archetype id is `archetype_id`.
fn composition(archetype_id: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "name": {"_type": "DV_TEXT", "value": "Synthetic"},
        "archetype_node_id": archetype_id,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {"_type": "ARCHETYPE_ID", "value": archetype_id},
            "rm_version": "1.2.0"
        },
        "language": {"_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-1"}, "code_string": "en"},
        "territory": {"_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_3166-1"}, "code_string": "NL"},
        "category": {"_type": "DV_CODED_TEXT", "value": "event",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}, "code_string": "433"}},
        "composer": {"_type": "PARTY_SELF"}
    })
}

/// The `Value_valid` findings on the composition's `archetype_details.archetype_id`.
fn archetype_id_findings(archetype_id: &str) -> Vec<String> {
    validate_rm_and_terminology(&composition(archetype_id), "COMPOSITION")
        .into_iter()
        .filter(|m| m.path == "/archetype_details/archetype_id")
        .map(|m| m.message)
        .collect()
}

#[test]
fn the_three_accepted_forms_carry_no_finding() {
    for valid in [
        "openEHR-EHR-COMPOSITION.synthetic.v1",
        "org.openehr::openEHR-EHR-COMPOSITION.synthetic.v1.2.3",
        "org.openehr::openEHR-EHR-COMPOSITION.synthetic.v2.0.0-rc.4",
        "openEHR-EHR-COMPOSITION.synthetic.v1.0.0",
    ] {
        let findings = archetype_id_findings(valid);
        assert!(findings.is_empty(), "{valid}: {findings:?}");
    }
}

#[test]
fn an_identifier_the_grammar_refuses_is_reported_at_its_attribute() {
    for invalid in [
        "org.openehr::openEHR-EHR-COMPOSITION.synthetic.v1.0.0-gamma",
        "org..openehr::openEHR-EHR-COMPOSITION.synthetic.v1",
        "openEHR-EHR-COMPOSITION.synthetic.v1.0.0.0",
        "openEHR-EHR-COMPOSITION.synthetic.vx",
        "openEHR-EHR-COMPOSITION.synthetic.v01",
    ] {
        let findings = archetype_id_findings(invalid);
        assert!(
            findings.iter().any(|m| m.contains("Value_valid")),
            "{invalid} must be refused by Value_valid: {findings:?}"
        );
    }
}
