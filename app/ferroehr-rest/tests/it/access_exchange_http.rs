// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The import and export of an EHR's access directives as FHIR `Consent`,
//! IHE BPPC and IHE APPC, end to end over the assembled router (#5517).
//!
//! An import commits a new `EHR_ACCESS` version over the one `If-Match`
//! names and refuses a document whole with the element named; an export
//! answers the directives in force in each format. The routes are our own
//! extension: no openEHR spec governs an exchange of access settings.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::common;
use crate::common::BASE;
use crate::common::EXT;

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An app with an administrator and a clinician, under RBAC.
async fn app() -> (testkit::TestDb, Router) {
    let (db, pool) = common::migrated_pool().await;
    let svc = Arc::new(common::binding_service(&pool));
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[("root", &["ADMIN"]), ("clinician", &["USER"])],
    )
    .await;
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(
        &authz,
        &config.api.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = common::build_full(config, svc, authz, Observability::default()).await;
    (db, app)
}

/// Sends `method uri` as `user` with the `If-Match` and body given.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    if_match: Option<&str>,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, basic(user))
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    if let Some(version) = if_match {
        req = req.header(header::IF_MATCH, format!("\"{version}\""));
    }
    let req = match body {
        Some((content_type, body)) => req
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .unwrap();
    common::send_body(app, req).await
}

/// The current `EHR_ACCESS` version of `ehr` and its directives.
async fn current(app: &Router, ehr: &str) -> (String, Vec<Value>) {
    let (status, body) = send(
        app,
        "GET",
        &format!("{EXT}/admin/ehr-access?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let read: Value = serde_json::from_str(&body).expect("versions");
    let newest = &read["versions"][0];
    (
        newest["version_uid"]
            .as_str()
            .expect("a version")
            .to_owned(),
        newest["settings"]["directives"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    )
}

/// An R5 `Consent` that permits everything except laboratory results.
fn consent() -> Value {
    json!({
        "resourceType": "Consent",
        "status": "active",
        "decision": "permit",
        "provision": [{ "code": [{ "coding": [{ "system": "http://loinc.org",
                                                  "code": "26436-6" }] }] }]
    })
}

#[tokio::test]
async fn directives_import_and_export_through_each_format() {
    let (_db, app) = app().await;
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let created: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = created["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let (first, _) = current(&app, &ehr).await;

    let consent_uri = format!("{EXT}/admin/ehr-access/fhir/r5/Consent?ehr_id={ehr}");
    let fhir = Some(("application/fhir+json", consent().to_string()));
    let (status, body) = send(&app, "POST", &consent_uri, "root", None, fhir.clone()).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "If-Match is required: {body}"
    );
    let (status, body) = send(
        &app,
        "POST",
        &consent_uri,
        "clinician",
        Some(&first),
        fhir.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an admin route: {body}");
    let (status, body) = send(&app, "POST", &consent_uri, "root", Some(&first), fhir).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).expect("json")["imported"],
        json!(1)
    );
    let (second, directives) = current(&app, &ehr).await;
    assert_eq!(directives.len(), 1);

    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/ehr-access/fhir/r4/Consent?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bundle: Value = serde_json::from_str(&body).expect("a bundle");
    assert_eq!(bundle["resourceType"], json!("Bundle"));
    assert_eq!(bundle["entry"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        bundle["entry"][0]["resource"]["policyRule"]["coding"][0]["code"],
        json!("OPTIN")
    );

    let appc_uri = format!("{EXT}/admin/ehr-access/appc?ehr_id={ehr}");
    let (status, xacml) = send(&app, "GET", &appc_uri, "root", None, None).await;
    assert_eq!(status, StatusCode::OK, "{xacml}");
    let (status, body) = send(
        &app,
        "POST",
        &appc_uri,
        "root",
        Some(&second),
        Some(("application/xml", xacml)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, directives) = current(&app, &ehr).await;
    assert_eq!(directives.len(), 2);
    assert_eq!(
        directives[0], directives[1],
        "the APPC round trip keeps the directive"
    );

    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/ehr-access/bppc?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bppc: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(bppc["documents"], json!([]));
    assert_eq!(
        bppc["refused"].as_array().map(Vec::len),
        Some(2),
        "no catalogue entry"
    );
}

#[tokio::test]
async fn a_refused_document_names_its_element_and_commits_nothing() {
    let (_db, app) = app().await;
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let created: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = created["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let (version, _) = current(&app, &ehr).await;
    let uri = format!("{EXT}/admin/ehr-access/fhir/r5/Consent?ehr_id={ehr}");
    let mut deep = consent();
    deep["provision"][0]["provision"] = json!([{ "code": [{ "coding": [{ "system":
        "http://loinc.org", "code": "1" }] }], "provision": [{ "code": [{ "coding": [{
        "system": "http://loinc.org", "code": "2" }] }] }] }]);
    let mut unknown = consent();
    unknown["provision"][0]["actor"] = json!([{ "reference": { "type": "Practitioner",
        "identifier": { "system": "urn:ietf:rfc:3986",
                        "value": "urn:uuid:0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f99" } } }]);
    for (document, element) in [
        (deep, "Consent.provision.provision.provision"),
        (unknown, "applies_to.party"),
    ] {
        let (status, body) = send(
            &app,
            "POST",
            &uri,
            "root",
            Some(&version),
            Some(("application/fhir+json", document.to_string())),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains(element), "names {element}: {body}");
    }
    let (status, body) = send(
        &app,
        "POST",
        &format!("{EXT}/admin/ehr-access/bppc?ehr_id={ehr}&evidence=ehr:/x"),
        "root",
        Some(&version),
        Some((
            "application/xml",
            "<ClinicalDocument xmlns='urn:hl7-org:v3'><templateId \
             root='1.3.6.1.4.1.19376.1.5.3.1.1.7'/><documentationOf><serviceEvent><templateId \
             root='1.3.6.1.4.1.19376.1.5.3.1.2.6'/><code code='1' codeSystem='2'/><effectiveTime>\
             <low value='20260101'/></effectiveTime></serviceEvent></documentationOf>\
             </ClinicalDocument>"
                .to_owned(),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("serviceEvent/code"), "{body}");
    let (after, directives) = current(&app, &ehr).await;
    assert_eq!(after, version, "nothing was committed");
    assert_eq!(directives, Vec::<Value>::new());
}
