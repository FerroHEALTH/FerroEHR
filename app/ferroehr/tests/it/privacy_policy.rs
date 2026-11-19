// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The clinical-side data-minimisation policy end to end
//! (`ferroehr::privacy`): the subject-reference rule and the identified-party
//! rule, as the write path enforces them, and the clinical values neither
//! rule reads.
//!
//! The write path is the single enforcement point. Neither rule is a
//! structural invariant of the schema — each varies with configuration — so
//! none is a database constraint.
//!
//! No openEHR spec governs the policy — our own design/extension (GDPR
//! Art. 4(5) + Art. 25(2), <https://eur-lex.europa.eu/eli/reg/2016/679/oj>).
//! The wire outcome IS spec-governed: a well-formed body refused for a
//! semantic reason is `422` (ITS-REST
//! `specifications/docs/overview/Requests_and_responses.md` §HTTP status
//! codes), which the service reports as the `content_invalid` status.
//!
//! Every identifier here is synthetic, constructed by running a published
//! algorithm forward over a chosen prefix. None is issued to anyone.

use ferroehr::service::ehr::object::EhrObject;
use ferroehr::service::version_update::WriteRequest;
use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::Row;

use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Invalid;
use ferroehr::privacy::PrivacyPolicy;
use ferroehr::privacy::config::PrivacyConfig;
use ferroehr::service::FerroEhrService;

use crate::fixtures::{composition, uv};

/// The pseudonym namespace these tests declare, and an opaque subject in it.
const PSEUDONYM_NS: &str = "urn:ferroehr:pseudonym";
const OPAQUE_SUBJECT: &str = "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3";

/// A nine-digit value that passes the eleven-test. Synthetic.
const SYNTHETIC_BSN: &str = "111222333"; // privacy-allow: a synthetic value, no register issues it

fn service(db: &testkit::TestDb, config: &PrivacyConfig) -> FerroEhrService {
    let policy = PrivacyPolicy::from(config);
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_privacy(Arc::new(policy))
}

/// The configuration a deployment that has declared its pseudonymisation
/// domain runs: every rule in force.
fn enforcing() -> PrivacyConfig {
    PrivacyConfig {
        subject_namespaces: vec![PSEUDONYM_NS.to_owned()],
        ..PrivacyConfig::default()
    }
}

fn status_with_subject(namespace: &str, id: &str) -> Value {
    json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": namespace,
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": id, "scheme": "pseudonym" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

fn typed_status(value: &Value) -> openehr_rm::prelude::EhrStatus {
    openehr_its::json::from_canonical_value(value).expect("the fixture EHR_STATUS decodes")
}

fn assert_refused(error: &ServiceError, expected_path: &str) {
    assert!(
        matches!(
            error,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "a privacy refusal is the 422-class content_invalid refusal, got {error:?}"
    );
    assert!(
        error.message().contains(expected_path),
        "the refusal names the offending RM path; got: {}",
        error.message()
    );
}

/// The same assertion over the service-layer error the composition routes
/// return before the status boundary converts it.
///
/// A privacy refusal travels as [`ServiceError::ValidationFailed`], the
/// per-path vehicle, so each finding keeps its own RM path all the way into the
/// ITS-REST `422` body's `validationErrors[]`.
fn assert_refused_service(error: &ServiceError, expected_path: &str) {
    let ServiceError::ValidationFailed(violations) = error else {
        panic!("a privacy refusal is ValidationFailed, got {error:?}");
    };
    assert!(
        violations.iter().any(|v| v.path == expected_path),
        "the refusal keys the finding by its RM path; got {:?}",
        violations.iter().map(|v| &v.path).collect::<Vec<_>>()
    );
}

// ── the subject reference ─────────────────────────────────────────────────────

#[tokio::test]
async fn an_opaque_subject_in_a_configured_namespace_is_stored_as_the_promoted_key() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let ehr_id = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(typed_status(
        &status_with_subject(PSEUDONYM_NS, OPAQUE_SUBJECT),
    )))))
    .await
    .map(|(id, _)| id)
    .expect("an opaque subject in a declared namespace is accepted");

    let row = sqlx::query("SELECT subject_id, subject_namespace FROM ehr WHERE id = $1")
        .bind(ehr_id)
        .fetch_one(&db.pool())
        .await
        .expect("the promoted subject columns");
    assert_eq!(
        row.get::<Option<String>, _>("subject_id").as_deref(),
        Some(OPAQUE_SUBJECT)
    );
    assert_eq!(
        row.get::<Option<String>, _>("subject_namespace").as_deref(),
        Some(PSEUDONYM_NS)
    );
}

#[tokio::test]
async fn a_non_uuid_subject_id_is_refused_and_the_value_never_reaches_the_wire() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let error = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(typed_status(
        &status_with_subject(PSEUDONYM_NS, SYNTHETIC_BSN),
    )))))
    .await
    .map(|(id, _)| id)
    .expect_err("a national identifier as the subject must be refused");
    assert_refused(&error, "EHR_STATUS/subject/external_ref/id/value");
    assert!(
        !error.message().contains(SYNTHETIC_BSN),
        "the refusal must name the shape, never the value: {}",
        error.message()
    );
}

#[tokio::test]
async fn an_unconfigured_namespace_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let error = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(typed_status(
        &status_with_subject("uk.org.nmc", OPAQUE_SUBJECT),
    )))))
    .await
    .map(|(id, _)| id)
    .expect_err("a namespace outside the pseudonymisation domain must be refused");
    assert_refused(&error, "EHR_STATUS/subject/external_ref/namespace");
}

#[tokio::test]
async fn the_subject_rule_also_binds_the_ehr_status_update_path() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let ehr_id = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(typed_status(
        &status_with_subject(PSEUDONYM_NS, OPAQUE_SUBJECT),
    )))))
    .await
    .map(|(id, _)| id)
    .expect("create");
    let status = svc
        .ehr_object(ehr_id, EhrObject::Status, None)
        .await
        .expect("read the status")
        .body;
    let preceding = status["uid"]["value"]
        .as_str()
        .expect("the stored status carries its version uid")
        .to_owned();

    let error = Box::pin(svc.replace_ehr_status(
        ehr_id,
        uv(
            &status_with_subject(PSEUDONYM_NS, SYNTHETIC_BSN),
            "251",
            Some(&preceding),
        ),
    ))
    .await
    .map(|m| m.uid)
    .expect_err("the update path enforces the same rule as the create path");
    assert_refused(&error, "EHR_STATUS/subject/external_ref/id/value");
}

// ── identified parties ────────────────────────────────────────────────────────

/// The write path takes a named composer under the shipped default — the
/// accepted cell of the matrix, proven through the service rather than the
/// rule.
///
/// `composition()` composes as a `PARTY_IDENTIFIED` carrying a name, which is
/// what `ctx/composer_name` and this server's own example generator produce.
/// The class is the proxy for a party "other than the subject of the record",
/// "Typically for health care providers" (RM common
/// `UML/classes/org.openehr.rm.common.party_identified.adoc` §Description).
#[tokio::test]
async fn a_named_composer_is_accepted_by_default() {
    let db = testkit::db().await.expect("testkit database");
    let strict = service(&db, &PrivacyConfig::default());
    let ehr_id = crate::fixtures::create_ehr(&strict).await;

    let named = composition("privacy composition");
    strict
        .create_composition(ehr_id, uv(&named, "249", None))
        .await
        .expect("a provider name on the composer is not the boundary's business");
}

/// The write path takes a composer's formal identifiers under the shipped
/// default: `PARTY_IDENTIFIED` is "Used to describe parties where only
/// identifiers may be known … e.g. name and provider number of an
/// institution" (RM common `UML/classes/org.openehr.rm.common.party_identified.adoc`
/// §Description), so a registration number on the composer is the class's
/// paradigm case (#3254). The refused cell is the party that IS the subject:
/// a `PARTY_RELATED` whose relationship is `self`, taken only under the opt-in.
#[tokio::test]
async fn composer_identifiers_are_accepted_and_a_self_party_needs_the_opt_in() {
    let db = testkit::db().await.expect("testkit database");
    let strict = service(&db, &PrivacyConfig::default());
    let ehr_id = crate::fixtures::create_ehr(&strict).await;

    let mut identified = composition("privacy composition");
    identified["composer"]["identifiers"] =
        json!([{ "_type": "DV_IDENTIFIER", "id": "GMC-1234567", "issuer": "GMC" }]);
    strict
        .create_composition(ehr_id, uv(&identified, "249", None))
        .await
        .expect("a provider's formal identifier on the composer is the class's paradigm case");

    let mut subject = composition("privacy composition self");
    subject["composer"] = json!({
        "_type": "PARTY_RELATED",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "MRN-0001", "issuer": "hospital" }],
        "relationship": {
            "_type": "DV_CODED_TEXT",
            "value": "self",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "0"
            }
        }
    });
    let error = strict
        .create_composition(ehr_id, uv(&subject, "249", None))
        .await
        .expect_err("the subject's own formal identifiers are refused by default");
    assert_refused_service(&error, "COMPOSITION/composer/identifiers");

    let permissive = service(
        &db,
        &PrivacyConfig {
            allow_identified_parties_in_ehr: true,
            ..PrivacyConfig::default()
        },
    );
    permissive
        .create_composition(ehr_id, uv(&subject, "249", None))
        .await
        .expect("the documented opt-in accepts it");
}

// ── clinical digit runs ───────────────────────────────────────────────────────

/// A valid COMPOSITION whose one free-text element carries `text`.
fn composition_with_text(name: &str, text: &str) -> Value {
    let mut composition = composition(name);
    composition["composer"] = json!({
        "_type": "PARTY_IDENTIFIED",
        "external_ref": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": OPAQUE_SUBJECT }
        }
    });
    composition["content"] = json!([{
        "_type": "EVALUATION",
        "archetype_node_id": "openEHR-EHR-EVALUATION.clinical_synopsis.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EVALUATION.clinical_synopsis.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "synopsis" },
        "language": { "_type": "CODE_PHRASE",
                      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
                      "code_string": "en" },
        "encoding": { "_type": "CODE_PHRASE",
                      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" },
                      "code_string": "UTF-8" },
        "subject": { "_type": "PARTY_SELF" },
        "data": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "tree" },
            "items": [{
                "_type": "ELEMENT",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "synopsis" },
                "value": { "_type": "DV_TEXT", "value": text }
            }]
        }
    }]);
    composition
}

/// Digit runs a laboratory carries that pass a national checksum by chance,
/// starting with the filler order number a partner reported (OBR-3 and ORC-3,
/// ten digits, a YYMMDD reading and a Luhn pass), with two order numbers
/// passing the NHS Number modulus 11 and runs passing the eleven-test and the
/// Norwegian and Swiss checks (#6081).
const CHECKSUM_VALID_RUNS: [&str; 6] = [
    "9702311805", // privacy-allow: a laboratory filler order number, not a person identifier
    "8412501861", // privacy-allow: a laboratory order number, not a person identifier
    "8372500932", // privacy-allow: a laboratory order number, not a person identifier
    SYNTHETIC_BSN,
    "15038545660",   // privacy-allow: synthetic, from the published algorithm
    "7561234567897", // privacy-allow: synthetic, built to satisfy the GS1 modulo-10 check
];

/// A lab result carrying `id` typed `kind` in the feeder audit's
/// originating-system item ids and as the order-id element.
fn lab_result(id: &str, kind: &str) -> Value {
    let identifier = json!({
        "_type": "DV_IDENTIFIER", "id": id, "type": kind, "assigner": "lab.example"
    });
    let mut composition = composition_with_text("lab result", "a laboratory result");
    composition["feeder_audit"] = json!({
        "_type": "FEEDER_AUDIT",
        "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "lab.example" },
        "originating_system_item_ids": [
            { "_type": "DV_IDENTIFIER", "id": "CL2509-0042", "type": "placer_order_number" },
            identifier.clone()
        ]
    });
    composition["content"][0]["data"]["items"][0]["value"] = identifier;
    composition
}

/// A `laboratory_test_result.v1` observation whose "Receiver order
/// identifier" element (at0063, under "Test request details", at0094) is
/// named `element_name` and carries `id` with no `type`.
fn laboratory_test_result(id: &str, element_name: &str) -> Value {
    let mut composition = composition_with_text("laboratory report", "Full blood count");
    let archetype = "openEHR-EHR-OBSERVATION.laboratory_test_result.v1";
    let entry = &mut composition["content"][0];
    entry["_type"] = json!("OBSERVATION");
    entry["archetype_node_id"] = json!(archetype);
    entry["archetype_details"]["archetype_id"]["value"] = json!(archetype);
    entry["name"] = json!({ "_type": "DV_TEXT", "value": "Laboratory test result" });
    if let Some(object) = entry.as_object_mut() {
        object.remove("data");
    }
    entry["data"] = json!({
        "_type": "HISTORY",
        "archetype_node_id": "at0001",
        "name": { "_type": "DV_TEXT", "value": "Event Series" },
        "origin": { "_type": "DV_DATE_TIME", "value": "2026-11-02T08:15:00Z" },
        "events": [{
            "_type": "POINT_EVENT",
            "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "Any event" },
            "time": { "_type": "DV_DATE_TIME", "value": "2026-11-02T08:15:00Z" },
            "data": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "Tree" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0005",
                    "name": { "_type": "DV_TEXT", "value": "Test name" },
                    "value": { "_type": "DV_TEXT", "value": "Full blood count" }
                }]
            }
        }]
    });
    entry["protocol"] = json!({
        "_type": "ITEM_TREE",
        "archetype_node_id": "at0004",
        "name": { "_type": "DV_TEXT", "value": "Tree" },
        "items": [{
            "_type": "CLUSTER",
            "archetype_node_id": "at0094",
            "name": { "_type": "DV_TEXT", "value": "Test request details" },
            "items": [{
                "_type": "ELEMENT",
                "archetype_node_id": "at0062",
                "name": { "_type": "DV_TEXT", "value": "Requester order identifier" },
                "value": { "_type": "DV_IDENTIFIER", "id": "CL2509-0042" }
            }, {
                "_type": "ELEMENT",
                "archetype_node_id": "at0063",
                "name": { "_type": "DV_TEXT", "value": element_name },
                "value": { "_type": "DV_IDENTIFIER", "id": id }
            }]
        }]
    });
    composition
}

/// A clinical commit carrying a checksum-valid digit run is stored as written,
/// wherever the run sits: a typed order identifier with the feeder audit's
/// item ids, an untyped order identifier element, free text and a coded value.
/// The policy reads the RM structure and guesses nothing from a value's shape
/// (#6081).
#[tokio::test]
async fn a_laboratory_commit_carrying_checksum_valid_digit_runs_is_stored() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let ehr_id = crate::fixtures::create_ehr(&svc).await;
    for run in CHECKSUM_VALID_RUNS {
        let mut coded = composition_with_text("coded", "a clinical synopsis");
        coded["content"][0]["data"]["items"][0]["value"] = json!({
            "_type": "DV_CODED_TEXT",
            "value": "Laboratory order",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": run
            }
        });
        for body in [
            lab_result(run, "filler_order_number"),
            laboratory_test_result(run, "Receiver order identifier"),
            composition_with_text("laboratory note", &format!("filler order {run}")),
            coded,
        ] {
            let committed = svc
                .create_composition(ehr_id, uv(&body, "249", None))
                .await
                .unwrap_or_else(|e| panic!("{run}: a laboratory value is stored: {e:?}"));
            let stored = svc
                .read_composition_raw(ehr_id, committed.vo_id, None)
                .await
                .expect("the stored COMPOSITION reads")
                .body
                .into_value()
                .expect("the stored COMPOSITION decodes");
            assert!(
                serde_json::to_string(&stored)
                    .expect("the stored COMPOSITION renders")
                    .contains(run),
                "{run} is stored as written"
            );
        }
    }
}
