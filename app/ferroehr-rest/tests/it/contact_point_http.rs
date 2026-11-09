// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The MyHealth@EU contact-point route at the data holder (#5102), driven
//! through the assembled router over a real `FerroEhrService` on a real
//! `PostgreSQL` 18, every request sender-constrained to a client certificate.
//!
//! A contact point's caller reaches patient data only for the patient of its
//! treatment relationship confirmation, resolved through the EHR index; every
//! other EHR gets one `404` `trc-patient-mismatch` answer carrying the
//! service's `*_NOT_FOUND` code (eHDSI XCA Profile 8.0.1 §4.2.1, §4.2.2), and
//! it discards a dispensation of its own point of care within the agreement's
//! window (MyHealth@EU Requirements Catalogue 10.0.0, requirement 07.04).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use ferroehr::config::agreement::{Accessor, Agreement, EhdsiService};
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use ferrohealth_server::section::CallerRate;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::federation::{FederationSection, Field, Mapping, Source};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::sender::proof::DpopSection;
use ferrohealth_smart::sender::{ClientCertificate, SenderConstraint};
use http::StatusCode;
use openehr_rm::prelude::EhrStatus;
use serde_json::{Value, json};

use crate::common::{self, BASE};

/// NCPeH-A's authorization server, a MyHealth@EU contact point.
const CONTACT_POINT: &str = "https://ncpeh-a.example";
/// This node's audience at the contact point.
const AUDIENCE: &str = "ferroehr-contact-point";

/// The assigning authority of the TRC patients.
const NAMESPACE: &str = "urn:oid:2.999.1";
/// The point of care the contact point's tokens name.
const PROVIDER: &str = "urn:oid:1.2.3.4";
/// The issuing authority the contact point's table maps for the provider.
const AUTHORITY: &str = "NCP-A Registry";
/// The dispensation template.
const DISPENSATION: &str = "International Patient Summary";
/// The issuer of the HP Identity Assertion, NCP-B.
const IDA_ISSUER: &str = "https://ncp-b.example";
/// The format of the HP Identity Assertion's `NameID`.
const NAME_ID_FORMAT: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:X509SubjectName";
/// The HP Identity Assertion's `NameID`.
const NAME_ID: &str = "CN=Synthetic Pharmacist,O=Pharmacy,C=BE";
/// The scopes a contact point's caller holds.
const SCOPE: &str = "patient/composition-*.crud";

/// The certificate every token is bound to.
fn certificate() -> ClientCertificate {
    ClientCertificate::from_der(b"synthetic client certificate DER")
}

/// The contact point's person agreement for ePrescription, reading and
/// writing the dispensation, with `discard` over `hours`.
fn eprescription(hours: u32) -> Agreement {
    Agreement {
        issuer: CONTACT_POINT.to_owned(),
        accessor: Some(Accessor::Person),
        reads: vec![SCOPE.to_owned()],
        writes: vec![DISPENSATION.to_owned()],
        discard: true,
        discard_window: ferrohealth_config::units::Duration::from_hours(u64::from(hours)),
        ehdsi_service: Some(EhdsiService::Eprescription),
        caller_rate: Some(CallerRate {
            requests_per_second: std::num::NonZeroU32::MIN.saturating_add(99),
            burst: std::num::NonZeroU32::MIN.saturating_add(99),
        }),
        ..Agreement::default()
    }
}

/// The router trusting the contact point under `agreement`: a
/// sender-constrained `federation` issuer with the eHDSI mapping, its table
/// mapping the professional's country and authority and the provider's
/// authority, over the local EHR index, with the dispensation template held.
async fn serve(agreement: Agreement) -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) =
        start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_ehr_index(EhrIndexProvider::Local)
            .with_audit(sender)
            .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool.clone())),
    );
    let constant = |value: &str| vec![Source::Constant(value.to_owned())];
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            issuer: CONTACT_POINT.to_owned(),
            kind: Some(IssuerKind::Federation),
            audience: vec![AUDIENCE.to_owned()],
            jwks_file: common::jwks_file().display().to_string(),
            sender_constraint: SenderConstraint::Required,
            ..IssuerEntry::default()
        }],
        federation: vec![FederationSection {
            issuer: CONTACT_POINT.to_owned(),
            contact_point: Some(true),
            mapping: Some(Mapping::Ehdsi),
            fields: BTreeMap::from([
                (Field::ProfessionalCountryCode, constant("BE")),
                (
                    Field::ProfessionalIssuingAuthorityName,
                    constant("NCP-B Belgium"),
                ),
                (Field::ProviderIssuingAuthorityName, constant(AUTHORITY)),
            ]),
            countries: vec!["BE".to_owned()],
            ..FederationSection::default()
        }],
        dpop: Some(DpopSection {
            external_url: "https://ehr.example".to_owned(),
            ..DpopSection::default()
        }),
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        auth,
        agreements: vec![agreement],
        ..common::api_config(true)
    };
    config.auth.smart.patient_issuers = vec![CONTACT_POINT.to_owned()];
    let app = ferroehr_rest::build_full(config, Arc::clone(&svc), None, Observability::default())
        .await
        .expect("build app");
    svc.template_adl14_upload(&common::ips_opt_xml())
        .await
        .expect("upload the IPS template");
    (db, svc, app)
}

/// Creates the EHR whose `EHR_STATUS` names `id` under [`NAMESPACE`].
async fn patient_ehr(svc: &FerroEhrService, id: &str) -> String {
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": {
            "_type": "PARTY_REF", "namespace": NAMESPACE, "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } } },
        "is_queryable": true,
        "is_modifiable": true
    });
    let status =
        openehr_its::json::from_canonical_value::<EhrStatus>(&status).expect("an EHR_STATUS");
    let (ehr_id, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(status))))
        .await
        .expect("the EHR commits");
    ehr_id.to_string()
}

/// A contact-point token bound to [`certificate`], its `ehdsi` claims
/// carrying the TRC patient `patient` and the point of care `provider`, with
/// `extra` merged into the `professional` claims.
fn token(patient: Option<&str>, provider: &str, extra: &Value) -> String {
    let now = jiff::Timestamp::now().as_second();
    let mut professional = json!({
        "hp_identifier": NAME_ID,
        "name": "Synthetic Pharmacist",
        "hp_professional_role": [{ "system": "2.16.840.1.113883.2.9.6.2.7", "code": "2262" }],
        "authentication_method": ferrohealth_smart::federation::contact::AUTHENTICATION_CLASSES[4]
    });
    if let (Some(professional), Some(extra)) = (professional.as_object_mut(), extra.as_object()) {
        professional.extend(extra.clone());
    }
    let mut ehdsi = json!({
        "professional": professional,
        "provider": {
            "healthcare_provider_identifier": provider,
            "facility_type": "Pharmacy",
            "home_community_id": "urn:oid:1.2.3"
        },
        "point_of_care": "Synthetic Pharmacy",
        "purpose_of_use": "TREATMENT",
        "assertion": {
            "issuer": IDA_ISSUER,
            "id": "urn:uuid:ida-1",
            "name_id_format": NAME_ID_FORMAT,
            "not_before": now - 60,
            "not_on_or_after": now + 3600
        },
        "trc": { "id": "urn:uuid:trc-1", "not_before": now - 60, "not_on_or_after": now + 3600 }
    });
    if let Some(patient) = patient {
        ehdsi["patient"] = json!({ "id": format!("{NAMESPACE}|{patient}") });
    }
    common::bearer(&json!({
        "iss": CONTACT_POINT, "sub": "hp-nameid", "client_id": "ncpeh-b", "aud": AUDIENCE,
        "exp": now + 600, "scope": SCOPE,
        "cnf": { "x5t#S256": certificate().thumbprint() },
        "ehdsi": ehdsi
    }))
}

/// Sends `method` on `path` under the API base as `credential` over the
/// bound connection, with `body`; the status and the body.
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    credential: &str,
    body: Option<&Value>,
) -> (StatusCode, String) {
    let mut request = common::req(method, format!("{BASE}{path}"))
        .auth(credential)
        .accept("application/json");
    if let Some(body) = body {
        request = request.json(body.to_string()).prefer_representation();
    }
    let mut request = request.build();
    request.extensions_mut().insert(certificate());
    common::send_body(app, request).await
}

/// The dispensation, recorded at `identifiers`.
fn dispensation(identifiers: &[(&str, &str)]) -> Value {
    let mut composition = common::ips_canonical_composition();
    let identifiers: Vec<Value> = identifiers
        .iter()
        .map(|(issuer, id)| json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id }))
        .collect();
    composition["context"]["health_care_facility"] = json!({
        "_type": "PARTY_IDENTIFIED", "name": "Synthetic Pharmacy", "identifiers": identifiers });
    composition
}

/// The point of care of a dispensation at `provider`: its identifier under
/// the authority, and the eHDSI `organization-id`.
fn point_of_care(provider: &str) -> Vec<(&'static str, &str)> {
    vec![
        (AUTHORITY, provider),
        (
            ferroehr::versioning::committer::external::ORGANIZATION_ID_ISSUER,
            provider,
        ),
    ]
}

/// Commits a dispensation at `identifiers` into `ehr` as `credential`; the
/// status and the version id it answered, or its body.
async fn dispense(
    app: &Router,
    ehr: &str,
    credential: &str,
    identifiers: &[(&str, &str)],
) -> (StatusCode, String) {
    let (status, body) = send(
        app,
        "POST",
        &format!("/ehr/{ehr}/composition"),
        credential,
        Some(&dispensation(identifiers)),
    )
    .await;
    let version = serde_json::from_str::<Value>(&body).ok().and_then(|c| {
        c.pointer("/uid/value")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    (status, version.unwrap_or(body))
}

#[tokio::test]
async fn a_contact_point_reads_only_the_ehr_of_its_trc_patient() {
    let (db, svc, app) = serve(eprescription(24)).await;
    let mine = patient_ehr(&svc, "SYNTHETIC-1").await;
    let theirs = patient_ehr(&svc, "SYNTHETIC-2").await;
    let subjectless = Box::pin(svc.create_ehr_meta(WriteRequest::new(None)))
        .await
        .map(|(id, _)| id.to_string())
        .expect("an EHR naming no patient");
    let unknown = "0190e6c4-0000-7000-8000-000000000000";
    let credential = token(Some("SYNTHETIC-1"), PROVIDER, &json!({}));
    let (status, body) = send(&app, "GET", &format!("/ehr/{mine}"), &credential, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Another patient's EHR and an EHR this server does not hold answer alike.
    let (status, other) = send(&app, "GET", &format!("/ehr/{theirs}"), &credential, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{other}");
    assert!(other.contains("trc-patient-mismatch"), "{other}");
    assert!(other.contains("ERROR_EP_NOT_FOUND"), "{other}");
    let (status, absent) = send(&app, "GET", &format!("/ehr/{unknown}"), &credential, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{absent}");
    let strip = |body: &str| {
        let mut problem: Value = serde_json::from_str(body).expect("a problem document");
        problem
            .as_object_mut()
            .map(|members| members.remove("instance"));
        problem
    };
    assert_eq!(strip(&other), strip(&absent), "one answer for both");
    let (status, body) = send(
        &app,
        "GET",
        &format!("/ehr/{subjectless}"),
        &credential,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(strip(&body), strip(&absent), "an EHR naming no patient too");

    // A TRC patient no EHR belongs to reaches nothing.
    let stranger = token(Some("SYNTHETIC-9"), PROVIDER, &json!({}));
    let (status, body) = send(&app, "GET", &format!("/ehr/{mine}"), &stranger, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.contains("trc-patient-mismatch"), "{body}");

    // Each mismatch is a security event, recorded once per request.
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-trc-patient-mismatch", 4).await;
}

#[tokio::test]
async fn a_contact_point_request_without_its_trc_patient_is_refused_before_the_index() {
    let (db, svc, app) = serve(eprescription(24)).await;
    let mine = patient_ehr(&svc, "SYNTHETIC-1").await;
    let credential = token(None, PROVIDER, &json!({}));
    let (status, body) = send(&app, "GET", &format!("/ehr/{mine}"), &credential, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("ERROR_SEC_DATA_INTEGRITY_NOT_ENSURED"),
        "{body}"
    );
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-trc-missing", 1).await;
    let resolutions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit.audit_event WHERE domain = 'linkage'")
            .fetch_one(&db.pool())
            .await
            .expect("query");
    assert_eq!(resolutions, 0, "the EHR index was never asked");
}

#[tokio::test]
async fn a_contact_point_discards_a_dispensation_of_its_point_of_care_within_the_window() {
    let (_db, svc, app) = serve(eprescription(24)).await;
    let mine = patient_ehr(&svc, "SYNTHETIC-1").await;
    let credential = token(Some("SYNTHETIC-1"), PROVIDER, &json!({}));
    let (status, version) = dispense(&app, &mine, &credential, &point_of_care(PROVIDER)).await;
    assert_eq!(status, StatusCode::CREATED, "the dispensation commits");

    // Another point of care may not discard it.
    let elsewhere = token(Some("SYNTHETIC-1"), "urn:oid:1.2.3.5", &json!({}));
    let path = format!("/ehr/{mine}/composition/{version}");
    let (status, body) = send(&app, "DELETE", &path, &elsewhere, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("another point of care"), "{body}");

    // A token whose permissions lack PPD-046 may not discard it either.
    let unpermitted = token(
        Some("SYNTHETIC-1"),
        PROVIDER,
        &json!({ "permissions": ["PRD-010"] }),
    );
    let (status, body) = send(&app, "DELETE", &path, &unpermitted, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("PPD-046"), "{body}");

    // The dispensing point of care discards it, as a logical delete.
    let permitted = token(
        Some("SYNTHETIC-1"),
        PROVIDER,
        &json!({ "permissions": ["urn:oasis:names:tc:xspa:1.0:subject:hl7:permission:PPD-046"] }),
    );
    let (status, body) = send(&app, "DELETE", &path, &permitted, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

#[tokio::test]
async fn a_dispensation_without_ppd_046_and_a_discard_after_the_window_are_refused() {
    let (_db, svc, app) = serve(eprescription(0)).await;
    let mine = patient_ehr(&svc, "SYNTHETIC-1").await;
    let unpermitted = token(
        Some("SYNTHETIC-1"),
        PROVIDER,
        &json!({ "permissions": ["PRD-010"] }),
    );
    let (status, body) = dispense(&app, &mine, &unpermitted, &point_of_care(PROVIDER)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("PPD-046"), "{body}");

    let credential = token(Some("SYNTHETIC-1"), PROVIDER, &json!({}));
    let (status, version) = dispense(&app, &mine, &credential, &point_of_care(PROVIDER)).await;
    assert_eq!(status, StatusCode::CREATED, "the dispensation commits");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let path = format!("/ehr/{mine}/composition/{version}");
    let (status, body) = send(&app, "DELETE", &path, &credential, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("discard window has passed"), "{body}");
}

/// The external committer of a contact point's caller, as the stored
/// `AUDIT_DETAILS` carries it (#5144).
fn relayed_committer() -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": "Synthetic Pharmacist", "identifiers": [
        { "_type": "DV_IDENTIFIER", "issuer": IDA_ISSUER, "id": NAME_ID,
          "type": format!("hp_identifier:{NAME_ID_FORMAT}") },
        { "_type": "DV_IDENTIFIER", "issuer": CONTACT_POINT, "id": "hp-nameid",
          "type": "oauth2" } ] })
}

/// A contact point's dispensation records the HP Identity Assertion's
/// `NameID` under the assertion's issuer, typed by its format, beside the
/// voucher's login, and its discard is a logical delete under the same
/// committer; a dispensation without the `organization-id` is refused with
/// `422` (#5144).
#[tokio::test]
async fn a_contact_points_dispensation_and_discard_record_the_ida_nameid() {
    let (_db, svc, app) = serve(eprescription(24)).await;
    let mine = patient_ehr(&svc, "SYNTHETIC-1").await;
    let credential = token(Some("SYNTHETIC-1"), PROVIDER, &json!({}));
    let (status, body) = dispense(&app, &mine, &credential, &[(AUTHORITY, PROVIDER)]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains("organization-id"), "{body}");

    let (status, version) = dispense(&app, &mine, &credential, &point_of_care(PROVIDER)).await;
    assert_eq!(status, StatusCode::CREATED, "the dispensation commits");
    let vo = version.split("::").next().expect("the versioned object id");
    let (status, body) = send(
        &app,
        "DELETE",
        &format!("/ehr/{mine}/composition/{version}"),
        &credential,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = send(
        &app,
        "GET",
        &format!("/ehr/{mine}/versioned_composition/{vo}/revision_history"),
        &credential,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let history: Value = serde_json::from_str(&body).expect("a REVISION_HISTORY");
    let audits: Vec<&Value> = history["items"]
        .as_array()
        .expect("the revision history items")
        .iter()
        .flat_map(|item| item["audits"].as_array().into_iter().flatten())
        .collect();
    assert_eq!(audits.len(), 2, "the dispensation and its discard: {body}");
    for audit in audits {
        assert_eq!(audit["committer"], relayed_committer(), "{body}");
    }
    assert_eq!(
        history["items"][1]["audits"][0]["change_type"]["defining_code"]["code_string"],
        json!("523"),
        "the discard is a logical delete: {body}"
    );
}
