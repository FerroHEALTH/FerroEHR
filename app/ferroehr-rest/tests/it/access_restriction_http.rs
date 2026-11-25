// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 8 restriction of access by health professionals, end to end
//! over the assembled router (#3682), as a `restriction` directive of the
//! EHR's `EHR_ACCESS` the controller commits (#5510).
//!
//! Regulation (EU) 2025/327 Art. 8 lets a person restrict the access of health
//! professionals and healthcare providers to all or parts of their data and
//! keeps the restriction "not visible to healthcare providers"; Art. 11(5) lets
//! the restricted data be reached in the vital interest of the person, logged
//! for them (`docs/law/eu/ehds/text.html`). The tests assert that a restricted
//! composition and a restricted EHR answer every professional route as an
//! unknown one does, that a query and an export leave them out, that only a
//! caller the policy's emergency justification names, stating a reason,
//! reads them, with the justification and its reason on its record, and that
//! a later version without the directive brings them back. No openEHR spec governs restriction of access:
//! our own design/extension.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
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
/// The header the default configuration reads the reason for a justified
/// access from.
const REASON: &str = "x-access-reason";
/// The purpose that, with a reason, declares the medic's emergency, and the
/// reason the medic states.
const STATED: (&str, &str) = ("ETREAT", "Anaphylaxis%2C%20unresponsive");
/// A versioned-object id no composition holds.
const UNKNOWN_VO: &str = "11111111-2222-4333-8444-555555555555";
/// An EHR id no EHR holds.
const UNKNOWN_EHR: &str = "66666666-7777-4888-9999-000000000000";

/// The `persons` issuer of the medic, the one caller the policy's emergency
/// justification names.
const MEDICS: &str = "https://medics.example";

/// The credential of `name`: the medic's token of [`MEDICS`], or a Basic
/// user's.
fn basic(name: &str) -> String {
    use base64::Engine;
    if name == "medic" {
        let exp = jiff::Timestamp::now().as_second() + 3600;
        return common::bearer(&json!({
            "sub": "medic", "iss": MEDICS, "aud": "ferroehr", "exp": exp,
            "roles": ["USER"],
        }));
    }
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app with `TREAT` and `ETREAT` agreed purposes, Basic users for
/// an administrator and a clinician, a medic of [`MEDICS`], an access policy
/// whose emergency justification for `ETREAT` names the medic, and the IPS
/// template uploaded.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        common::binding_service(&pool)
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[("root", &["ADMIN"]), ("clinician", &["USER"])],
    )
    .await;
    common::add_issuer(
        &mut config,
        ferrohealth_smart::config::IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            ..common::entry(MEDICS, "ferroehr")
        },
    );
    common::bind_issuer(&mut config, MEDICS);
    common::bind_subjects(&pool, &[(MEDICS, "medic")]).await;
    common::grant_to(
        &svc,
        &pool,
        "root",
        ferroehr::party_binding::grants::operation::Operation::WriteAccessPolicy,
    )
    .await;
    let medic = common::issuer_person(&pool, MEDICS, "medic").await;
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
    let app = common::build_full(config, svc, authz, Observability::default()).await;
    put_policy(&app, medic).await;
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

/// Writes the controller's first policy as the administrator, whose
/// emergency justification for `ETREAT` names the `PERSON` `medic`.
async fn put_policy(app: &Router, medic: uuid::Uuid) {
    let archetype = "openEHR-EHR-ACCESS_POLICY.controller.v1";
    let policy = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": false,
        "purpose_required": false,
        "justifications": [{ "_type": "ACCESS_JUSTIFICATION", "emergency": true,
            "purposes": [{ "_type": "DV_CODED_TEXT", "value": "emergency treatment",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_ActReason" },
                    "code_string": "ETREAT" } }],
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": {
                "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": medic.to_string() } } }] }]
    });
    let (status, _, body) = common::send(
        app,
        Request::put(format!("{EXT}/admin/access_policy"))
            .header(header::AUTHORIZATION, basic("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_NONE_MATCH, "*")
            .body(Body::from(policy.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the policy: {body}");
}

/// Sends `method uri` as `user`, with an optional declared purpose and body.
/// [`STATED`] declares its purpose with its reason.
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
        match purpose.split_once('|') {
            Some((code, reason)) => {
                req = req.header(PURPOSE, code).header(REASON, reason);
            }
            None => req = req.header(PURPOSE, purpose),
        }
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

/// Creates an EHR as the clinician and returns its id.
async fn ehr(app: &Router) -> String {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "clinician", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// Commits the IPS composition into `ehr` as the clinician; returns its
/// versioned-object id.
async fn commit(app: &Router, ehr: &str) -> String {
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
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned()
}

/// The current `EHR_ACCESS` version uid and settings of `ehr`, as the admin
/// reads them.
async fn current_settings(app: &Router, ehr: &str) -> (String, Value) {
    let (status, body) = send(
        app,
        "GET",
        &format!("{EXT}/admin/ehr-access?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "settings read: {body}");
    let read: Value = serde_json::from_str(&body).expect("settings json");
    let current = &read["versions"][0];
    (
        current["version_uid"]
            .as_str()
            .expect("version uid")
            .to_owned(),
        current["settings"].clone(),
    )
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

/// The admin records (`lift = false`) a restriction directive of `vo`, or of
/// the whole EHR, or lifts (`lift = true`) every restriction at that grain, by
/// committing a new `EHR_ACCESS` version on the person's instruction.
async fn mark(app: &Router, ehr: &str, vo: Option<&str>, lift: bool) {
    let (preceding, settings) = current_settings(app, ehr).await;
    let mut directives: Vec<Value> = settings["directives"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let grain = |directive: &Value| {
        directive["data"][0]["objects"][0]["id"]["value"]
            .as_str()
            .map(str::to_owned)
    };
    if lift {
        directives.retain(|directive| grain(directive).as_deref() != vo);
    } else {
        let objects: Vec<&str> = vo.into_iter().collect();
        directives.push(common::access_directive("restriction", &objects));
    }
    let (status, response) = commit_settings(
        app,
        ehr,
        &preceding,
        &common::directive_settings(&directives),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "lift={lift}: {response}");
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
            json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" }).to_string(),
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

/// A restricted composition answers every professional read as an unknown one
/// does, leaves the query result and the export, and comes back on a lift.
#[tokio::test]
async fn a_restricted_composition_reads_as_absent_until_lifted() {
    let (_db, _pool, app) = Box::pin(app()).await;
    let ehr = ehr(&app).await;
    let kept = commit(&app, &ehr).await;
    let hidden = commit(&app, &ehr).await;
    let read = |vo: String| {
        let app = app.clone();
        let ehr = ehr.clone();
        async move {
            send(
                &app,
                "GET",
                &format!("{BASE}/ehr/{ehr}/composition/{vo}"),
                "clinician",
                None,
                None,
            )
            .await
        }
    };
    assert_eq!(read(hidden.clone()).await.0, StatusCode::OK);

    mark(&app, &ehr, Some(&hidden), false).await;

    let (status, body) = read(hidden.clone()).await;
    let (unknown_status, unknown_body) = read(UNKNOWN_VO.to_owned()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, unknown_status);
    assert_eq!(error_kind(&body), error_kind(&unknown_body));
    assert!(!body.to_lowercase().contains("restrict"), "{body}");
    for path in [
        format!("{BASE}/ehr/{ehr}/versioned_composition/{hidden}"),
        format!("{BASE}/ehr/{ehr}/versioned_composition/{hidden}/revision_history"),
    ] {
        let (status, body) = send(&app, "GET", &path, "clinician", None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
    let (status, body) = send(
        &app,
        "DELETE",
        &format!("{BASE}/ehr/{ehr}/composition/{hidden}::local.ehrbase.org::1"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a write is refused as absent: {body}"
    );

    let uids = composition_uids(&app, "clinician", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&kept)), "{uids:?}");
    assert!(
        !uids.iter().any(|uid| uid.starts_with(&hidden)),
        "the query leaves the restricted composition out: {uids:?}"
    );
    let (status, export) = send(
        &app,
        "GET",
        &format!("{EXT}/message/export/{ehr}"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "export: {export}");
    assert!(!export.contains(&hidden), "the export leaves it out");
    assert!(export.contains(&kept), "the export keeps the rest");

    mark(&app, &ehr, Some(&hidden), true).await;
    assert_eq!(
        read(hidden.clone()).await.0,
        StatusCode::OK,
        "a lift brings it back"
    );
    let uids = composition_uids(&app, "clinician", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&hidden)), "{uids:?}");
}

/// A whole-EHR restriction makes the EHR answer as an unknown EHR does, by id,
/// in a scoped query, and for a write, and keeps its compositions out of a
/// population query; the admin still reads the restriction in the settings.
#[tokio::test]
async fn a_restricted_ehr_reads_as_absent() {
    let (_db, _pool, app) = Box::pin(app()).await;
    let ehr = ehr(&app).await;
    let vo = commit(&app, &ehr).await;
    mark(&app, &ehr, None, false).await;

    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "clinician",
        None,
        None,
    )
    .await;
    let (unknown_status, unknown_body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{UNKNOWN_EHR}"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, unknown_status);
    assert_eq!(error_kind(&body), error_kind(&unknown_body));

    let (status, body) = send(
        &app,
        "GET",
        &format!(
            "{BASE}/query/aql?q={}&ehr_id={ehr}",
            urlencoding::encode("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c")
        ),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a scoped query names no such EHR: {body}"
    );

    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/composition"),
        "clinician",
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
        "a write to it is refused as absent: {body}"
    );

    let uids = composition_uids(&app, "clinician", None).await;
    assert!(!uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let (_, settings) = current_settings(&app, &ehr).await;
    let directives = settings["directives"].as_array().expect("directives");
    assert_eq!(directives.len(), 1, "{settings}");
    assert_eq!(
        directives[0]["kind"]["defining_code"]["code_string"],
        json!("restriction")
    );
}

/// Only the caller the emergency justification names, declaring its purpose
/// with a reason, reads restricted data, and that access carries the
/// justification and the reason; a caller it does not name, no purpose, or
/// the purpose without a reason reads it as absent, so nothing says the data
/// are restricted.
#[tokio::test]
async fn only_an_emergency_access_reads_restricted_data_and_is_marked() {
    let (_db, pool, app) = Box::pin(app()).await;
    let ehr = ehr(&app).await;
    let vo = commit(&app, &ehr).await;
    mark(&app, &ehr, Some(&vo), false).await;
    let path = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let stated = format!("{}|{}", STATED.0, STATED.1);

    for (user, purpose) in [
        ("clinician", Some(stated.as_str())),
        ("medic", None),
        ("medic", Some("TREAT")),
    ] {
        let (status, body) = send(&app, "GET", &path, user, purpose, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{user} {purpose:?}: {body}");
    }
    let (status, body) = send(&app, "GET", &path, "medic", Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no reason: {body}");
    assert!(!body.contains("refused-"), "nothing says why: {body}");
    let (status, body) = send(&app, "GET", &path, "medic", Some(&stated), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the emergency access reads it: {body}"
    );
    let uids = composition_uids(&app, "medic", Some(&stated)).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let marked: Option<Option<Value>> = sqlx::query_scalar(
            "SELECT justification FROM audit.access_trail \
             WHERE operation = 'composition_get' AND outcome = 0 AND principal = 'medic' \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(marked) = marked {
            assert_eq!(
                marked,
                Some(json!({ "kind": "emergency", "reason": "Anaphylaxis, unresponsive" })),
                "the emergency access carries the justification and its reason"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the emergency read was not recorded"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A malformed EHR id is refused `400` naming it, on recording, lifting and
/// reading an access restriction alike, and a restriction naming an object by
/// an id that names no versioned object is refused `422` naming it, content
/// that reads but breaks a rule (RFC 9110 §15.5.21); nothing is committed.
#[tokio::test]
async fn a_malformed_id_is_refused_on_every_restriction_route() {
    let (_db, _pool, app) = Box::pin(app()).await;
    let ehr = ehr(&app).await;
    let (preceding, _) = current_settings(&app, &ehr).await;
    let whole = common::directive_settings(&[common::access_directive("restriction", &[])]);
    for settings in [whole, common::directive_settings(&[])] {
        let (status, answer) = commit_settings(&app, "not-a-uuid", &preceding, &settings).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert!(answer.contains("invalid EHR id: not-a-uuid"), "{answer}");
    }
    let object =
        common::directive_settings(&[common::access_directive("restriction", &["not-a-uuid"])]);
    let (status, answer) = commit_settings(&app, &ehr, &preceding, &object).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(
        answer.contains("not-a-uuid") && answer.contains("names no versioned object"),
        "{answer}"
    );
    assert_eq!(
        current_settings(&app, &ehr).await.0,
        preceding,
        "nothing is committed"
    );
    let (status, answer) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/ehr-access?ehr_id=not-a-uuid"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "register: {answer}");
    assert!(answer.contains("invalid EHR id: not-a-uuid"), "{answer}");
}
