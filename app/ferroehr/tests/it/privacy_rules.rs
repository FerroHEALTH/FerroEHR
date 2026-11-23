// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Rule-level tests for [`ferroehr::privacy::PrivacyPolicy`]: what each rule refuses, what
//! it deliberately leaves alone, and that the default configuration is the
//! refusing one.

use serde_json::{Value, json};

use crate::fixtures::committer;
use ferroehr::privacy::config::PrivacyConfig;
use ferroehr::privacy::{Finding, PrivacyPolicy};

/// A pseudonym namespace and an opaque subject, the shape the boundary
/// mandates.
const PSEUDONYM_NS: &str = "urn:ferroehr:pseudonym";
const OPAQUE_SUBJECT: &str = "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3";

fn policy(config: &PrivacyConfig) -> PrivacyPolicy {
    PrivacyPolicy::from(config)
}

fn enforcing() -> PrivacyPolicy {
    policy(&PrivacyConfig {
        subject_namespaces: vec![PSEUDONYM_NS.to_owned()],
        ..PrivacyConfig::default()
    })
}

fn status_with_subject(namespace: &str, id: &str) -> Value {
    json!({
        "_type": "EHR_STATUS",
        "name": { "_type": "DV_TEXT", "value": "status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": namespace,
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": id, "scheme": "local" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

fn paths(findings: &[Finding]) -> Vec<&str> {
    findings.iter().map(|f| f.path.as_str()).collect()
}

// ── the shipped default ───────────────────────────────────────────────────────

/// The shipped default is the minimising posture: the identified-party opt-in
/// off (which half of that rule bites is the matrix below) and the subject
/// rule waiting for a deployment fact.
#[test]
fn the_configuration_default_is_the_minimising_posture() {
    let default = PrivacyConfig::default();
    assert!(!default.allow_identified_parties_in_ehr);
    // The subject rule is the one that waits for a deployment fact: there is no
    // pseudonym namespace a server could invent for an operator.
    assert_eq!(default.subject_namespaces, Vec::<String>::new());
    let compiled = policy(&default);
    assert!(!compiled.subject_rule_in_force());
    assert!(compiled.applies_to("COMPOSITION"));
}

// ── rule 1: the subject reference ─────────────────────────────────────────────

#[test]
fn an_opaque_subject_in_a_configured_namespace_passes() {
    let findings = enforcing().findings(
        "EHR_STATUS",
        &status_with_subject(PSEUDONYM_NS, OPAQUE_SUBJECT),
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn an_unconfigured_namespace_is_refused() {
    let findings = enforcing().findings(
        "EHR_STATUS",
        &status_with_subject("uk.org.nmc", OPAQUE_SUBJECT),
    );
    assert_eq!(
        paths(&findings),
        ["EHR_STATUS/subject/external_ref/namespace"]
    );
}

#[test]
fn a_non_uuid_subject_id_is_refused_and_the_value_never_travels() {
    let findings = enforcing().findings(
        "EHR_STATUS",
        &status_with_subject(PSEUDONYM_NS, "111222333"), // privacy-allow: synthetic, and the refusal must not echo it
    );
    assert_eq!(
        paths(&findings),
        ["EHR_STATUS/subject/external_ref/id/value"]
    );
    for finding in &findings {
        assert!(
            !finding.message.contains("111222333"), // privacy-allow: synthetic, asserted ABSENT
            "a refusal must name the shape, never the value: {}",
            finding.message
        );
    }
}

#[test]
fn the_subject_rule_waits_for_a_configured_namespace() {
    let unconfigured = policy(&PrivacyConfig::default());
    assert_eq!(
        unconfigured.findings(
            "EHR_STATUS",
            &status_with_subject("uk.org.nmc", "patient-42")
        ),
        Vec::<Finding>::new()
    );
}

#[test]
fn an_ehr_status_with_no_subject_reference_is_the_spec_baseline_and_passes() {
    // ITS-REST `operations/ehr_create.yaml`: the default EHR_STATUS the service
    // uses carries "`subject`: a PARTY_SELF object" and nothing else.
    let bare = json!({
        "_type": "EHR_STATUS",
        "name": { "_type": "DV_TEXT", "value": "status" },
        "subject": { "_type": "PARTY_SELF" },
        "is_queryable": true,
        "is_modifiable": true
    });
    assert_eq!(
        enforcing().findings("EHR_STATUS", &bare),
        Vec::<Finding>::new()
    );
}

#[test]
fn the_subject_rule_binds_ehr_status_only() {
    // A COMPOSITION has no `subject` slot of this kind; a FOLDER neither. The
    // rule must not wander into a body that merely carries the word.
    let composition = json!({
        "_type": "COMPOSITION",
        "subject": { "_type": "PARTY_SELF", "external_ref": {
            "_type": "PARTY_REF", "namespace": "elsewhere", "type": "PERSON",
            "id": { "_type": "GENERIC_ID", "value": "not-a-uuid", "scheme": "local" } } }
    });
    assert_eq!(
        enforcing().findings("COMPOSITION", &composition),
        Vec::<Finding>::new()
    );
}

// ── rule 2: identified parties ────────────────────────────────────────────────

/// The permitted half of the matrix: `PARTY_IDENTIFIED.name` under the
/// shipped default.
///
/// The class is "Proxy data for an identified party other than the subject of
/// the record", "Typically for health care providers" (RM common
/// `UML/classes/org.openehr.rm.common.party_identified.adoc` §Description), so
/// a composer name is provider identity and the boundary does not reach it.
#[test]
fn a_named_party_identified_in_clinical_content_is_accepted_by_default() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": committer("Dr Author")
    });
    let findings = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
    assert!(findings.is_empty(), "{findings:?}");
}

/// The refused half of the matrix on the same class: `identifiers`.
///
/// "One or more formal identifiers (possibly computable)" (same file,
/// §Attributes) is the national-identifier slot, which the clinical side does
/// not hold whatever the party is.
#[test]
fn formal_identifiers_on_a_party_identified_are_accepted_by_default() {
    // "Used to describe parties where only identifiers may be known … e.g.
    // name and provider number of an institution" (RM common
    // `UML/classes/org.openehr.rm.common.party_identified.adoc` §Description):
    // a registration number on the composer is the class's paradigm case, and
    // `ctx/participation_identifiers` builds exactly this (#3254).
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": {
            "_type": "PARTY_IDENTIFIED",
            "name": "Dr Author",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "GMC-1234567", "issuer": "GMC" }]
        }
    });
    let findings = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
    assert!(findings.is_empty(), "{findings:?}");
}

/// A `PARTY_RELATED` whose relationship names a third party keeps its name.
///
/// The class is "Proxy type for identifying a party and its relationship to
/// the subject of the record" (RM common
/// `UML/classes/org.openehr.rm.common.party_related.adoc`); a mother (openEHR
/// `subject relationship` code `10`) is not the subject, and the released
/// spec obliges a server to accept the content the RM models (#3252).
#[test]
fn a_named_party_related_to_a_third_party_is_accepted_by_default() {
    let composition = json!({
        "_type": "COMPOSITION",
        "content": [{
            "_type": "OBSERVATION",
            "other_participations": [{
                "_type": "PARTICIPATION",
                "performer": {
                    "_type": "PARTY_RELATED",
                    "name": "A Relative",
                    "relationship": {
                        "_type": "DV_CODED_TEXT",
                        "value": "mother",
                        "defining_code": {
                            "_type": "CODE_PHRASE",
                            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                            "code_string": "10"
                        }
                    }
                }
            }]
        }]
    });
    let findings = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
    assert!(findings.is_empty(), "{findings:?}");
}

/// The refused cell of the `PARTY_RELATED` row: a name on the party that IS
/// the subject, "If it is the patient, coded as self" (RM common
/// `UML/classes/org.openehr.rm.common.party_related.adoc` §Attributes), coded
/// `openehr::0` or spelled out where no code settles it.
#[test]
fn a_named_party_related_coded_self_is_refused_by_default() {
    for relationship in [
        json!({
            "_type": "DV_CODED_TEXT",
            "value": "self",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "0"
            }
        }),
        json!({ "_type": "DV_CODED_TEXT", "value": "Self" }),
    ] {
        let composition = json!({
            "_type": "COMPOSITION",
            "content": [{
                "_type": "OBSERVATION",
                "other_participations": [{
                    "_type": "PARTICIPATION",
                    "performer": {
                        "_type": "PARTY_RELATED",
                        "name": "The Patient",
                        "relationship": relationship
                    }
                }]
            }]
        });
        let findings = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
        assert_eq!(
            paths(&findings),
            ["COMPOSITION/content[0]/other_participations[0]/performer/name"]
        );
    }
}

#[test]
fn an_identified_party_reduced_to_its_external_ref_passes() {
    // PARTY_IDENTIFIED.Basic_validity is `name /= Void or identifiers /= Void or
    // external_ref /= Void`, so the reduced form is still a valid instance.
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": { "_type": "PARTY_IDENTIFIED", "external_ref": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": OPAQUE_SUBJECT } } }
    });
    assert_eq!(
        policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition),
        Vec::<Finding>::new()
    );
}

/// Both refused cells of the `PARTY_RELATED` row at once — a name on a `self`
/// party and formal identifiers — and the opt-in over the same body.
#[test]
fn identifiers_on_a_party_related_are_refused_too_and_the_opt_in_accepts_both() {
    let composition = json!({
        "_type": "COMPOSITION",
        "content": [{
            "_type": "OBSERVATION",
            "other_participations": [{
                "_type": "PARTICIPATION",
                "performer": {
                    "_type": "PARTY_RELATED",
                    "name": "The Patient",
                    "relationship": { "_type": "DV_CODED_TEXT", "value": "self" },
                    "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "REL-7" }]
                }
            }]
        }]
    });
    let refused = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
    assert_eq!(
        paths(&refused),
        [
            "COMPOSITION/content[0]/other_participations[0]/performer/name",
            "COMPOSITION/content[0]/other_participations[0]/performer/identifiers",
        ]
    );
    let permitted = policy(&PrivacyConfig {
        allow_identified_parties_in_ehr: true,
        ..PrivacyConfig::default()
    });
    assert_eq!(
        permitted.findings("COMPOSITION", &composition),
        Vec::<Finding>::new()
    );
}

/// The opt-in accepts every cell of the matrix, both classes and both
/// attributes, so the key still means what it always meant when set.
#[test]
fn the_opt_in_accepts_every_cell_of_the_matrix() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": {
            "_type": "PARTY_IDENTIFIED",
            "name": "Dr Author",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "GMC-1234567" }]
        },
        "content": [{
            "_type": "OBSERVATION",
            "other_participations": [{
                "_type": "PARTICIPATION",
                "performer": {
                    "_type": "PARTY_RELATED",
                    "name": "The Patient",
                    "relationship": { "_type": "DV_CODED_TEXT", "value": "self" },
                    "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "REL-7" }]
                }
            }]
        }]
    });
    let permitted = policy(&PrivacyConfig {
        allow_identified_parties_in_ehr: true,
        ..PrivacyConfig::default()
    });
    let findings = permitted.findings("COMPOSITION", &composition);
    assert!(findings.is_empty(), "{findings:?}");
}

/// The identified-party opt-in leaves the subject rule in force: an
/// `EHR_STATUS` still meets the pass, a COMPOSITION has nothing to meet.
#[test]
fn the_opt_in_leaves_the_subject_rule_in_force() {
    let permitted = policy(&PrivacyConfig {
        subject_namespaces: vec![PSEUDONYM_NS.to_owned()],
        allow_identified_parties_in_ehr: true,
        ..PrivacyConfig::default()
    });
    assert!(permitted.applies_to("EHR_STATUS"));
    assert!(!permitted.applies_to("COMPOSITION"));
    let findings = permitted.findings(
        "EHR_STATUS",
        &status_with_subject(PSEUDONYM_NS, "not-a-pseudonym"),
    );
    assert_eq!(
        paths(&findings),
        ["EHR_STATUS/subject/external_ref/id/value"]
    );
    let open = policy(&PrivacyConfig {
        allow_identified_parties_in_ehr: true,
        ..PrivacyConfig::default()
    });
    assert!(!open.applies_to("EHR_STATUS"));
}

#[test]
fn a_party_self_carries_no_name_slot_and_is_untouched() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": { "_type": "PARTY_SELF" }
    });
    assert_eq!(
        policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition),
        Vec::<Finding>::new()
    );
}

/// The library default is the refusing posture the shipped configuration
/// builds (#3243): a service built without an explicit policy refuses a
/// subject's own name on a `self` party, never a more permissive shape than
/// the binary.
#[test]
fn the_default_policy_is_the_refusing_posture() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": {
            "_type": "PARTY_RELATED",
            "name": "The Patient",
            "relationship": { "_type": "DV_CODED_TEXT", "value": "self" }
        }
    });
    let findings = PrivacyPolicy::default().findings("COMPOSITION", &composition);
    assert_eq!(paths(&findings), ["COMPOSITION/composer/name"]);
    assert!(PrivacyPolicy::default().applies_to("COMPOSITION"));
}

#[test]
fn a_uuid_subject_is_accepted_in_either_case() {
    let upper = OPAQUE_SUBJECT.to_ascii_uppercase();
    assert_eq!(
        enforcing().findings("EHR_STATUS", &status_with_subject(PSEUDONYM_NS, &upper)),
        Vec::<Finding>::new()
    );
    // The braced and URN spellings the uuid parser also accepts are refused:
    // the promoted column compares as text.
    for spelling in [
        format!("{{{OPAQUE_SUBJECT}}}"),
        format!("urn:uuid:{OPAQUE_SUBJECT}"),
        OPAQUE_SUBJECT.replace('-', ""),
    ] {
        let findings =
            enforcing().findings("EHR_STATUS", &status_with_subject(PSEUDONYM_NS, &spelling));
        assert!(
            findings.iter().any(|f| f.path.ends_with("/id/value")),
            "{spelling} should be refused"
        );
    }
}

// ── clinical digit runs ───────────────────────────────────────────────────────

/// Digit runs a laboratory carries that pass a national checksum by chance:
/// the filler order number a partner reported (ten digits, a YYMMDD reading
/// and a Luhn pass), two order numbers passing the NHS Number modulus 11, and
/// runs passing the eleven-test and the Swiss GS1 check. Not one names a
/// person, and the policy guesses nothing from a value's shape (#6081).
const CHECKSUM_VALID_RUNS: [&str; 5] = [
    "9702311805", // privacy-allow: a laboratory filler order number, not a person identifier
    "8412501861", // privacy-allow: a laboratory order number, not a person identifier
    "8372500932", // privacy-allow: a laboratory order number, not a person identifier
    "123456782",  // privacy-allow: synthetic, built to satisfy the eleven-test
    "7561234567897", // privacy-allow: synthetic, built to satisfy the GS1 modulo-10 check
];

/// A `DV_IDENTIFIER` with an optional `type`, as canonical JSON.
fn identifier(id: &str, kind: Option<&str>) -> Value {
    match kind {
        Some(kind) => json!({
            "_type": "DV_IDENTIFIER", "id": id, "type": kind, "assigner": "lab.example"
        }),
        None => json!({ "_type": "DV_IDENTIFIER", "id": id, "assigner": "lab.example" }),
    }
}

/// A `laboratory_test_result.v1` observation carrying `order` where a
/// laboratory carries it: the feeder audit's item ids, the protocol's
/// "Receiver order identifier" element (at0063), a provider's formal
/// identifier and the free-text name.
fn laboratory_result(order: &str, order_type: Option<&str>) -> Value {
    json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": format!("laboratory report {order}") },
        "composer": {
            "_type": "PARTY_IDENTIFIED",
            "name": "Dr Author",
            "identifiers": [identifier(order, None)]
        },
        "feeder_audit": {
            "originating_system_audit": { "system_id": "lab.example" },
            "originating_system_item_ids": [identifier(order, order_type)]
        },
        "content": [{
            "_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.laboratory_test_result.v1",
            "name": { "_type": "DV_TEXT", "value": "Laboratory test result" },
            "protocol": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0004",
                "name": { "_type": "DV_TEXT", "value": "Tree" },
                "items": [{
                    "_type": "CLUSTER",
                    "archetype_node_id": "at0094",
                    "name": { "_type": "DV_TEXT", "value": "Test request details" },
                    "items": [{
                        "_type": "ELEMENT",
                        "archetype_node_id": "at0063",
                        "name": { "_type": "DV_TEXT", "value": "Receiver order identifier" },
                        "value": identifier(order, order_type)
                    }]
                }]
            }
        }]
    })
}

#[test]
fn a_checksum_valid_digit_run_in_laboratory_content_is_not_a_finding() {
    let default = policy(&PrivacyConfig::default());
    for order in CHECKSUM_VALID_RUNS {
        for kind in [None, Some("filler_order_number"), Some("NHS Number")] {
            assert_eq!(
                default.findings("COMPOSITION", &laboratory_result(order, kind)),
                Vec::<Finding>::new(),
                "{kind:?}"
            );
        }
    }
}
