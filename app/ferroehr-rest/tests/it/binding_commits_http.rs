// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The committer of an attestation and the protection of a professional's
//! `PERSON` over HTTP (#5011), driven through the assembled router over a real
//! `FerroEhrService` on a real `PostgreSQL` 18 with real tokens.
//!
//! RM common master04 §Attestation makes an attestation an explicit signing by
//! a healthcare agent, so only a caller bound to a professional `PERSON`
//! attests; RM common `org.openehr.rm.common.audit_details.adoc` lets the
//! server write who committed. Who may version a `PERSON` the login binding
//! reads is governed by no openEHR spec: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::clients::{Facility, Origin, SystemClient};
use ferroehr::party_binding::source::BindingSource;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::kind::IssuerKind;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{
    AUDIENCE, ISSUER, LOGIN, config, mapping, person, professional, router_with, token, vo,
};

/// The deployment's administrator role, which `role_issuers` lets `ISSUER`
/// grant.
const ADMIN: &str = "ADMIN";
/// The `machines` issuer of the backend client `lab`.
const MACHINES: &str = "https://machines.example";
/// The audience of [`MACHINES`].
const MACHINE_AUDIENCE: &str = "ferroehr-machines";
/// The `patients` issuer where the record's persons sign in.
const PATIENTS: &str = "https://patients.example";
/// The issuer of the facility identifiers the client `lab` commits for.
const FACILITIES: &str = "https://facilities.example";
/// The template the client `lab` writes.
const IPS: &str = "International Patient Summary";
/// The SMART scopes of a backend client that writes EHRs.
const SYSTEM_SCOPES: &str = "system/composition-*.cruds system/template-*.r";
/// The binding key of the professional whose `PERSON` is protected.
const CLINICIAN: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
/// The binding key of another professional, holding no administrator role.
const COLLEAGUE: &str = "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e";
/// The binding key of the administrator's own `PERSON`.
const ADMINISTRATOR: &str = "3c4d5e6f-7a8b-4c9d-8e0f-2a3b4c5d6e7f";

/// The binding configuration of `party_binding_http`, also trusting the
/// machines issuer with `lab` declared as a relay client and the patients
/// issuer, with `ISSUER`'s role claims granting the administrator role.
fn deployment() -> AppConfig {
    let mut config = config(None);
    let jwks_file = common::jwks_file().display().to_string();
    {
        let entries = &mut config.auth.shared.issuer;
        entries.push(IssuerEntry {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(MACHINES),
            kind: Some(IssuerKind::Machines),
            audience: vec![MACHINE_AUDIENCE.to_owned()],
            jwks_file: jwks_file.clone(),
            // NOTE: the test key signs RFC 9068 `at+jwt` tokens, which the
            // profile's overridable `token_types` admits here.
            token_types: Some(vec!["at+jwt".to_owned()]),
            ..IssuerEntry::default()
        });
        entries.push(IssuerEntry {
            kind: Some(IssuerKind::Patients),
            jwks_file,
            ..common::entry(PATIENTS, AUDIENCE)
        });
    }
    config.rbac = common::rbac_trusting(ISSUER);
    config.system_clients = vec![SystemClient {
        issuer: MACHINES.to_owned(),
        client_id: "lab".to_owned(),
        origins: vec![Origin::Relay],
        writes: vec![IPS.to_owned()],
        facilities: vec![Facility {
            issuer: FACILITIES.to_owned(),
            id: Some("ward-7".to_owned()),
            id_prefix: None,
        }],
        ..SystemClient::default()
    }];
    config.auth.smart.patient_issuers = vec![PATIENTS.to_owned()];
    config
}

/// The platform service over a fresh database, its binding mapping naming
/// [`ADMIN`] the administrator role, and the router over it.
async fn app() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_binding_mapping(mapping().with_admin_role(ADMIN)),
    );
    let source: Arc<dyn BindingSource> = Arc::new(svc.party_source("demographic"));
    let app = router_with(Arc::clone(&svc), deployment(), source).await;
    (db, svc, app)
}

/// The backend client `lab`'s app-only token.
fn system() -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({
        "iss": MACHINES, "aud": MACHINE_AUDIENCE, "exp": exp,
        "sub": "lab", "client_id": "lab", "scope": SYSTEM_SCOPES
    }))
}

/// The placeholder committer naming the login `subject` of `issuer`.
fn placeholder(issuer: &str, subject: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": subject }] })
}

/// The wire `DV_CODED_TEXT` of the `openehr` code `code` with rubric `value`.
fn coded(code: &str, value: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code } })
}

/// The IPS composition as the client `lab` relays it: its composer named by
/// an identifier and its facility one the client commits for.
fn relayed() -> Value {
    let mut composition = common::ips_canonical_composition();
    if let Some(object) = composition.as_object_mut() {
        object.remove("uid");
    }
    composition["composer"] = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr A",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "https://professionals.example",
                          "id": "hp-1" }] });
    composition["context"]["health_care_facility"] = json!({ "_type": "PARTY_IDENTIFIED",
        "name": "Ward 7",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": FACILITIES, "id": "ward-7" }] });
    composition
}

/// A CONTRIBUTION creating `composition`, every committer `committer`,
/// carrying an attestation by `committer` when `attested`.
fn contribution(composition: &Value, committer: &Value, attested: bool) -> Value {
    let audit = |code: &str, value: &str| json!({ "change_type": coded(code, value), "committer": committer });
    let mut version = json!({
        "_type": "ORIGINAL_VERSION",
        "commit_audit": audit("249", "creation"),
        "lifecycle_state": coded("532", "complete"),
        "data": composition
    });
    if attested {
        version["attestations"] = json!([{
            "_type": "UPDATE_ATTESTATION",
            "change_type": coded("666", "attestation"),
            "committer": committer,
            "reason": { "_type": "DV_TEXT", "value": "signed" },
            "is_pending": false
        }]);
    }
    json!({ "versions": [version], "audit": audit("251", "modification") })
}

/// A `method` request for `path` under the API base as `credential`, asking
/// for the representation.
pub(crate) fn request(method: &str, path: &str, credential: &str) -> common::Req {
    common::req(method, format!("{BASE}{path}"))
        .auth(credential)
        .accept("application/json")
        .prefer_representation()
}

/// A `method` request for `path` as `credential` carrying `body`.
pub(crate) fn with_body(method: &str, path: &str, credential: &str, body: &Value) -> common::Req {
    request(method, path, credential).json(body.to_string())
}

/// Sends `request` and asserts the answer is `expected`, naming `case`;
/// returns the answer's headers and body.
pub(crate) async fn answered(
    app: &Router,
    request: common::Req,
    expected: StatusCode,
    case: &str,
) -> (http::HeaderMap, String) {
    let (status, headers, body) = common::send(app, request.build()).await;
    assert_eq!(status, expected, "{case}: {body}");
    (headers, body)
}

/// Creates an EHR as `credential`, returning its id.
pub(crate) async fn ehr(app: &Router, credential: &str) -> String {
    let (headers, _) = answered(
        app,
        request("POST", "/ehr", credential),
        StatusCode::CREATED,
        "the EHR",
    )
    .await;
    common::etag_uid(&headers)
}

#[tokio::test]
async fn a_bound_professionals_attestation_stores_the_servers_committer() {
    let (_db, svc, app) = app().await;
    let (person_uid, _) = professional(&svc, CLINICIAN, "532").await;
    let credential = token(CLINICIAN, &json!({}));
    let ehr = ehr(&app, &credential).await;

    let (_, body) = answered(
        &app,
        with_body(
            "POST",
            &format!("/ehr/{ehr}/contribution"),
            &credential,
            &contribution(
                &crate::abac_e2e::composition(),
                &placeholder(ISSUER, CLINICIAN),
                true,
            ),
        ),
        StatusCode::CREATED,
        "the attested contribution",
    )
    .await;
    let created: Value = serde_json::from_str(&body).expect("a CONTRIBUTION");
    let uid = created["versions"][0]["id"]["value"]
        .as_str()
        .expect("the version uid")
        .to_owned();
    let vo_id = uid.split("::").next().expect("a container id");
    let (_, body) = answered(
        &app,
        request(
            "GET",
            &format!("/ehr/{ehr}/versioned_composition/{vo_id}/version/{uid}"),
            &credential,
        ),
        StatusCode::OK,
        "the attested version",
    )
    .await;
    let version: Value = serde_json::from_str(&body).expect("an ORIGINAL_VERSION");
    let attester = &version["attestations"][0]["committer"];
    assert_eq!(attester["_type"], "PARTY_IDENTIFIED", "{attester}");
    assert_eq!(
        attester["external_ref"]["id"]["value"],
        vo(&person_uid).0.to_string(),
        "the attestation names the bound PERSON: {attester}"
    );
    assert!(
        attester["identifiers"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id["id"] == CLINICIAN)),
        "the attestation carries the login: {attester}"
    );
    assert_eq!(
        attester, &version["commit_audit"]["committer"],
        "the attestation and the version are committed by the same server committer"
    );
}

#[tokio::test]
async fn an_attestation_from_a_system_caller_is_refused() {
    let (_db, svc, app) = app().await;
    professional(&svc, CLINICIAN, "532").await;
    answered(
        &app,
        request(
            "POST",
            "/definition/template/adl1.4",
            &token(CLINICIAN, &json!({})),
        )
        .header(http::header::CONTENT_TYPE, "application/xml")
        .body(common::ips_opt_xml()),
        StatusCode::CREATED,
        "the template",
    )
    .await;
    let credential = system();
    let ehr = ehr(&app, &credential).await;
    let path = format!("/ehr/{ehr}/contribution");
    let committer = placeholder(MACHINES, "lab");

    // The client commits the same contribution when it carries no attestation.
    answered(
        &app,
        with_body(
            "POST",
            &path,
            &credential,
            &contribution(&relayed(), &committer, false),
        ),
        StatusCode::CREATED,
        "the unattested contribution",
    )
    .await;
    let (_, body) = answered(
        &app,
        with_body(
            "POST",
            &path,
            &credential,
            &contribution(&relayed(), &committer, true),
        ),
        StatusCode::FORBIDDEN,
        "the attested contribution",
    )
    .await;
    assert!(body.contains("only a caller bound"), "{body}");
}

/// A `PERSON` of the professional template carrying no binding key.
fn unkeyed() -> Value {
    let mut body = person(LOGIN, CLINICIAN);
    if let Some(object) = body.as_object_mut() {
        object.remove("details");
    }
    body
}

/// `body` carrying `key` as a further binding key.
fn with_key(body: &Value, key: &str) -> Value {
    let mut keyed = body.clone();
    let details = person(LOGIN, key)["details"].clone();
    match keyed
        .pointer_mut("/details/items/0/items")
        .and_then(Value::as_array_mut)
    {
        Some(items) => items.push(details["items"][0]["items"][0].clone()),
        None => keyed["details"] = details,
    }
    keyed
}

/// The uid of the head version of the `PERSON` container `container`.
async fn head(app: &Router, container: &str, credential: &str) -> String {
    let (headers, _) = answered(
        app,
        request("GET", container, credential),
        StatusCode::OK,
        "the head",
    )
    .await;
    common::etag_uid(&headers)
}

#[tokio::test]
async fn only_an_administrator_versions_a_professionals_person() {
    let (_db, svc, app) = app().await;
    let (uid, body) = professional(&svc, CLINICIAN, "532").await;
    professional(&svc, COLLEAGUE, "532").await;
    professional(&svc, ADMINISTRATOR, "532").await;
    let colleague = token(COLLEAGUE, &json!({}));
    let administrator = token(ADMINISTRATOR, &json!({ "roles": [ADMIN] }));
    let container = format!("/demographic/person/{}", vo(&uid).0);
    let forbidden = StatusCode::FORBIDDEN;

    // A non-administrator neither versions, retires nor deletes it.
    let version = with_body("PUT", &container, &colleague, &body).if_match(format!("\"{uid}\""));
    answered(&app, version, forbidden, "a colleague's version").await;
    let retired = with_body("PUT", &container, &colleague, &body)
        .if_match(format!("\"{uid}\""))
        .header("openehr-version", "lifecycle_state.code_string=\"800\"");
    answered(&app, retired, forbidden, "a colleague's lifecycle change").await;
    let deleted = request("DELETE", &format!("/demographic/person/{uid}"), &colleague);
    answered(&app, deleted, forbidden, "a colleague's delete").await;
    assert_eq!(
        head(&app, &container, &colleague).await,
        uid,
        "no refused write landed"
    );

    // Nor plants a binding key: neither in a new PERSON nor in one it wrote.
    let planted = with_body(
        "POST",
        "/demographic/person",
        &colleague,
        &with_key(&unkeyed(), "4d5e6f7a-8b9c-4d0e-9f1a-3b4c5d6e7f8a"),
    );
    answered(&app, planted, forbidden, "a key-bearing PERSON").await;
    let (headers, _) = answered(
        &app,
        with_body("POST", "/demographic/person", &colleague, &unkeyed()),
        StatusCode::CREATED,
        "a PERSON carrying no key",
    )
    .await;
    let plain = common::etag_uid(&headers);
    let keyed = with_key(&unkeyed(), "5e6f7a8b-9c0d-4e1f-8a2b-4c5d6e7f8a9b");
    let added = with_body(
        "PUT",
        &format!("/demographic/person/{}", vo(&plain).0),
        &colleague,
        &keyed,
    )
    .if_match(format!("\"{plain}\""));
    answered(&app, added, forbidden, "a key added by a colleague").await;

    // The administrator neither adds a key nor drops the one it carries.
    let second = with_key(&body, "6f7a8b9c-0d1e-4f2a-9b3c-5d6e7f8a9b0c");
    let added =
        with_body("PUT", &container, &administrator, &second).if_match(format!("\"{uid}\""));
    answered(&app, added, forbidden, "a key added by the administrator").await;
    let mut dropped = body.clone();
    if let Some(object) = dropped.as_object_mut() {
        object.remove("details");
    }
    let omitted =
        with_body("PUT", &container, &administrator, &dropped).if_match(format!("\"{uid}\""));
    let (_, refusal) = answered(
        &app,
        omitted,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a dropped key",
    )
    .await;
    assert!(refusal.contains("protected"), "{refusal}");
    assert_eq!(
        head(&app, &container, &administrator).await,
        uid,
        "no refused write landed"
    );

    // And versions it.
    let version =
        with_body("PUT", &container, &administrator, &body).if_match(format!("\"{uid}\""));
    answered(&app, version, StatusCode::OK, "the administrator's version").await;
}

#[tokio::test]
async fn a_patients_demographic_write_is_refused() {
    let (_db, svc, app) = app().await;
    professional(&svc, COLLEAGUE, "532").await;
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let patient = common::bearer(&json!({
        "iss": PATIENTS, "aud": AUDIENCE, "exp": exp, "sub": "jane",
        "scope": "patient/composition-*.cruds __ferroehr.platform"
    }));
    let (_, body) = answered(
        &app,
        with_body("POST", "/demographic/person", &patient, &unkeyed()),
        StatusCode::FORBIDDEN,
        "a patient's PERSON",
    )
    .await;
    assert!(body.contains("commits no demographic version"), "{body}");
    // A professional writes the same PERSON.
    let colleague = token(COLLEAGUE, &json!({}));
    answered(
        &app,
        with_body("POST", "/demographic/person", &colleague, &unkeyed()),
        StatusCode::CREATED,
        "a professional's PERSON",
    )
    .await;
}
