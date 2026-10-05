// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the MUST-level ITS-REST protocol tail (B6 cluster
//! 4): the `openEHR-VERSION.*` / `openEHR-AUDIT_DETAILS.*` committal request
//! headers (parse + merge), `If-Match` hardening (malformed → 400), the
//! `OPTIONS /` System-Options-and-Conformance endpoint, and canonical-XML
//! responses for the VERSION family. Driven through the assembled
//! router over a **real** `FerroEhrService` on a real `PostgreSQL`.
//!
//! The committal-header merge is now verified end-to-end: the update is
//! committed and the persisted `ORIGINAL_VERSION` is read back to confirm the
//! header-supplied lifecycle/audit values were merged (replacing the former
//! `Mock` hook that captured the `UpdateVersion` in-process). The signed-version
//! XML asserts the real server-side digest signature (the default `FerroEhrService`
//! signer is enabled), not the Mock's injected fixture.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_assert_message,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A syntactically valid EHR/VO id for the malformed-If-Match probes (the
/// precondition is rejected before the backend, so the ids need not exist).
const EHR_ID: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";
const VO_ID: &str = "8849182c-82ad-4088-a07f-48ead4180515";
/// The client-supplied EHR id the `PUT /ehr/{ehr_id}` committal test creates.
const CREATED_EHR_ID: &str = "1f4d1a3e-24bb-4a1f-9e6f-2f0dcb0a5c11";

fn opt_xml() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt"),
    )
    .expect("ips.v0.opt vendored in openehr-its")
}

fn canonical_composition() -> Value {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
        ),
    )
    .expect("ips_canonical.json vendored in openehr-its");
    serde_json::from_str(&text).expect("valid canonical composition")
}

fn config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_owned(),
            base_path: BASE.to_owned(),
            max_in_flight: 1024,
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            basic: None,
            oidc: None,
            ..AuthConfig::default()
        },
        ..Default::default()
    }
}

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn etag_uid(h: &header::HeaderMap) -> String {
    h.get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

fn vo_of(ovid: &str) -> &str {
    ovid.split("::").next().expect("vo uuid")
}

/// [`commit_ips_composition`] with a caller-supplied composition body.
async fn commit_composition_body(app: &Router, body: Value) -> (String, String) {
    let (status, h, _b) = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let ehr_id = etag_uid(&h);

    let (status, _h, resp) = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(opt_xml()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {resp}");

    let (status, h, resp) = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {resp}");
    (ehr_id, etag_uid(&h))
}

/// Create an EHR, upload the IPS OPT, and commit the IPS composition; return the
/// `(ehr_id, version_uid)` of the committed COMPOSITION.
async fn commit_ips_composition(app: &Router) -> (String, String) {
    let (status, h, _b) = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let ehr_id = etag_uid(&h);

    let (status, _h, body) = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(opt_xml()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");

    let (status, h, body) = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(canonical_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
    (ehr_id, etag_uid(&h))
}

// ── OPTIONS on the API base path (the System API's one location) ──────────

#[tokio::test]
async fn options_root_is_system_options_and_conformance() {
    let (_pg, app) = app().await;
    // The System API mounts at the API base-path root ONLY
    // (`system.openapi.yaml` servers `{baseUrl}/v1`, path `/`); the former
    // bare-`/` alias was our own duplication and is gone.
    let req = Request::builder()
        .method("OPTIONS")
        .uri(BASE)
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK);
    // The `Allow` header lists the supported methods.
    assert_eq!(
        h.get(header::ALLOW).and_then(|v| v.to_str().ok()),
        Some("GET, POST, PUT, DELETE, OPTIONS")
    );
    // The `Options` conformance manifest body.
    let v: Value = serde_json::from_str(&body).expect("options body");
    // The served identity is the released ITS-REST contract version — the
    // `openehr-its` crate version, via the single provenance constant (a
    // plain version string, matching the System API OAS example).
    assert_eq!(
        v["restapi_specs_version"],
        ferroehr::telemetry::provenance::ITS_REST
    );
    assert_eq!(v["conformance_profile"], "STANDARD");
    assert!(v["endpoints"].as_array().is_some_and(|e| !e.is_empty()));
}

// ── committal headers (openEHR-VERSION.* / openEHR-AUDIT_DETAILS.*) ──────────

#[tokio::test]
async fn committal_headers_merge_into_the_commit() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1).to_owned();

    // Update the composition, supplying the committal metadata via the MUST-level
    // request headers; re-post the canonical body (strip the server-owned uid).
    let mut body = canonical_composition();
    body.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{v1}\""))
        // 800|inactive| — the `deactivate` transition of the RM common master06
        // §Abandoned and Inactive States table, which is exactly what this PUT
        // performs from the `532|complete|` first version. It is a state the
        // server would never default to, so the assertion below proves the
        // merge really happened. `523|deleted|` is NOT usable here: master06
        // §Logical Deletion makes the deleted state and the data-Void one act,
        // so a data-carrying PUT claiming it is refused — that refusal has its
        // own asserted test below.
        .header("openEHR-VERSION.lifecycle_state", "code_string=\"800\"")
        .header("openEHR-AUDIT_DETAILS.change_type", "code_string=\"251\"")
        .header(
            "openEHR-AUDIT_DETAILS.description",
            "value=\"An updated composition\"",
        )
        .header(
            "openEHR-AUDIT_DETAILS.committer",
            "name=\"John Doe\", external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\", \
             external_ref.namespace=\"demographic\", external_ref.type=\"PERSON\"",
        )
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, h, resp_body) = send(&app, req).await;
    assert!(
        status.is_success(),
        "update succeeded, got {status}: {resp_body}"
    );
    let v2 = etag_uid(&h);

    // Read the persisted ORIGINAL_VERSION back and confirm the header-supplied
    // committal metadata was merged into the commit.
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{v2}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    assert_eq!(ver["_type"], "ORIGINAL_VERSION");
    // Spec MUST (ITS-REST overview §"openehr-version and openehr-audit-details"):
    // "whatever is provided [in the committal headers] MUST be merged with the
    // default VERSION and VERSION.audit_details attributes on commit runtime."
    // The former `Mock` hook only captured the dispatcher-built `UpdateVersion`
    // (which is correct — see committal.rs unit tests); with the real service the
    // *persisted* ORIGINAL_VERSION must reflect the merged values. These
    // assertions verify the end-to-end MUST.
    assert_eq!(
        ver["lifecycle_state"]["defining_code"]["code_string"], "800",
        "openEHR-VERSION.lifecycle_state merged: {ver}"
    );
    let audit = &ver["commit_audit"];
    assert_eq!(
        audit["change_type"]["defining_code"]["code_string"], "251",
        "openEHR-AUDIT_DETAILS.change_type merged"
    );
    assert_eq!(
        audit["description"]["value"], "An updated composition",
        "openEHR-AUDIT_DETAILS.description merged"
    );
    assert_eq!(audit["committer"]["name"], "John Doe");
    assert_eq!(audit["committer"]["external_ref"]["type"], "PERSON");
}

/// The REFUSAL twin of the merge test above: a content-carrying `PUT` whose
/// `openehr-version` header claims `523|deleted|` is rejected `422`, and the
/// composition stays at its previous version.
///
/// RM common `master06-change_control_package.adoc` §Logical Deletion states
/// deletion as ONE procedure — "create a new Version in the normal way; delete
/// its `_data_` …; set the `_lifecycle_state_` value to the code for
/// `deleted`; commit in the normal way" — so a version that carries data
/// cannot be the deleted one. Merging the header value here would leave the
/// resource reading as deleted (`204`) while its content stayed stored.
#[tokio::test]
async fn a_deleted_lifecycle_header_on_a_content_commit_is_refused() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1).to_owned();

    let mut body = canonical_composition();
    body.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{v1}\""))
        .header("openehr-version", "lifecycle_state.code_string=\"523\"")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, _h, resp_body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a data-carrying commit may not claim the deleted state: {resp_body}"
    );
    assert!(
        resp_body.contains("Logical Deletion"),
        "the refusal names the spec rule it enforces, got {resp_body}"
    );

    // Nothing was committed: the composition is still readable at v1.
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{v1}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the composition is still live: {body}"
    );
}

/// A DELETE whose `openehr-version` header names a lifecycle other than
/// `523|deleted|` is refused `400` rather than having the value silently
/// discarded — the merge duty (ITS-REST overview §"openehr-version and
/// openehr-audit-details": "whatever is provided it MUST be merged") cannot be
/// honoured for a state the operation itself fixes (RM common master06
/// §Logical Deletion). The accepting twin states the `523` the DELETE commits.
#[tokio::test]
async fn a_contradictory_lifecycle_header_on_delete_is_refused() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;

    let refused = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{v1}"))
        .header("openehr-version", "lifecycle_state.code_string=\"532\"")
        .body(Body::empty())
        .unwrap();
    let (status, _h, resp_body) = send(&app, refused).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a DELETE may not claim a live lifecycle: {resp_body}"
    );

    // The accepting twin: the same DELETE stating the state it does commit.
    let accepted = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{v1}"))
        .header("openehr-version", "lifecycle_state.code_string=\"523\"")
        .body(Body::empty())
        .unwrap();
    let (status, _h, resp_body) = send(&app, accepted).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the delete stating 523 succeeds: {resp_body}"
    );
}

/// A legal DIVERGENT client `change_type` is honoured, not overwritten:
/// `250|amendment|` on an update commits an amendment (ITS-REST overview
/// §"openehr-version and openehr-audit-details" lists `change_type` first
/// among the client-suppliable attributes and requires "whatever is provided
/// it MUST be merged"; both 250 and 251 are legal update codes per the
/// `audit_change_type` group, RM common master06 §Contributions).
#[tokio::test]
async fn client_change_type_amendment_is_merged() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1).to_owned();

    let mut body = canonical_composition();
    body.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{v1}\""))
        // The Release-1.1.0 header name (attribute path in the value).
        .header("openehr-audit-details", "change_type.code_string=\"250\"")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, h, resp_body) = send(&app, req).await;
    assert!(status.is_success(), "update: {status}: {resp_body}");
    let v2 = etag_uid(&h);

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{v2}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    assert_eq!(
        ver["commit_audit"]["change_type"]["defining_code"]["code_string"], "250",
        "client-supplied amendment change_type merged: {ver}"
    );
}

/// A group code that contradicts the operation is a 400 change-control
/// mismatch (`249|creation|` on an update — mirroring the CONTRIBUTION
/// path's rule), and an out-of-group token is a 422
/// (`AUDIT_DETAILS.Change_type_valid`).
#[tokio::test]
async fn client_change_type_mismatch_and_out_of_group_are_rejected() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1).to_owned();
    let mut body = canonical_composition();
    body.as_object_mut().unwrap().remove("uid");

    for (token, expected) in [
        ("249", StatusCode::BAD_REQUEST),
        ("999", StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let req = Request::builder()
            .method("PUT")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, format!("\"{v1}\""))
            .header(
                "openehr-audit-details",
                format!("change_type.code_string=\"{token}\""),
            )
            .body(Body::from(body.to_string()))
            .unwrap();
        let (status, _h, resp_body) = send(&app, req).await;
        assert_eq!(
            status, expected,
            "change_type {token} on an update: {resp_body}"
        );
    }
}

/// A DELETE is a commit on a change-controlled resource, so the committal
/// headers are accepted and merged there too (overview §"openehr-version and
/// openehr-audit-details": services MUST allow PUT, POST and DELETE directly
/// and MUST accept both headers) — verified against the persisted
/// `523|deleted|` `ORIGINAL_VERSION`.
#[tokio::test]
async fn delete_accepts_and_merges_committal_headers() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1).to_owned();

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{v1}"))
        .header(
            "openehr-audit-details",
            "description.value=\"retracted per patient request\",committer.name=\"Dr Chart\"",
        )
        .body(Body::empty())
        .unwrap();
    let (status, h, resp_body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete: {resp_body}");
    let v2 = etag_uid(&h);

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{v2}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "deleted-version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    let audit = &ver["commit_audit"];
    assert_eq!(
        audit["change_type"]["defining_code"]["code_string"], "523",
        "a delete commits 523|deleted|: {ver}"
    );
    assert_eq!(
        audit["description"]["value"], "retracted per patient request",
        "header description merged into the delete audit"
    );
    assert_eq!(audit["committer"]["name"], "Dr Chart");
}

/// EHR creation is a commit on change-controlled content, so `POST /ehr`
/// accepts and merges the committal headers too. The overview MUST
/// (§"openehr-version and openehr-audit-details") covers "all
/// change-controlled resources (e.g. COMPOSITION, `EHR_STATUS`, FOLDER, etc.)"
/// on `PUT`, `POST` **and** `DELETE`, and EHR creation commits "a root EHR
/// object, an EHR Status object, and an EHR Access object … in a Contribution"
/// (RM ehr master04 §EHR Creation) — verified against the persisted
/// `EHR_STATUS` `ORIGINAL_VERSION`.
#[tokio::test]
async fn ehr_create_accepts_and_merges_committal_headers() {
    let (_pg, app) = app().await;

    // `Prefer: return=representation` yields the RM EHR body, whose
    // `ehr_status` OBJECT_REF carries the committed version's identity.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header("Prefer", "return=representation")
        .header("openehr-version", "lifecycle_state.code_string=\"553\"")
        .header(
            "openehr-audit-details",
            "description.value=\"EHR opened at triage\",committer.name=\"Dr Chart\",\
             committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
             committer.external_ref.namespace=\"demographic\",\
             committer.external_ref.type=\"PERSON\"",
        )
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "ehr create: {body}");
    let ehr_id = etag_uid(&h);
    let ehr: Value = serde_json::from_str(&body).expect("ehr json");
    // The EHR body's ehr_status ref names the CONTAINER (HIER_OBJECT_ID —
    // RM ehr Ehr_status_valid + BASE master05 OBJECT_REF.id); the VERSION
    // uid comes from the served EHR_STATUS itself.
    assert_eq!(ehr["ehr_status"]["id"]["_type"], "HIER_OBJECT_ID");
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "status read: {body}");
    let st: Value = serde_json::from_str(&body).expect("ehr_status json");
    let status_uid = st["uid"]["value"]
        .as_str()
        .expect("ehr_status version id")
        .to_owned();

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_ehr_status/version/{status_uid}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "status version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    assert_eq!(ver["_type"], "ORIGINAL_VERSION");
    // "whatever is provided it MUST be merged with the default VERSION and
    // VERSION.audit_details attributes on commit runtime."
    assert_eq!(
        ver["lifecycle_state"]["defining_code"]["code_string"], "553",
        "openehr-version lifecycle_state merged into the creation commit: {ver}"
    );
    let audit = &ver["commit_audit"];
    assert_eq!(
        audit["description"]["value"], "EHR opened at triage",
        "openehr-audit-details description merged: {ver}"
    );
    assert_eq!(audit["committer"]["name"], "Dr Chart");
    assert_eq!(audit["committer"]["external_ref"]["type"], "PERSON");
    // A create commits a FIRST version, so its change type stays
    // `249|creation|` (RM common master06 §Contributions) — the merge of the
    // other attributes must not disturb it.
    assert_eq!(
        audit["change_type"]["defining_code"]["code_string"], "249",
        "creation change type intact: {ver}"
    );
}

/// The same MUST on the id-supplied create (`PUT /ehr/{ehr_id}`) — the
/// overview names `PUT` first among the direct-commit methods the headers are
/// accepted on.
#[tokio::test]
async fn ehr_create_with_id_accepts_and_merges_committal_headers() {
    let (_pg, app) = app().await;

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{CREATED_EHR_ID}"))
        .header("Prefer", "return=representation")
        .header(
            "openehr-audit-details",
            "description.value=\"EHR pre-registered\",committer.name=\"Registrar\"",
        )
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "ehr create with id: {body}");
    let ehr: Value = serde_json::from_str(&body).expect("ehr json");
    // The EHR body's ehr_status ref names the CONTAINER (HIER_OBJECT_ID —
    // RM ehr Ehr_status_valid + BASE master05 OBJECT_REF.id); the VERSION
    // uid comes from the served EHR_STATUS itself.
    assert_eq!(ehr["ehr_status"]["id"]["_type"], "HIER_OBJECT_ID");
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{CREATED_EHR_ID}/ehr_status"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "status read: {body}");
    let st: Value = serde_json::from_str(&body).expect("ehr_status json");
    let status_uid = st["uid"]["value"]
        .as_str()
        .expect("ehr_status version id")
        .to_owned();

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{CREATED_EHR_ID}/versioned_ehr_status/version/{status_uid}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "status version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    let audit = &ver["commit_audit"];
    assert_eq!(audit["description"]["value"], "EHR pre-registered");
    assert_eq!(audit["committer"]["name"], "Registrar");
    assert_eq!(
        audit["change_type"]["defining_code"]["code_string"], "249",
        "creation change type intact: {ver}"
    );
}

/// A create commits a FIRST version, so the only `audit_change_type` group
/// code compatible with it is `249|creation|` (RM common master06
/// §Contributions; the same operation-compatibility rule the CONTRIBUTION
/// path applies). A legal-but-divergent code is a 400 change-control
/// mismatch, an out-of-group token a 422
/// (`AUDIT_DETAILS.Change_type_valid`), and a restated `249` passes.
#[tokio::test]
async fn ehr_create_rejects_a_change_type_that_is_not_a_creation() {
    let (_pg, app) = app().await;

    for (token, expected) in [
        ("250", StatusCode::BAD_REQUEST),
        ("523", StatusCode::BAD_REQUEST),
        ("999", StatusCode::UNPROCESSABLE_ENTITY),
        ("249", StatusCode::CREATED),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .header(
                "openehr-audit-details",
                format!("change_type.code_string=\"{token}\""),
            )
            .body(Body::empty())
            .unwrap();
        let (status, _h, body) = send(&app, req).await;
        assert_eq!(
            status, expected,
            "change_type {token} on an EHR create: {body}"
        );
    }
}

/// The BARE deprecated header name from the §"Deprecated headers" table
/// (`openEHR-AUDIT_DETAILS`) "remain[s] available for backward
/// compatibility" — accepted with the same attribute-path-in-value grammar.
#[tokio::test]
async fn bare_deprecated_audit_details_header_is_accepted() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1).to_owned();

    let mut body = canonical_composition();
    body.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{v1}\""))
        .header(
            "openEHR-AUDIT_DETAILS",
            "description.value=\"from a 1.0.x client\"",
        )
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, h, resp_body) = send(&app, req).await;
    assert!(status.is_success(), "update: {status}: {resp_body}");
    let v2 = etag_uid(&h);

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{v2}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    assert_eq!(
        ver["commit_audit"]["description"]["value"], "from a 1.0.x client",
        "bare deprecated header merged: {ver}"
    );
}

// ── If-Match hardening ─────────────────────────────────────

#[tokio::test]
async fn malformed_if_match_is_rejected_not_bypassed() {
    // A required If-Match that is not a well-formed OBJECT_VERSION_ID must be a
    // client error (400), never a silent skip of the precondition — rejected
    // before the backend, so the (non-existent) target ids are irrelevant.
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR_ID}/composition/{VO_ID}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, "\"not-an-object-version-id\"")
        .body(Body::from(r#"{"_type":"COMPOSITION"}"#))
        .unwrap();
    let (status, _h, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn malformed_if_match_on_ehr_status_update_is_rejected() {
    // The required-If-Match ehr_status update rejects a malformed precondition
    // (400) before the backend, never treating it as no-precondition.
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR_ID}/ehr_status"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, "\"garbage\"")
        .body(Body::from(r#"{"_type":"EHR_STATUS"}"#))
        .unwrap();
    let (status, _h, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn malformed_if_match_on_directory_update_is_rejected() {
    // The required-If-Match directory update rejects a malformed precondition
    // (400) at the wire, never a silent bypass.
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR_ID}/directory"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, "\"a::b::c::3\"")
        .body(Body::from(r#"{"_type":"FOLDER","name":{"value":"root"}}"#))
        .unwrap();
    let (status, _h, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ── VERSION-family canonical XML (ECC-COM-022, ECC-SIG-001) ──────────────────

#[tokio::test]
async fn versioned_composition_serves_xml() {
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/versioned_composition/{vo}"))
        .header(header::ACCEPT, "application/xml")
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        Some("application/xml")
    );
    assert!(body.contains("<versioned_composition"), "root: {body}");
    assert!(body.contains(vo), "uid present: {body}");
}

#[tokio::test]
async fn composition_version_serves_xml_with_signature() {
    // ECC-SIG-001: the ORIGINAL_VERSION XML carries the `<signature>` element —
    // the default `FerroEhrService` signer (SHA-256 digest) commits a genuine
    // `sha256:` signature which the canonical XML serializes there.
    //
    // The DOCUMENT is pinned too. ITS-REST `Resources.md` §"XML Format" MUSTs
    // conformance to the published XSDs, whose VERSION document element is
    // abstract (`Version.xsd`), so the conforming response names a derived type
    // with `xsi:type="ORIGINAL_VERSION"` (XML Schema Part 1 §2.6.1 + §3.4.6).
    let (_pg, app) = app().await;
    let (ehr_id, v1) = commit_ips_composition(&app).await;
    let vo = vo_of(&v1);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{v1}"
        ))
        .header(header::ACCEPT, "application/xml")
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        Some("application/xml")
    );
    assert!(
        body.starts_with("<version "),
        "the published document element is the root: {body}"
    );
    assert!(
        body.contains(r#"xsi:type="ORIGINAL_VERSION""#),
        "an instance of the abstract VERSION type names its concrete class: {body}"
    );
    assert!(
        !body.contains("<original_version"),
        "no undeclared per-subtype root: {body}"
    );
    assert!(body.contains("<signature"), "signature element: {body}");
    assert!(body.contains("sha256:"), "digest signature value: {body}");
}

/// The JSON-accept composition GET serves the stored canonical body BYTES
/// verbatim (#2913): `_type` first, every field at its BMM-declared position —
/// including the server-stamped `uid`, which the RM BMM declares third on
/// LOCATABLE (after `name` and `archetype_node_id`), exercised here by a
/// commit whose request body carries NO `uid`. The served text is
/// byte-identical to the canonical codec's own re-encoding, and the
/// version-addressed variant passes through the same bytes. An XML accept on
/// the same resource still parses and re-serializes (the passthrough is
/// representation-local).
#[tokio::test]
async fn composition_get_serves_stored_json_verbatim() {
    let (_pg, app) = app().await;
    // The committed body must be uid-less so the stamped-field PLACEMENT is
    // what the byte comparison exercises (the borutjures case on #2913).
    let mut fixture = canonical_composition();
    fixture
        .as_object_mut()
        .expect("a JSON object")
        .remove("uid");
    let (ehr_id, v1) = commit_composition_body(&app, fixture).await;
    let vo = vo_of(&v1);

    let get = |uri: String, accept: &'static str| {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(header::ACCEPT, accept)
            .body(Body::empty())
            .unwrap()
    };

    // The latest read.
    let (status, h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr_id}/composition/{vo}"),
            "application/json",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert!(
        body.starts_with("{\"_type\":\"COMPOSITION\""),
        "the canonical encoding opens with _type: {body}"
    );
    let parsed: Value = serde_json::from_str(&body).expect("the passthrough text is JSON");
    let keys: Vec<&str> = parsed
        .as_object()
        .expect("a JSON object")
        .keys()
        .take(4)
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["_type", "name", "archetype_node_id", "uid"],
        "the stamped uid sits at its BMM-declared position"
    );
    assert_eq!(parsed["uid"]["value"], Value::String(v1.clone()));

    // The served bytes ARE the codec's encoding: parsing them into the typed
    // RM value and re-encoding reproduces the identical text.
    let typed: openehr_rm::v1_2::composition::composition::Composition =
        openehr_its::json::from_canonical_value(&parsed).expect("served body decodes as typed RM");
    let reencoded = openehr_its::json::to_canonical_json(&typed);
    assert_eq!(
        body, reencoded,
        "the served bytes are byte-identical to the canonical codec's encoding"
    );

    // The version-addressed read takes the same passthrough.
    let (status, _h, at_version) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr_id}/composition/{v1}"),
            "application/json",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {at_version}");
    assert_eq!(at_version, body, "both reads serve the same stored bytes");

    // XML negotiation still parses and re-serializes the same resource.
    let (status, h, xml) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr_id}/composition/{vo}"),
            "application/xml",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {xml}");
    assert_eq!(
        h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        Some("application/xml")
    );
    assert!(xml.contains(&v1), "the XML body carries the uid: {xml}");
}

/// The versioned-composition by-id read is container-scoped: a well-formed,
/// EXISTING `version_uid` whose `object_id` names a DIFFERENT container than
/// the path's `{versioned_object_uid}` names no version of that resource -> `404` (ITS-REST overview `Resources.md` §Identifier types: "the `object_id`
/// matches the `VERSIONED_OBJECT` identifier"; RM common `version.adoc`
/// invariant `Owner_id_valid`). The coherent pair still serves 200.
#[tokio::test]
async fn versioned_composition_version_by_id_is_container_scoped() {
    let (_pg, app) = app().await;
    // Two independent compositions in the same EHR: the second create gives a
    // second container in the same EHR (the IPS OPT is event-category, so a
    // second create is allowed).
    let (ehr_id, v_a) = commit_ips_composition(&app).await;
    let (status, h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(canonical_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "second commit: {body}");
    let v_b = etag_uid(&h);
    let container_a = vo_of(&v_a).to_owned();
    let container_b = vo_of(&v_b).to_owned();
    assert_ne!(container_a, container_b, "two distinct version containers");

    // Incoherent pair: container A's URL, container B's version → 404.
    let (status, _h, _b) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_composition/{container_a}/version/{v_b}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a foreign container's version is not a version of this resource"
    );

    // Coherent pair unchanged.
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/versioned_composition/{container_a}/version/{v_a}"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "coherent read: {body}");
}
