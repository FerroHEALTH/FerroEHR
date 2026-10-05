// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The clinical-side data-minimisation policy end to end
//! (`ferroehr::privacy`): the subject-reference rule, the identified-party rule
//! and the jurisdiction-keyed identifier scanner, as the write path enforces
//! them.
//!
//! The write path is the single enforcement point. None of the three rules is a
//! structural invariant of the schema — each varies with configuration — so
//! none is a database constraint.
//!
//! No openEHR spec governs the policy — our own design/extension (GDPR
//! Art. 4(5) + Art. 25(2), <https://eur-lex.europa.eu/eli/reg/2016/679/oj>).
//! The wire outcome IS spec-governed: a well-formed body refused for a
//! semantic reason is `422` (ITS-REST
//! `specifications/docs/overview/Requests_and_responses.md` §HTTP status
//! codes), which the service reports as the SM `content_invalid` status.
//!
//! Every identifier here is synthetic, constructed by running a published
//! algorithm forward over a chosen prefix. None is issued to anyone.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this module's fixture helpers; a \
              failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::Row;

use ferroehr::privacy::PrivacyPolicy;
use ferroehr::privacy::config::{IdentifierScanConfig, PrivacyConfig, ScanMode};
use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;
use ferroehr::service::status::{CallStatusType, SmError};

use crate::fixtures::{composition, uv};

/// The pseudonym namespace these tests declare, and an opaque subject in it.
const PSEUDONYM_NS: &str = "urn:ferroehr:pseudonym";
const OPAQUE_SUBJECT: &str = "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3";

/// A value the `nl-bsn` rule claims. Synthetic.
const SYNTHETIC_BSN: &str = "111222333"; // privacy-allow: a synthetic value, no register issues it

/// A value the `no-fodselsnummer` rule claims — the same scanner, a different
/// jurisdiction. Synthetic.
const SYNTHETIC_FODSELSNUMMER: &str = "15038545660";

fn service(db: &testkit::TestDb, config: &PrivacyConfig) -> FerroEhrService {
    let policy = PrivacyPolicy::compile(config).expect("the test policy compiles");
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

fn assert_refused(error: &SmError, expected_path: &str) {
    assert_eq!(
        error.status,
        CallStatusType::ContentInvalid,
        "a privacy refusal is the 422-class content_invalid status, got {error:?}"
    );
    assert!(
        error.message.contains(expected_path),
        "the refusal names the offending RM path; got: {}",
        error.message
    );
}

/// Every finding message the service-layer error carries.
fn finding_messages(error: &ServiceError) -> String {
    let ServiceError::ValidationFailed(violations) = error else {
        panic!("a privacy refusal is ValidationFailed, got {error:?}");
    };
    violations
        .iter()
        .map(|v| format!("{}: {}", v.path, v.message))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The same assertion over the service-layer error the composition routes
/// return before the SM boundary converts it.
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
    let ehr_id = svc
        .create_ehr(Some(typed_status(&status_with_subject(
            PSEUDONYM_NS,
            OPAQUE_SUBJECT,
        ))))
        .await
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
    let error = svc
        .create_ehr(Some(typed_status(&status_with_subject(
            PSEUDONYM_NS,
            SYNTHETIC_BSN,
        ))))
        .await
        .expect_err("a national identifier as the subject must be refused");
    assert_refused(&error, "EHR_STATUS/subject/external_ref/id/value");
    assert!(
        !error.message.contains(SYNTHETIC_BSN),
        "the refusal must name the shape, never the value: {}",
        error.message
    );
}

#[tokio::test]
async fn an_unconfigured_namespace_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let error = svc
        .create_ehr(Some(typed_status(&status_with_subject(
            "uk.org.nmc",
            OPAQUE_SUBJECT,
        ))))
        .await
        .expect_err("a namespace outside the pseudonymisation domain must be refused");
    assert_refused(&error, "EHR_STATUS/subject/external_ref/namespace");
}

#[tokio::test]
async fn the_subject_rule_also_binds_the_ehr_status_update_path() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &enforcing());
    let ehr_id = svc
        .create_ehr(Some(typed_status(&status_with_subject(
            PSEUDONYM_NS,
            OPAQUE_SUBJECT,
        ))))
        .await
        .expect("create");
    let status = svc.get_ehr_status(ehr_id).await.expect("read the status");
    let preceding = status["uid"]["value"]
        .as_str()
        .expect("the stored status carries its version uid")
        .to_owned();

    let error = svc
        .replace_ehr_status(
            ehr_id,
            uv(
                &status_with_subject(PSEUDONYM_NS, SYNTHETIC_BSN),
                "251",
                Some(&preceding),
            ),
        )
        .await
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
    let ehr_id = strict.create_ehr(None).await.expect("create_ehr");

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
    let ehr_id = strict.create_ehr(None).await.expect("create_ehr");

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

// ── the identifier scanner ────────────────────────────────────────────────────

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

#[tokio::test]
async fn strict_mode_refuses_a_composition_carrying_a_synthetic_bsn() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &PrivacyConfig::default());
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");

    let carrying = composition_with_text(
        "scanner strict",
        &format!("referral note for {SYNTHETIC_BSN}"),
    );
    let error = svc
        .create_composition(ehr_id, uv(&carrying, "249", None))
        .await
        .expect_err("strict mode refuses an identifier-shaped value");
    assert_refused_service(&error, "COMPOSITION/content[0]/data/items[0]/value/value");
    let messages = finding_messages(&error);
    assert!(
        !messages.contains(SYNTHETIC_BSN),
        "the refusal must name the rule, never the value: {messages}"
    );
}

#[tokio::test]
async fn warn_mode_accepts_the_same_composition() {
    let db = testkit::db().await.expect("testkit database");
    let svc = service(
        &db,
        &PrivacyConfig {
            identifier_scan: IdentifierScanConfig {
                mode: ScanMode::Warn,
                ..IdentifierScanConfig::default()
            },
            ..PrivacyConfig::default()
        },
    );
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");
    let carrying = composition_with_text(
        "scanner warn",
        &format!("referral note for {SYNTHETIC_BSN}"),
    );
    svc.create_composition(ehr_id, uv(&carrying, "249", None))
        .await
        .expect("warn mode records the finding and lets the write through");
}

#[tokio::test]
async fn an_identifier_from_another_jurisdiction_is_refused_by_the_same_default() {
    // The scanner is not built around one country: the default ruleset covers
    // every jurisdiction the build ships.
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &PrivacyConfig::default());
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");

    let carrying = composition_with_text(
        "scanner cross-border",
        &format!("referral from Norway, {SYNTHETIC_FODSELSNUMMER}"),
    );
    let error = svc
        .create_composition(ehr_id, uv(&carrying, "249", None))
        .await
        .expect_err("a Norwegian identity number is refused by the same default");
    assert_refused_service(&error, "COMPOSITION/content[0]/data/items[0]/value/value");
    let messages = finding_messages(&error);
    assert!(
        messages.contains("no-fodselsnummer"),
        "the refusal names the rule that claimed it: {messages}"
    );
}

#[tokio::test]
async fn a_terminology_code_satisfying_a_checksum_is_not_refused() {
    // The measured false-positive class: 27 of the 30 nine-digit values in this
    // repository that satisfy the Dutch elfproef are SNOMED CT concept
    // identifiers. The scanner skips `code_string`, which is CODE_PHRASE's only
    // string attribute — a structural carve-out, not a per-country one.
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &PrivacyConfig::default());
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");

    let mut coded = composition_with_text("coded", "a clinical synopsis");
    coded["content"][0]["data"]["items"][0]["value"] = json!({
        "_type": "DV_CODED_TEXT",
        "value": "Hypertensive disorder",
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "SNOMED-CT" },
            // privacy-allow: a terminology code from the vendored OPT fixtures
            "code_string": "288526004"
        }
    });
    svc.create_composition(ehr_id, uv(&coded, "249", None))
        .await
        .expect("a SNOMED code is clinical data, not an identifier");
}
