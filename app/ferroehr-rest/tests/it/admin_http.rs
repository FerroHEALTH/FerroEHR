// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the ADMIN API group (physical EHR delete): the
//! config gate (`AppConfig::admin.enabled`) and the `204`/`404` wire outcomes —
//! driven through the assembled router over a **real** `AdminService` on a real
//! `PostgreSQL`. Per the vendored Admin OAS (`operations/admin_ehr_delete_all.yaml`)
//! the bulk delete declares only bodyless successes (`202`/`204`), and an absent
//! `ehr_id` list means "delete ALL EHRs" (`parameters/query/ehr_id_Admin.yaml`).
//!
//! Spec grounding: SM `I_ADMIN_SERVICE.physical_ehr_delete` and the CNF Robot
//! prior art (`CNF/tests/platform/robot/I_ADMIN_SERVICE/001-EHR.robot`:
//! `DELETE /admin/ehr/{ehr_id}` → `204`).
//!
//! The former `Mock` backend's call-counter/recorder assertions (proving the
//! backend was/was-not consulted) are re-targeted to real observation: the EHRs
//! are created through the wire and their subsequent existence (`GET /ehr/{id}`
//! → `200` vs `404`) proves whether the delete reached the backend and took
//! effect.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};

use crate::common;
use crate::common::BASE;

/// A syntactically valid EHR id that is never created — the "unknown" probe.
const OTHER: &str = "11111111-2222-3333-4444-555555555555";

async fn app(admin_enabled: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        common::router_with(common::api_config(admin_enabled), service),
    )
}

/// The `Allow` field value of a `405`, which RFC 9110 §15.5.6 makes mandatory:
/// "The origin server MUST generate an Allow header field in a 405 response
/// containing a list of the target resource's currently supported methods."
/// Fails loudly when the header is absent — an empty VALUE is legal, an absent
/// header is not.
fn allow_of(headers: &header::HeaderMap) -> &str {
    headers
        .get(header::ALLOW)
        .expect("a 405 MUST carry an Allow header (RFC 9110 §15.5.6)")
        .to_str()
        .expect("Allow is ASCII")
}

/// Whether an EHR still exists (`GET /ehr/{id}` → 200 vs 404).
async fn ehr_exists(app: &Router, id: &str) -> bool {
    match common::send_status(app, common::get(&format!("{BASE}/ehr/{id}"))).await {
        StatusCode::OK => true,
        StatusCode::NOT_FOUND => false,
        other => panic!("unexpected GET /ehr status {other}"),
    }
}

#[tokio::test]
async fn disabled_admin_is_405_and_never_deletes() {
    let (_pg, app) = app(false).await;
    // Create a real EHR, then attempt the (disabled) admin delete.
    let id = common::create_ehr(&app).await;
    let (status, headers, _) =
        common::send(&app, common::delete(&format!("{BASE}/admin/ehr/{id}"))).await;
    // The gate answers 405 Method Not Allowed — the status the OAS itself
    // declares for a disabled admin operation
    // (`admin_ehr_delete_all.yaml` + `responses/405.yaml`).
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    // This 405 comes from a MATCHED handler, so axum's allow-header machinery
    // never runs and the handler must supply `Allow` itself. The value is the
    // EMPTY field value RFC 9110 §10.2.1 defines for exactly this situation:
    // "An empty Allow field value indicates that the resource allows no
    // methods, which might occur in a 405 response if the resource has been
    // temporarily disabled by configuration."
    assert_eq!(allow_of(&headers), "");
    // RE-TARGET (was a call-counter assertion): the backend was never reached,
    // proven by the EHR still existing.
    assert!(ehr_exists(&app, &id).await, "the EHR must not be deleted");
}

#[tokio::test]
async fn enabled_delete_is_204() {
    let (_pg, app) = app(true).await;
    let id = common::create_ehr(&app).await;
    let (status, body) =
        common::send_body(&app, common::delete(&format!("{BASE}/admin/ehr/{id}"))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty(), "204 carries no body, got {body:?}");
    // RE-TARGET (was `calls == 1`): the delete took effect — the EHR is gone.
    assert!(!ehr_exists(&app, &id).await, "the EHR must be deleted");
}

#[tokio::test]
async fn enabled_delete_unknown_maps_to_404() {
    let (_pg, app) = app(true).await;
    // The backend's NotFound (ehr_id_does_not_exist) surfaces as HTTP 404.
    let (status, _) =
        common::send_body(&app, common::delete(&format!("{BASE}/admin/ehr/{OTHER}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enabled_delete_all_is_204_bodyless() {
    // `admin_ehr_delete_all.yaml:18-26`: the only declared success responses are
    // `202` (async) and `204 No Content` (sync) — both bodyless.
    let (_pg, app) = app(true).await;
    let a = common::create_ehr(&app).await;
    let b = common::create_ehr(&app).await;
    let (status, body) = common::send_body(
        &app,
        common::delete(&format!("{BASE}/admin/ehr/all?ehr_id={a},{b}")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty(), "204 carries no body, got {body:?}");
    // RE-TARGET (was `calls == 1`): both listed EHRs are gone.
    assert!(!ehr_exists(&app, &a).await, "a deleted");
    assert!(!ehr_exists(&app, &b).await, "b deleted");
}

#[tokio::test]
async fn enabled_delete_all_repeated_param_reaches_backend_with_both_ids() {
    // The repeated `?ehr_id=` form is surfaced from the raw query (the generated
    // Option<String> param would otherwise keep only the first). RE-TARGET (was
    // a recorder of the ids handed to the mock): both EHRs being deleted proves
    // the RFC 6570 `{?ehr_id*}` list handling passed both ids to the backend.
    let (_pg, app) = app(true).await;
    let a = common::create_ehr(&app).await;
    let b = common::create_ehr(&app).await;
    let (status, body) = common::send_body(
        &app,
        common::delete(&format!("{BASE}/admin/ehr/all?ehr_id={a}&ehr_id={b}")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty(), "204 carries no body, got {body:?}");
    assert!(!ehr_exists(&app, &a).await, "first repeated ehr_id deleted");
    assert!(
        !ehr_exists(&app, &b).await,
        "second repeated ehr_id deleted"
    );
}

#[tokio::test]
async fn disabled_admin_config_is_405() {
    // The config view shares the group gate: with the admin API disabled it
    // answers 405 Method Not Allowed (never a 403) — the status the OAS
    // declares for a disabled admin operation
    // (`admin_ehr_delete_all.yaml` + `responses/405.yaml`), applied
    // uniformly across the group.
    let (_pg, app) = app(false).await;
    let (status, headers, body) =
        common::send(&app, common::get(&format!("{BASE}/admin/config"))).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    // Mandatory on every 405 (RFC 9110 §15.5.6), empty because the resource
    // currently allows no methods at all (RFC 9110 §10.2.1) — see
    // `disabled_admin_is_405_and_never_deletes`.
    assert_eq!(allow_of(&headers), "");
    // …and the openEHR `{ error, message }` body, as on every other error path.
    let v: serde_json::Value = serde_json::from_str(&body).expect("openEHR error body");
    assert_eq!(v["error"], "Method Not Allowed");
}

#[tokio::test]
async fn enabled_admin_config_is_200_json() {
    // With the admin API enabled (and auth off in this fixture) the config view
    // returns 200 with a JSON body. The redaction of every secret field is
    // proven by the unit test on `FerroEhrConfig::to_redacted_json`; here the
    // point is the route is mounted and served under the enabled gate.
    let (_pg, app) = app(true).await;
    let (status, _body) =
        common::send_body(&app, common::get(&format!("{BASE}/admin/config"))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn enabled_delete_all_missing_list_deletes_all() {
    // `parameters/query/ehr_id_Admin.yaml`: `ehr_id` is "an optional parameter
    // to perform the operation on a subset of EHRs" — an absent list means
    // "delete ALL EHRs" (`admin_ehr_delete_all.yaml:5`), expressed to the
    // backend as the empty list.
    let (_pg, app) = app(true).await;
    let a = common::create_ehr(&app).await;
    let b = common::create_ehr(&app).await;
    let (status, body) =
        common::send_body(&app, common::delete(&format!("{BASE}/admin/ehr/all"))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty(), "204 carries no body, got {body:?}");
    // RE-TARGET (was `calls == 1`): the all-EHRs request removed every EHR.
    assert!(!ehr_exists(&app, &a).await, "all EHRs deleted (a)");
    assert!(!ehr_exists(&app, &b).await, "all EHRs deleted (b)");
}

// ── the two admin-extension deletes (issue #2130) ────────────────────────────
// DELETE /admin/template/{template_id} and
// DELETE /admin/query/{qualified_query_name}/{version} realize NO SM
// operation (vocab/wire_surface.yaml, family admin-extension-routes), so no
// catalogue case can bind them; this battery is where their functional
// branches are pinned. NOTE: no openEHR spec governs these routes — our own
// design/extension; the outcome shapes mirror the released admin deletes.

async fn upload_ips_template(app: &Router) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(common::ips_opt_xml()))
        .unwrap();
    let (status, body) = common::send_body(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
}

fn ips_template_path() -> String {
    format!("{BASE}/admin/template/International%20Patient%20Summary")
}

#[tokio::test]
async fn template_delete_is_204_and_the_template_is_gone() {
    let (_pg, app) = app(true).await;
    upload_ips_template(&app).await;
    let (status, body) = common::send_body(&app, common::delete(&ips_template_path())).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "template delete: {body}");
    assert!(body.is_empty(), "204 carries no body, got {body:?}");
    let (status, _body) = common::send_body(
        &app,
        common::get(&format!(
            "{BASE}/definition/template/adl1.4/International%20Patient%20Summary"
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "deleted template still served"
    );
}

#[tokio::test]
async fn template_delete_unknown_is_404() {
    let (_pg, app) = app(true).await;
    let (status, _body) = common::send_body(
        &app,
        common::delete(&format!("{BASE}/admin/template/no-such-template")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn template_delete_still_referenced_is_409() {
    // A template referenced by a committed version is never orphaned: the
    // committed COMPOSITION's archetype_details names the template, so the
    // physical delete refuses with 409 and the template stays served.
    let (_pg, app) = app(true).await;
    upload_ips_template(&app).await;
    let ehr_id = common::create_ehr(&app).await;
    let (status, body) = common::send_body(
        &app,
        common::post_json(
            &format!("{BASE}/ehr/{ehr_id}/composition"),
            &common::ips_canonical_composition().to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "referencing commit: {body}");
    let (status, body) = common::send_body(&app, common::delete(&ips_template_path())).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "in-use template delete: {body}"
    );
    let (status, _body) = common::send_body(
        &app,
        common::get(&format!(
            "{BASE}/definition/template/adl1.4/International%20Patient%20Summary"
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "refused delete must leave the template served"
    );
}

#[tokio::test]
async fn query_version_delete_is_204_and_that_version_is_gone() {
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/definition/query/cnf.admin::probe/1.0.0"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("SELECT e/ehr_id/value FROM EHR e"))
        .unwrap();
    let (status, body) = common::send_body(&app, req).await;
    assert_eq!(status, StatusCode::OK, "query store: {body}");
    let (status, body) = common::send_body(
        &app,
        common::delete(&format!("{BASE}/admin/query/cnf.admin::probe/1.0.0")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "query version delete: {body}"
    );
    assert!(body.is_empty(), "204 carries no body, got {body:?}");
    let (status, _body) = common::send_body(
        &app,
        common::get(&format!("{BASE}/definition/query/cnf.admin::probe/1.0.0")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "deleted query version still served"
    );
}

#[tokio::test]
async fn query_version_delete_unknown_is_404() {
    let (_pg, app) = app(true).await;
    let (status, _body) = common::send_body(
        &app,
        common::delete(&format!("{BASE}/admin/query/cnf.admin::absent/9.9.9")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
