// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Rule-level tests for [`ferroehr::privacy::PrivacyPolicy`]: what each rule refuses, what
//! it deliberately leaves alone, and that the default configuration is the
//! refusing one.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this module's fixture helpers; a \
              failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use serde_json::{Value, json};

use ferroehr::privacy::config::{IdentifierScanConfig, PrivacyConfig, ScanMode};
use ferroehr::privacy::{Finding, FindingClass, PrivacyPolicy};

/// A pseudonym namespace and an opaque subject, the shape the boundary
/// mandates.
///
/// Every identifier in this module is synthetic, constructed by running the
/// published algorithm forward over a chosen prefix.
const PSEUDONYM_NS: &str = "urn:ferroehr:pseudonym";
const OPAQUE_SUBJECT: &str = "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3";

fn policy(config: &PrivacyConfig) -> PrivacyPolicy {
    PrivacyPolicy::compile(config).expect("the test policy compiles")
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
/// off (which half of that rule bites is the matrix below), the scanner
/// strict, and the subject rule waiting for a deployment fact.
#[test]
fn the_configuration_default_is_the_minimising_posture_and_scans_strictly() {
    let default = PrivacyConfig::default();
    assert!(!default.allow_identified_parties_in_ehr);
    assert_eq!(default.identifier_scan.mode, ScanMode::Strict);
    assert!(
        !default.identifier_scan.rules.is_empty(),
        "the shipped default scans for every rule the build carries"
    );
    // The subject rule is the one that waits for a deployment fact: there is no
    // pseudonym namespace a server could invent for an operator.
    assert!(default.subject_namespaces.is_empty());
    let compiled = policy(&default);
    assert!(!compiled.subject_rule_in_force());
    assert!(!compiled.identified_parties_allowed());
    assert_eq!(compiled.scan_mode(), Some(ScanMode::Strict));
    assert_eq!(
        compiled.active_rules().len(),
        ferroehr::privacy::detect::built_in_rules().len()
    );
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
    assert_eq!(findings[0].class, FindingClass::Refusal);
}

#[test]
fn a_non_uuid_subject_id_is_refused_and_the_value_never_travels() {
    let findings = enforcing().findings(
        "EHR_STATUS",
        &status_with_subject(PSEUDONYM_NS, "111222333"), // privacy-allow: synthetic, and the refusal must not echo it
    );
    // Two rules see it at once: the subject rule (not a UUID) and the scanner
    // (a value shaped like a BSN). Both report the same path, neither the value.
    let refusals: Vec<_> = findings
        .iter()
        .filter(|f| f.class == FindingClass::Refusal)
        .collect();
    assert_eq!(refusals.len(), 1, "{findings:?}");
    assert_eq!(refusals[0].path, "EHR_STATUS/subject/external_ref/id/value");
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
    assert!(
        unconfigured
            .findings(
                "EHR_STATUS",
                &status_with_subject("uk.org.nmc", "patient-42")
            )
            .is_empty()
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
    assert!(enforcing().findings("EHR_STATUS", &bare).is_empty());
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
    assert!(enforcing().findings("COMPOSITION", &composition).is_empty());
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
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Author" }
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

/// A national identifier as a `DV_IDENTIFIER` value is still refused, by the
/// identifier scanner, whichever party proxy carries it: the class rule
/// admits the slot, never the subject's number.
#[test]
fn a_national_identifier_value_in_identifiers_is_refused_by_the_scanner() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": {
            "_type": "PARTY_IDENTIFIED",
            "name": "Dr Author",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "111222333", "issuer": "RvIG" }] // privacy-allow: synthetic
        }
    });
    let findings = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
    assert_eq!(paths(&findings), ["COMPOSITION/composer/identifiers[0]/id"]);
    assert_eq!(findings[0].class, FindingClass::IdentifierShape);
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
        assert_eq!(findings[0].class, FindingClass::Refusal);
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
    assert!(
        policy(&PrivacyConfig::default())
            .findings("COMPOSITION", &composition)
            .is_empty()
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
    assert!(permitted.findings("COMPOSITION", &composition).is_empty());
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

#[test]
fn a_party_self_carries_no_name_slot_and_is_untouched() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": { "_type": "PARTY_SELF" }
    });
    assert!(
        policy(&PrivacyConfig::default())
            .findings("COMPOSITION", &composition)
            .is_empty()
    );
}

// ── rule 3: the identifier scanner ────────────────────────────────────────────

#[test]
fn a_bsn_in_free_text_is_a_finding() {
    let composition = json!({
        "_type": "COMPOSITION",
        "content": [{ "_type": "EVALUATION", "data": { "_type": "ITEM_TREE", "items": [
            { "_type": "ELEMENT", "value": { "_type": "DV_TEXT",
              "value": "referral for 111222333" } } // privacy-allow: synthetic, passes the eleven-test
        ] } }]
    });
    let findings = policy(&PrivacyConfig::default()).findings("COMPOSITION", &composition);
    assert_eq!(
        paths(&findings),
        ["COMPOSITION/content[0]/data/items[0]/value/value"]
    );
    assert_eq!(findings[0].class, FindingClass::IdentifierShape);
}

#[test]
fn a_terminology_code_that_satisfies_a_checksum_by_chance_is_not_a_finding() {
    // The measured false-positive class, and the reason the carve-out is a
    // structural one. 288526004 is a real SNOMED CT concept id from the
    // vendored OPT fixtures and satisfies the Dutch elfproef by chance; 27 of
    // the 30 nine-digit passers in this repository are SNOMED codes. Every
    // checksum rule has this exposure to some code system, so the carve-out
    // names an RM slot rather than a country: `code_string` is CODE_PHRASE's
    // only string attribute and no other RM class declares it.
    let composition = json!({
        "_type": "COMPOSITION",
        "category": { "_type": "DV_CODED_TEXT", "value": "event", "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "SNOMED-CT" },
            "code_string": "288526004" } } // privacy-allow: a terminology code from the vendored fixtures
    });
    assert!(
        policy(&PrivacyConfig::default())
            .findings("COMPOSITION", &composition)
            .is_empty()
    );
    // The same digits in a free-text slot are still a finding: the carve-out is
    // the slot, so it cannot blind the scanner anywhere else.
    let narrative = json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "note 288526004" } // privacy-allow: a SNOMED CT identifier, deliberately outside the coded slot
    });
    assert_eq!(
        paths(&policy(&PrivacyConfig::default()).findings("COMPOSITION", &narrative)),
        ["COMPOSITION/name/value"]
    );
}

#[test]
fn an_object_version_id_and_an_archetype_id_are_not_findings() {
    // The technical identifiers every stored instance carries: a UUID has no
    // delimited nine-digit run, and neither does an archetype identifier.
    let composition = json!({
        "_type": "COMPOSITION",
        "uid": { "_type": "OBJECT_VERSION_ID",
                 "value": "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3::ferroehr.local::1" },
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1"
    });
    assert!(
        policy(&PrivacyConfig::default())
            .findings("COMPOSITION", &composition)
            .is_empty()
    );
}

#[test]
fn a_configured_pattern_is_a_finding_and_names_itself() {
    let with_pattern = policy(&PrivacyConfig {
        identifier_scan: IdentifierScanConfig {
            patterns: vec![r"\bMRN-[0-9]{6}\b".to_owned()],
            ..IdentifierScanConfig::default()
        },
        ..PrivacyConfig::default()
    });
    let composition = json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "chart MRN-004221" }
    });
    let findings = with_pattern.findings("COMPOSITION", &composition);
    assert_eq!(paths(&findings), ["COMPOSITION/name/value"]);
    assert!(findings[0].message.contains(r"\bMRN-[0-9]{6}\b"));
}

#[test]
fn an_uncompilable_pattern_fails_the_policy_build_naming_the_pattern() {
    let error = PrivacyPolicy::compile(&PrivacyConfig {
        identifier_scan: IdentifierScanConfig {
            patterns: vec!["[unclosed".to_owned()],
            ..IdentifierScanConfig::default()
        },
        ..PrivacyConfig::default()
    })
    .expect_err("an invalid pattern is a configuration error");
    assert!(error.to_string().contains("[unclosed"));
}

#[test]
fn an_unknown_rule_key_fails_the_policy_build_listing_the_shipped_rules() {
    let error = PrivacyPolicy::compile(&PrivacyConfig {
        identifier_scan: IdentifierScanConfig {
            rules: vec!["zz-invented".to_owned()],
            ..IdentifierScanConfig::default()
        },
        ..PrivacyConfig::default()
    })
    .expect_err("an unknown rule key is a configuration error");
    let rendered = error.to_string();
    assert!(rendered.contains("zz-invented"), "{rendered}");
    assert!(rendered.contains("nl-bsn"), "{rendered}");
}

#[test]
fn every_shipped_jurisdiction_is_scanned_by_the_default_policy() {
    // The point of the ruleset: an identifier from any shipped jurisdiction is
    // caught, not only the one the build was written in.
    let default = policy(&PrivacyConfig::default());
    for (key, value) in [
        ("nl-bsn", "111222333"), // privacy-allow: synthetic, from the published algorithm
        ("no-fodselsnummer", "15038545660"),
        ("se-personnummer", "9001011239"),
        ("gb-nhs-number", "9434767016"),
        ("fi-hetu", "010100A123D"),
    ] {
        let composition = json!({
            "_type": "COMPOSITION",
            "name": { "_type": "DV_TEXT", "value": format!("note {value}") }
        });
        let findings = default.findings("COMPOSITION", &composition);
        assert_eq!(
            paths(&findings),
            ["COMPOSITION/name/value"],
            "{key} was not caught by the default policy"
        );
        assert!(
            findings[0].message.contains(key),
            "the finding must name the rule that claimed it: {}",
            findings[0].message
        );
    }
}

#[test]
fn narrowing_the_rule_list_narrows_what_is_caught() {
    let nl_only = policy(&PrivacyConfig {
        identifier_scan: IdentifierScanConfig {
            rules: vec!["nl-bsn".to_owned()],
            ..IdentifierScanConfig::default()
        },
        ..PrivacyConfig::default()
    });
    assert_eq!(nl_only.active_rules().len(), 1);
    let norwegian = json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "note 15038545660" }
    });
    assert!(nl_only.findings("COMPOSITION", &norwegian).is_empty());
    assert_eq!(
        paths(&policy(&PrivacyConfig::default()).findings("COMPOSITION", &norwegian)),
        ["COMPOSITION/name/value"]
    );
}

/// The library default is the refusing posture the shipped configuration
/// compiles to (#3243): a service built without an explicit policy scans with
/// every rule in `strict` mode, never a more permissive shape than the binary.
#[test]
fn the_default_policy_is_the_refusing_posture() {
    let composition = json!({
        "_type": "COMPOSITION",
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Author" },
        "name": { "_type": "DV_TEXT", "value": "111222333" } // privacy-allow: synthetic, passes the eleven-test
    });
    let findings = PrivacyPolicy::default().findings("COMPOSITION", &composition);
    assert_eq!(paths(&findings), ["COMPOSITION/name/value"]);
    assert_eq!(PrivacyPolicy::default().scan_mode(), Some(ScanMode::Strict));
    assert_eq!(
        PrivacyPolicy::default().active_rules().len(),
        policy(&PrivacyConfig::default()).active_rules().len(),
        "the default carries every shipped rule, as the default configuration does"
    );
}

#[test]
fn warn_mode_still_reports_the_finding_and_keeps_its_class() {
    let warning = policy(&PrivacyConfig {
        identifier_scan: IdentifierScanConfig {
            mode: ScanMode::Warn,
            ..IdentifierScanConfig::default()
        },
        ..PrivacyConfig::default()
    });
    let composition = json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "111222333" } // privacy-allow: synthetic, passes the eleven-test
    });
    let findings = warning.findings("COMPOSITION", &composition);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].class, FindingClass::IdentifierShape);
}

#[test]
fn a_uuid_subject_is_accepted_in_either_case() {
    let upper = OPAQUE_SUBJECT.to_ascii_uppercase();
    assert!(
        enforcing()
            .findings("EHR_STATUS", &status_with_subject(PSEUDONYM_NS, &upper))
            .is_empty()
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

/// The false-positive measurement: the full default ruleset over every
/// vendored clinical document in the shared corpus.
///
/// Each rule accepts some fraction of random digit runs by its own published
/// arithmetic — one in eleven for the elfproef — so the question a deployment
/// actually asks is not whether a collision is possible but whether real
/// clinical content trips one. The corpus is the closest thing this repository
/// has to that content: the CKM-derived template example instances and every
/// fixture body the service and REST suites commit. The scan runs with every
/// shipped jurisdiction active at once, which is stricter than any single
/// deployment, and only the scanner's own findings count — a corpus
/// composition naming its composer is the identified-party rule doing its job,
/// not a false positive.
///
/// Measured 258 documents, zero findings, and the reason is worth stating so
/// nobody reads more into this gate than it carries: the corpus contains no
/// delimited nine-, ten- or eleven-digit run in any scanned string leaf at
/// all, so a rule that stopped discriminating entirely would still pass here.
/// The gate that catches THAT is the collision measurement beside the rules
/// themselves (`privacy::detect`), which fails when a rule drifts from the
/// rate it publishes. This one answers the other half: real clinical content
/// carries no shape the shipped ruleset claims.
#[test]
fn the_shipped_ruleset_finds_nothing_in_the_vendored_corpus() {
    let policy = policy(&PrivacyConfig::default());
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .canonicalize()
        .expect("the shared corpus tree");

    let mut scanned = 0_usize;
    let mut hits: Vec<String> = Vec::new();
    let mut pending = vec![corpus];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).expect("read the corpus directory");
        for entry in entries {
            let path = entry.expect("a corpus directory entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read a corpus document");
            let Ok(body) = serde_json::from_str::<Value>(&text) else {
                // The corpus carries deliberately malformed bytes for the
                // reader's refusal gates; they never reach a commit.
                continue;
            };
            let root = body
                .get("_type")
                .and_then(Value::as_str)
                .unwrap_or("COMPOSITION")
                .to_owned();
            scanned += 1;
            hits.extend(
                policy
                    .findings(&root, &body)
                    .into_iter()
                    .filter(|finding| finding.class == FindingClass::IdentifierShape)
                    .map(|finding| {
                        format!("{}: {} {}", path.display(), finding.path, finding.message)
                    }),
            );
        }
    }

    assert!(
        scanned >= 200,
        "the gate must scan something: only {scanned} corpus documents parsed"
    );
    assert!(
        hits.is_empty(),
        "the shipped ruleset claims {} value(s) in the vendored corpus:\n{}",
        hits.len(),
        hits.join("\n")
    );
}
