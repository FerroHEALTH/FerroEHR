// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 10 opt-out from access through the access services, end to
//! end over the assembled router (#4516), as an `opt_out` directive of the
//! EHR's `EHR_ACCESS` the controller commits (#5510).
//!
//! Regulation (EU) 2025/327 Art. 10(1) lets a person opt out from access to
//! their data "through the electronic health data access services referred to
//! in Articles 4 and 12", reversibly, and Art. 10(2) lets a healthcare
//! provider or health professional reach the data where processing is
//! necessary to protect vital interests (`docs/law/eu/ehds/text.html`). The
//! tests assert that an opted-out EHR answers an access-service caller at
//! every route, query and export as an unknown EHR does while every other
//! caller reads it as before, that a later version without it brings it back,
//! that an emergency purpose overrides it with the override on the trail,
//! that recording, reversing and reading it are access records and its history
//! is kept as `EHR_ACCESS` versions, and that recording is refused while the
//! deployment declares no access-service caller. No openEHR spec governs an
//! opt-out: our own design/extension.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;
use crate::common::EXT;

/// The header the default configuration reads the declared purpose from.
const PURPOSE: &str = "x-purpose-of-use";
/// An EHR id no EHR holds.
const UNKNOWN_EHR: &str = "66666666-7777-4888-9999-000000000000";
/// A role an access service's own credential may name, which grants nothing:
/// an access service is an `access_service` grant in the CDR (#5521).
const ACCESS_SERVICE: &str = "EHDS-ACCESS-SERVICE";

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app with `ETREAT` agreed as an emergency purpose, Basic users
/// for an administrator, a clinician of the provider's own care, an access
/// service and a caller naming [`ACCESS_SERVICE`] with no grant, and the IPS
/// template uploaded; `declared` decides whether `[authz.grants] allowed`
/// allows `access_service` and the CDR grants it to the access service.
async fn app(declared: bool) -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        emergency_purpose_codes: vec!["ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let allowed = if declared {
        vec![Operation::AccessService]
    } else {
        Vec::new()
    };
    let svc = Arc::new(
        common::binding_service(&pool)
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone()))
            .with_binding_mapping(crate::party_binding_http::mapping())
            .with_grant_ceiling(GrantCeiling::new(allowed)),
    );
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[
            ("root", &["ADMIN"]),
            ("clinician", &["USER"]),
            ("service", &["USER"]),
            ("named", &["USER", ACCESS_SERVICE]),
        ],
    )
    .await;
    if declared {
        common::grant_to(&svc, &pool, "service", Operation::AccessService).await;
    }
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
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/definition/template/adl1.4"),
        "root",
        None,
        Some(("application/xml", common::ips_opt_xml())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (db, pool, app)
}

/// Sends `method uri` as `user`, with an optional declared purpose and body.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    purpose: Option<&str>,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, basic(user))
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    if let Some(purpose) = purpose {
        req = req.header(PURPOSE, purpose);
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

/// Creates an EHR as the clinician and commits the IPS composition into it;
/// returns the EHR id and the composition's versioned-object id.
async fn ehr_with_composition(app: &Router) -> (String, String) {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "clinician", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let (status, headers, body) = common::send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::AUTHORIZATION, basic("clinician"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(common::ips_canonical_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let vo = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned();
    (ehr, vo)
}

/// Every `EHR_ACCESS` version of `ehr`, newest first, as the admin reads
/// them; the answer when the read is refused.
async fn versions(app: &Router, ehr: &str) -> Result<Vec<Value>, (StatusCode, String)> {
    let (status, body) = send(
        app,
        "GET",
        &format!("{EXT}/admin/ehr-access?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    if status != StatusCode::OK {
        return Err((status, body));
    }
    let read: Value = serde_json::from_str(&body).expect("settings json");
    Ok(read["versions"].as_array().expect("versions").clone())
}

/// Whether the settings of `version` hold an `opt_out` directive.
fn opts_out(version: &Value) -> bool {
    version["settings"]["directives"]
        .as_array()
        .is_some_and(|directives| {
            directives
                .iter()
                .any(|d| d["kind"]["defining_code"]["code_string"] == json!("opt_out"))
        })
}

/// The admin commits `settings` for `ehr` over the version `preceding`.
async fn commit_settings(
    app: &Router,
    ehr: &str,
    preceding: &str,
    settings: &Value,
) -> (StatusCode, String) {
    send(
        app,
        "POST",
        &common::full("/admin/ehr-access"),
        "root",
        None,
        Some((
            "application/json",
            json!({ "ehr_id": ehr, "preceding_version_uid": preceding, "settings": settings })
                .to_string(),
        )),
    )
    .await
}

/// The admin records (`reverse = false`) or reverses the opt-out of `ehr`, by
/// reading the current `EHR_ACCESS` and committing a version with or without
/// an `opt_out` directive; returns the answer.
async fn opt_out(app: &Router, ehr: &str, reverse: bool) -> (StatusCode, String) {
    let current = match versions(app, ehr).await {
        Ok(versions) => versions.first().cloned().expect("a current version"),
        Err(refused) => return refused,
    };
    let directives = if reverse {
        Vec::new()
    } else {
        vec![common::access_directive("opt_out", &[])]
    };
    let preceding = current["version_uid"].as_str().expect("version uid");
    commit_settings(
        app,
        ehr,
        preceding,
        &common::directive_settings(&directives),
    )
    .await
}

/// The composition uids an AQL population query returns to `user`.
async fn composition_uids(app: &Router, user: &str, purpose: Option<&str>) -> Vec<String> {
    let (status, body) = send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        user,
        purpose,
        Some((
            "application/json",
            json!({ "q": "SELECT c/uid/value FROM COMPOSITION c" }).to_string(),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {body}");
    let result: Value = serde_json::from_str(&body).expect("RESULT_SET JSON");
    result["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The problem type of an error body.
fn error_kind(body: &str) -> Value {
    serde_json::from_str::<Value>(body).expect("an error body")["type"].clone()
}

/// The `(action, principal, emergency_access)` of every stored record whose
/// resource is `resource`, polled until `expected` have landed.
async fn trail(pool: &PgPool, resource: &str, expected: usize) -> Vec<(String, String, bool)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
            "SELECT action, principal, emergency_access FROM audit.access_trail \
             WHERE resource_id = $1 ORDER BY chain_seq",
        )
        .bind(resource)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= expected {
            return rows
                .into_iter()
                .map(|(action, principal, emergency)| {
                    (action, principal.unwrap_or_default(), emergency)
                })
                .collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} records on {resource} landed",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// While the opt-out stands, the access service reads the EHR as absent at
/// every route, query and export, a write included, and the provider's own
/// clinician and a caller naming an access-service role with no grant read it
/// as before; the reversal brings it back.
#[tokio::test]
async fn an_opted_out_ehr_reads_as_absent_to_the_access_services_until_reversed() {
    let (_db, _pool, app) = Box::pin(app(true)).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let (_, kept) = ehr_with_composition(&app).await;
    let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    assert_eq!(
        send(&app, "GET", &composition, "service", None, None)
            .await
            .0,
        StatusCode::OK
    );

    let (status, body) = opt_out(&app, &ehr, false).await;
    assert_eq!(status, StatusCode::OK, "record: {body}");

    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "service",
        None,
        None,
    )
    .await;
    let (unknown_status, unknown_body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{UNKNOWN_EHR}"),
        "service",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, unknown_status);
    assert_eq!(error_kind(&body), error_kind(&unknown_body));
    // A caller naming an access-service role holds no grant, so the opt-out
    // does not bear on it.
    assert_eq!(
        send(&app, "GET", &composition, "named", None, None).await.0,
        StatusCode::OK
    );
    for path in [
        composition.clone(),
        format!("{BASE}/ehr/{ehr}/ehr_status"),
        format!("{BASE}/ehr/{ehr}/versioned_composition/{vo}"),
        format!("{EXT}/message/export/{ehr}"),
        format!(
            "{BASE}/query/aql?q={}&ehr_id={ehr}",
            urlencoding::encode("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c")
        ),
    ] {
        let (status, body) = send(&app, "GET", &path, "service", None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/composition"),
        "service",
        None,
        Some((
            "application/json",
            common::ips_canonical_composition().to_string(),
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a write is refused as absent: {body}"
    );
    let uids = composition_uids(&app, "service", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&kept)), "{uids:?}");
    assert!(
        !uids.iter().any(|uid| uid.starts_with(&vo)),
        "the population query leaves the opted-out EHR out: {uids:?}"
    );

    assert_eq!(
        send(&app, "GET", &composition, "clinician", None, None)
            .await
            .0,
        StatusCode::OK,
        "the provider's own care reads the EHR as before"
    );
    let uids = composition_uids(&app, "clinician", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let (status, body) = opt_out(&app, &ehr, true).await;
    assert_eq!(status, StatusCode::OK, "reverse: {body}");
    assert_eq!(
        send(&app, "GET", &composition, "service", None, None)
            .await
            .0,
        StatusCode::OK,
        "the reversal brings the EHR back to the access services"
    );
    let uids = composition_uids(&app, "service", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");
}

/// Recording, reversing and reading the opt-out are access records naming
/// the administrator, and the `EHR_ACCESS` versions keep the reversed opt-out.
#[tokio::test]
async fn recording_reversing_and_reading_the_opt_out_are_access_records() {
    let (_db, pool, app) = Box::pin(app(true)).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    assert_eq!(opt_out(&app, &ehr, false).await.0, StatusCode::OK);
    assert_eq!(opt_out(&app, &ehr, true).await.0, StatusCode::OK);
    let versions = versions(&app, &ehr).await.expect("the versions read");
    assert_eq!(versions.len(), 3, "{versions:?}");
    assert!(!opts_out(&versions[0]), "the reversal is current");
    assert!(opts_out(&versions[1]), "the reversed opt-out is kept");

    // Each act reads the current version before it commits over it.
    let actions: Vec<String> = trail(&pool, "ehr-access:ehr-wide", 5)
        .await
        .into_iter()
        .map(|(action, principal, emergency)| {
            assert_eq!(principal, "root");
            assert!(!emergency);
            action
        })
        .collect();
    assert_eq!(actions, ["R", "U", "R", "U", "R"]);
}

/// An access service declaring an emergency purpose reads the opted-out EHR
/// under Art. 10(2), with the emergency mark on its record and the override
/// recorded against the EHR; an ordinary purpose does not lift the opt-out.
#[tokio::test]
async fn the_vital_interest_override_reads_the_ehr_and_is_recorded() {
    let (_db, pool, app) = Box::pin(app(true)).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    assert_eq!(opt_out(&app, &ehr, false).await.0, StatusCode::OK);
    let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");

    let (status, body) = send(&app, "GET", &composition, "service", Some("TREAT"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an ordinary purpose: {body}");
    let (status, body) = send(&app, "GET", &composition, "service", Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::OK, "the override reads it: {body}");
    let uids = composition_uids(&app, "service", Some("ETREAT")).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    assert_eq!(
        trail(&pool, "access-opt-out-override:ehr-wide", 1).await,
        vec![("E".to_owned(), "service".to_owned(), true)]
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let marked: Option<bool> = sqlx::query_scalar(
            "SELECT emergency_access FROM audit.access_trail \
             WHERE operation = 'composition_get' AND outcome = 0 AND principal = 'service' \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(marked) = marked {
            assert!(marked, "the override access carries the emergency mark");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the override read was not recorded"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// While `[authz.grants] allowed` does not allow `access_service`, the opt-out
/// is off: recording one is refused and nothing is written, while reversing
/// and reading stay open.
#[tokio::test]
async fn recording_is_refused_while_no_access_service_is_declared() {
    let (_db, _pool, app) = Box::pin(app(false)).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let (status, body) = opt_out(&app, &ehr, false).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("access_service") && body.contains("[authz.grants] allowed"),
        "{body}"
    );
    assert_eq!(opt_out(&app, &ehr, true).await.0, StatusCode::OK);
    let versions = versions(&app, &ehr).await.expect("the versions read");
    assert!(
        !versions.iter().any(opts_out),
        "no version holds an opt-out: {versions:?}"
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("{BASE}/ehr/{ehr}/composition/{vo}"),
            "service",
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

/// A malformed EHR id is refused `400` on recording, reversing and reading the
/// opt-out alike.
#[tokio::test]
async fn a_malformed_ehr_id_is_refused_on_every_opt_out_route() {
    let (_db, _pool, app) = Box::pin(app(true)).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    let current = versions(&app, &ehr).await.expect("the versions read");
    let preceding = current[0]["version_uid"].as_str().expect("version uid");
    for directives in [vec![common::access_directive("opt_out", &[])], Vec::new()] {
        let settings = common::directive_settings(&directives);
        let (status, body) = commit_settings(&app, "not-a-uuid", preceding, &settings).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("invalid EHR id: not-a-uuid"), "{body}");
    }
    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/ehr-access?ehr_id=not-a-uuid"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "register: {body}");
    assert!(body.contains("invalid EHR id: not-a-uuid"), "{body}");
}

/// A canonical `EXTRACT_SPEC` whose manifest names `ehrs`, each by `ehr_id`.
fn manifest(ehrs: &[&str]) -> String {
    let entities: Vec<Value> = ehrs
        .iter()
        .map(|id| json!({ "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": id, "ehr_id": id }))
        .collect();
    json!({
        "_type": "EXTRACT_SPEC",
        "extract_type": {
            "_type": "DV_CODED_TEXT", "value": "openEHR EHR",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "803"
            }
        },
        "include_multimedia": false,
        "priority": 0,
        "link_depth": 0,
        "manifest": { "_type": "EXTRACT_MANIFEST", "entities": entities }
    })
    .to_string()
}

/// A manifest export naming an opted-out EHR reads it as absent to the access
/// service, and under the Art. 10(2) override exports it with the override
/// recorded once against the EHR, as a path read records it. Before, the
/// manifest form exported it under the override with no record.
#[tokio::test]
async fn a_manifest_export_under_the_override_records_it() {
    let (_db, pool, app) = Box::pin(app(true)).await;
    let (ehr, _vo) = ehr_with_composition(&app).await;
    let (kept, _) = ehr_with_composition(&app).await;
    assert_eq!(opt_out(&app, &ehr, false).await.0, StatusCode::OK);
    let uri = format!("{EXT}/message/export");
    let body = manifest(&[&kept, &ehr, &ehr]);
    let export = |purpose| {
        send(
            &app,
            "POST",
            &uri,
            "service",
            purpose,
            Some(("application/json", body.clone())),
        )
    };

    let (status, body) = export(Some("TREAT")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an ordinary purpose: {body}");

    let (status, body) = export(Some("ETREAT")).await;
    assert_eq!(status, StatusCode::OK, "the override exports it: {body}");
    let extracts: Value = serde_json::from_str(&body).expect("extract list");
    assert_eq!(extracts.as_array().map(Vec::len), Some(3));
    assert_eq!(
        trail(&pool, "access-opt-out-override:ehr-wide", 1).await,
        vec![("E".to_owned(), "service".to_owned(), true)],
        "one override record for the opted-out EHR the manifest names twice"
    );
}

/// Recording and reversing an opt-out each write an access-log entry for the
/// EHR, naming the `EHR_ACCESS` settings they commit (#5564).
#[tokio::test]
async fn recording_and_reversing_an_opt_out_each_write_an_entry() {
    let (_db, pool, app) = Box::pin(app(true)).await;
    let (ehr, _vo) = ehr_with_composition(&app).await;
    for (step, reverse) in [(1, false), (2, true)] {
        let (status, body) = opt_out(&app, &ehr, reverse).await;
        assert_eq!(status, StatusCode::OK, "opt-out {reverse}: {body}");
        let written = crate::access_log_http::entries(&pool, step, |s| {
            s.ehr_id.as_deref() == Some(ehr.as_str())
                && s.action.as_deref() == Some("update")
                && s.content.contains("\"ehr-access:ehr-wide\"")
        })
        .await;
        assert!(
            written
                .iter()
                .all(|s| s.result.as_deref() == Some("granted")),
            "{written:?}"
        );
    }
}

/// The canonical entry of the first stored access-log entry of `ehr` that
/// `keep` selects.
async fn entry_of(pool: &PgPool, ehr: &str, keep: impl Fn(&Value) -> bool) -> Value {
    let read = |stored: &crate::access_log_http::Stored| {
        serde_json::from_str::<Value>(&stored.content).expect("a canonical entry")
    };
    let kept = crate::access_log_http::entries(pool, 1, |stored| {
        stored.ehr_id.as_deref() == Some(ehr) && keep(&read(stored))
    })
    .await;
    read(&kept[0])
}

/// The absent answer of an opt-out is an access-log entry refused under
/// `refused-opt-out` with its step, though the `404` says nothing (EHDS Art.
/// 8); the vital-interest override is an entry marked as an emergency with a
/// ground naming the opt-out it lifted and the `EHR_ACCESS` version (EHDS
/// Annex II 3.2, Art. 10(2), Art. 11(5)).
#[tokio::test]
async fn the_absent_answer_and_the_override_are_recorded_with_their_grounds() {
    let (_db, pool, app) = Box::pin(app(true)).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    assert_eq!(opt_out(&app, &ehr, false).await.0, StatusCode::OK);
    let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");

    let (status, body) = send(&app, "GET", &composition, "service", Some("TREAT"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        !body.contains("refused-"),
        "the answer says nothing: {body}"
    );
    let refused = entry_of(&pool, &ehr, |entry| {
        entry["outcome"]["result"]["value"] == "refused"
    })
    .await;
    assert_eq!(
        refused["outcome"]["reason"]["defining_code"]["code_string"], "refused-opt-out",
        "{refused}"
    );
    let step = refused["outcome"]["description"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(
        step.starts_with("step 1 (opt-out): directive 1 of EHR_ACCESS "),
        "{step}"
    );

    let (status, body) = send(&app, "GET", &composition, "service", Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let marked = entry_of(&pool, &ehr, |entry| {
        entry["is_emergency"] == true && entry["outcome"]["result"]["value"] == "granted"
    })
    .await;
    let grounds = marked["authorisations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let lifted = grounds
        .iter()
        .find(|ground| ground["kind"]["value"] == "emergency")
        .unwrap_or_else(|| panic!("an emergency ground: {marked}"));
    assert_eq!(lifted["name"], "EHR_ACCESS", "{marked}");
    let version = lifted["version"].as_str().expect("the EHR_ACCESS version");
    assert!(
        grounds
            .iter()
            .any(|ground| ground["kind"]["value"] == "role" && ground["name"] == "access_service"),
        "the capability it acted under: {marked}"
    );
    assert!(version.contains("::"), "an OBJECT_VERSION_ID: {version}");
}
