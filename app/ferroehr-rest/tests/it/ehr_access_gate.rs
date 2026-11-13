// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end `EHR_ACCESS` gate over the real axum app.
//!
//! `EHR_ACCESS` is the spec-grounded access-decision authority ("All access
//! decisions to data in the EHR must be made in accordance with the policies
//! and rules in this object" — RM `org.openehr.rm.ehr.ehr_access.adoc`); the
//! gate runs before dispatch on every EHR-scoped route. These tests drive the
//! assembled router with `tower`'s `oneshot`, over the **real** `FerroEhrService`
//! (the scripted `Mock`/scheme hook is gone). The per-test scheme is
//! now a **real committed `EHR_ACCESS` version** (the only write path is a
//! CONTRIBUTION — RM ehr `master04-ehr_package.adoc` §EHR Access), seeded at the
//! service level before the app is built.
//!
//! They assert: default-open admits anonymous-era flows; `restricted` denies a
//! non-listed principal (403) and admits a listed one; `role:` principals match;
//! the Composition privacy ceiling blocks a read above `max_level` and admits
//! one below (the level is drawn from the scheme's `composition_overrides`, so no
//! real Composition is needed); and the gate-keeper preflight blocks a
//! non-gate-keeper `EHR_ACCESS` commit (403) while admitting the gate-keeper.
//! `403` follows the ITS-REST 401/403 discipline.
//!
//! The concrete scheme is our own design — no openEHR spec governs it.
use ferroehr::service::version_update::WriteRequest;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::config::rest::ApiConfig;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;

use crate::common;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

const EHR_ID: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

// ── config + app assembly ─────────────────────────────────────────────────────

/// A config with Basic auth on (or off) and no OIDC. The four users cover the
/// `user:`/`role:` matching cases.
fn rest_config(auth_enabled: bool) -> AppConfig {
    let mut config = AppConfig {
        server: common::server(),
        api: ApiConfig {
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        ..Default::default()
    };
    common::basic_users(&mut config, USERS);
    config.auth.shared.enabled = auth_enabled;
    config
}

/// The Basic users: three granted no role and a nurse.
const USERS: &[(&str, &[&str])] = &[
    ("alice", &[]),
    ("bob", &[]),
    ("carol", &[]),
    ("nadia", &["NURSE"]),
];

/// The committer a contribution names: the placeholder naming the Basic user
/// `alice` by its login, since a Basic user commits as its bound `PERSON`,
/// which the server writes.
fn committer() -> Value {
    json!({ "_type": "PARTY_IDENTIFIED",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "ferroehr", "id": "alice" }] })
}

fn coded(code: &str, value: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code
        }
    })
}

/// Commit a new `EHR_ACCESS` version carrying `scheme` as its settings, updating
/// the EHR's default (settings-less, default-open) `EHR_ACCESS`.
async fn seed_scheme(svc: &FerroEhrService, ehr_id: ferroehr::ids::EhrId, scheme: &Value) {
    let ehr = svc.ehr_read(ehr_id).await.expect("ehr object").body;
    svc.create_ehr_contribution(ehr_id, access_contribution(&ehr, scheme))
        .await
        .expect("EHR_ACCESS scheme contribution must commit");
}

/// The CONTRIBUTION that commits `scheme` as the second `EHR_ACCESS` version of
/// the EHR `ehr` (its `EHR` object) names.
fn access_contribution(ehr: &Value, scheme: &Value) -> Value {
    let access_vo = ehr["ehr_access"]["id"]["value"]
        .as_str()
        .expect("EHR.ehr_access.id.value");
    let preceding = format!("{access_vo}::{DEFAULT_SYSTEM_ID}::1");
    json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "commit_audit": { "change_type": coded("251", "modification"), "committer": committer() },
            "lifecycle_state": coded("532", "complete"),
            "preceding_version_uid": { "value": preceding },
            "data": {
                "_type": "EHR_ACCESS",
                "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1",
                // Roots carry ARCHETYPED (LOCATABLE.Archetyped_valid).
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID",
                                      "value": "openEHR-EHR-EHR_ACCESS.generic.v1" },
                    "rm_version": "1.2.0"
                },
                "name": { "_type": "DV_TEXT", "value": "EHR Access" },
                "settings": scheme
            }
        } ],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer() }
    })
}

/// Build the app over a real service whose EHR (`EHR_ID`) carries `scheme`
/// (when `Some`) — otherwise it keeps the default-open `EHR_ACCESS`. `authz` is
/// `None` throughout, so the `EHR_ACCESS` gate is the only thing that can produce
/// a 403 (isolating what we assert).
async fn app(auth_enabled: bool, scheme: Option<Value>) -> (testkit::TestDb, Router) {
    let (pg, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id: ferroehr::ids::EhrId = EHR_ID.parse().expect("valid ehr uuid");
    Box::pin(svc.create_ehr_with_id_meta(ehr_id, WriteRequest::new(None)))
        .await
        .expect("create ehr");
    if let Some(scheme) = scheme {
        seed_scheme(&svc, ehr_id, &scheme).await;
    }
    common::grant_basic_users(&svc, &pool, USERS).await;
    let app = common::build_full(
        rest_config(auth_enabled),
        Arc::new(svc),
        None,
        Observability::default(),
    )
    .await;
    (pg, app)
}

// ── requests ──────────────────────────────────────────────────────────────────

fn basic(name: &str) -> String {
    format!("Basic {}", base64_encode(format!("{name}:pw").as_bytes()))
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn req(method: &str, path: &str, authorization: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(common::full(path))
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9");
    if let Some(auth) = authorization {
        b = b.header("authorization", auth);
    }
    b.body(Body::from(body.to_owned())).expect("request")
}

async fn status(app: &Router, request: Request<Body>) -> StatusCode {
    app.clone()
        .oneshot(request)
        .await
        .expect("oneshot")
        .status()
}

// ── tests ───────────────────────────────────────────────────────────────────

/// Default-open (no scheme settings) admits an anonymous request — the gate
/// never turns a working, unconfigured EHR away (`master07` "sensible
/// defaults"). The backend then answers; the point is it is NOT a `403`.
#[tokio::test]
async fn default_open_admits_anonymous() {
    let (_pg, app) = app(false, None).await;
    let code = status(&app, req("GET", &format!("/ehr/{EHR_ID}"), None, "")).await;
    assert_ne!(
        code,
        StatusCode::FORBIDDEN,
        "default-open must not gate anonymous"
    );
}

/// `restricted` denies a principal absent from the access list (403) and admits
/// a listed one.
#[tokio::test]
async fn restricted_gates_by_user_principal() {
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "default_access": "restricted",
        "access_list": [ { "principal": "user:bob", "access": "full" } ]
    });
    let (_pg, app) = app(true, Some(scheme)).await;

    let denied = status(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("carol")), ""),
    )
    .await;
    assert_eq!(
        denied,
        StatusCode::FORBIDDEN,
        "carol is not on the access list"
    );

    let allowed = status(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("bob")), ""),
    )
    .await;
    assert_ne!(
        allowed,
        StatusCode::FORBIDDEN,
        "bob is listed → gate passes"
    );
}

/// A `role:` access-list entry matches the caller's roles.
#[tokio::test]
async fn restricted_gates_by_role_principal() {
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "default_access": "restricted",
        "access_list": [ { "principal": "role:nurse", "access": "full" } ]
    });
    let (_pg, app) = app(true, Some(scheme)).await;

    let allowed = status(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("nadia")), ""),
    )
    .await;
    assert_ne!(
        allowed,
        StatusCode::FORBIDDEN,
        "nadia has role NURSE → gate passes"
    );

    let denied = status(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("alice")), ""),
    )
    .await;
    assert_eq!(denied, StatusCode::FORBIDDEN, "alice has no matching role");
}

/// The Composition privacy ceiling blocks a read of a Composition raised above
/// the caller's `max_level` and admits one at/below it. The level is drawn from
/// the scheme's `composition_overrides`, so no real Composition is required.
#[tokio::test]
async fn privacy_ceiling_gates_composition_reads() {
    // vo head of the "high" composition uid the override pins to level 3.
    let high = "8849182c-82ad-4088-a07f-48ead4180515";
    let low = "11111111-1111-4111-8111-111111111111";
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "default_access": "open",
        "access_list": [ { "principal": "role:nurse", "access": "restricted_below", "max_level": 2 } ],
        "privacy": {
            "default_level": 0,
            "composition_overrides": [ { "uid": high, "level": 3 } ]
        }
    });
    let (_pg, app) = app(true, Some(scheme)).await;

    // nadia (NURSE, ceiling 2): the level-3 composition is blocked.
    let blocked = status(
        &app,
        req(
            "GET",
            &format!("/ehr/{EHR_ID}/composition/{high}::sys::1"),
            Some(&basic("nadia")),
            "",
        ),
    )
    .await;
    assert_eq!(blocked, StatusCode::FORBIDDEN, "level 3 >= ceiling 2");

    // A default-level (0) composition is readable (0 < 2).
    let readable = status(
        &app,
        req(
            "GET",
            &format!("/ehr/{EHR_ID}/composition/{low}::sys::1"),
            Some(&basic("nadia")),
            "",
        ),
    )
    .await;
    assert_ne!(
        readable,
        StatusCode::FORBIDDEN,
        "level 0 < ceiling 2 → gate passes"
    );
}

/// The gate-keeper preflight: a CONTRIBUTION carrying an `EHR_ACCESS` version is
/// refused (403) for a non-gate-keeper and admitted for the gate-keeper.
#[tokio::test]
async fn gate_keeper_guards_ehr_access_commits() {
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "gate_keeper": "user:alice",
        "default_access": "open"
    });
    let (_pg, app) = app(true, Some(scheme)).await;

    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "data": { "_type": "EHR_ACCESS", "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1" }
        } ],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": { "_type": "PARTY_IDENTIFIED", "name": "x" } }
    })
    .to_string();

    let denied = status(
        &app,
        req(
            "POST",
            &format!("/ehr/{EHR_ID}/contribution"),
            Some(&basic("bob")),
            &contribution,
        ),
    )
    .await;
    assert_eq!(denied, StatusCode::FORBIDDEN, "bob is not the gate-keeper");

    let allowed = status(
        &app,
        req(
            "POST",
            &format!("/ehr/{EHR_ID}/contribution"),
            Some(&basic("alice")),
            &contribution,
        ),
    )
    .await;
    assert_ne!(
        allowed,
        StatusCode::FORBIDDEN,
        "alice is the gate-keeper → gate passes"
    );
}

/// A CONTRIBUTION that does NOT carry an `EHR_ACCESS` version is not gate-kept,
/// even from a non-gate-keeper (the preflight is scoped to `EHR_ACCESS` writes).
#[tokio::test]
async fn gate_keeper_ignores_non_ehr_access_contributions() {
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "gate_keeper": "user:alice",
        "default_access": "open"
    });
    let (_pg, app) = app(true, Some(scheme)).await;

    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [ { "_type": "ORIGINAL_VERSION", "data": { "_type": "COMPOSITION" } } ],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": { "_type": "PARTY_IDENTIFIED", "name": "x" } }
    })
    .to_string();

    let code = status(
        &app,
        req(
            "POST",
            &format!("/ehr/{EHR_ID}/contribution"),
            Some(&basic("bob")),
            &contribution,
        ),
    )
    .await;
    assert_ne!(
        code,
        StatusCode::FORBIDDEN,
        "a COMPOSITION contribution is not gate-kept"
    );
}

// ── AQL ───────────────────────────────────────────────────────────────────────

/// A second EHR with the default-open `EHR_ACCESS`, so a population query has a
/// readable EHR beside the withheld one.
const OPEN_EHR_ID: &str = "7c1b9d3e-2a45-4f6b-9c8d-0e1f2a3b4c5d";
const STORED_NAME: &str = "org.example.test::ehr_access_population";

/// The router over [`EHR_ID`] (restricted to `user:bob`) and [`OPEN_EHR_ID`]
/// (default-open), with Basic auth on.
async fn query_app() -> (testkit::TestDb, Router) {
    query_app_with(None).await
}

/// [`query_app`], with `stored_default_access` written over the committed
/// `default_access` of [`EHR_ID`] in storage when it is `Some`: settings that
/// no longer parse, as only a write before the write-time check could store.
async fn query_app_with(stored_default_access: Option<&str>) -> (testkit::TestDb, Router) {
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "default_access": "restricted",
        "access_list": [ { "principal": "user:bob", "access": "full" } ]
    });
    let (pg, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id: ferroehr::ids::EhrId = EHR_ID.parse().expect("valid ehr uuid");
    let open_id: ferroehr::ids::EhrId = OPEN_EHR_ID.parse().expect("valid ehr uuid");
    Box::pin(svc.create_ehr_with_id_meta(ehr_id, WriteRequest::new(None)))
        .await
        .expect("create ehr");
    Box::pin(svc.create_ehr_with_id_meta(open_id, WriteRequest::new(None)))
        .await
        .expect("create open ehr");
    seed_scheme(&svc, ehr_id, &scheme).await;
    if let Some(stored) = stored_default_access {
        sqlx::query(
            "UPDATE version SET body = jsonb_set(body::jsonb, '{settings,default_access}', \
             to_jsonb($2::text))::text WHERE kind = 'EHR_ACCESS' AND ehr_id = $1",
        )
        .bind(ehr_id)
        .bind(stored)
        .execute(&pool)
        .await
        .expect("store settings that do not parse");
    }
    common::grant_basic_users(&svc, &pool, USERS).await;
    let app = common::build_full(
        rest_config(true),
        Arc::new(svc),
        None,
        Observability::default(),
    )
    .await;
    (pg, app)
}

/// The `ehr_id` values of a `RESULT_SET`'s first column, sorted.
async fn ehr_ids_of(app: &Router, request: Request<Body>) -> Vec<String> {
    let (code, result) = common::send_json(app, request).await;
    assert_eq!(code, StatusCode::OK, "{result}");
    let mut ids: Vec<String> = result["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].as_str().expect("ehr_id text").to_owned())
        .collect();
    ids.sort();
    ids
}

fn aql(q: &str, user: &str) -> Request<Body> {
    req(
        "POST",
        "/query/aql",
        Some(&basic(user)),
        &json!({ "q": q }).to_string(),
    )
}

/// An EHR the caller may not read contributes no rows to any AQL form — scoped
/// by the `WHERE` clause, by the `EHR` predicate, by the `ehr_id` parameter, or
/// not at all — while the listed principal is served on every one (RM ehr
/// `ehr_access.adoc` §`EHR_ACCESS` Class).
#[tokio::test]
async fn aql_withholds_an_ehr_the_caller_may_not_read() {
    let (_pg, app) = query_app().await;

    let control = status(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("carol")), ""),
    )
    .await;
    assert_eq!(
        control,
        StatusCode::FORBIDDEN,
        "the path gate refuses carol"
    );

    let by_where = format!("SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = '{EHR_ID}'");
    let by_predicate = format!("SELECT e/ehr_id/value FROM EHR e[ehr_id/value = '{EHR_ID}']");
    let population = "SELECT e/ehr_id/value FROM EHR e";
    let by_parameter = req(
        "POST",
        &format!("/query/aql?ehr_id={EHR_ID}"),
        Some(&basic("carol")),
        &json!({ "q": population }).to_string(),
    );

    assert_eq!(
        ehr_ids_of(&app, aql(&by_where, "carol")).await,
        Vec::<String>::new()
    );
    assert_eq!(
        ehr_ids_of(&app, aql(&by_predicate, "carol")).await,
        Vec::<String>::new()
    );
    assert_eq!(ehr_ids_of(&app, by_parameter).await, Vec::<String>::new());
    assert_eq!(
        ehr_ids_of(&app, aql(population, "carol")).await,
        vec![OPEN_EHR_ID.to_owned()],
        "the population query serves the readable EHR only"
    );

    assert_eq!(
        ehr_ids_of(&app, aql(&by_where, "bob")).await,
        vec![EHR_ID.to_owned()]
    );
    assert_eq!(
        ehr_ids_of(&app, aql(&by_predicate, "bob")).await,
        vec![EHR_ID.to_owned()]
    );
    let mut both = vec![EHR_ID.to_owned(), OPEN_EHR_ID.to_owned()];
    both.sort();
    assert_eq!(ehr_ids_of(&app, aql(population, "bob")).await, both);
}

/// The stored-query run path holds the same decision.
#[tokio::test]
async fn a_stored_query_withholds_an_ehr_the_caller_may_not_read() {
    let (_pg, app) = query_app().await;
    let mut store = req(
        "PUT",
        &format!("/definition/query/{STORED_NAME}"),
        Some(&basic("bob")),
        "SELECT e/ehr_id/value FROM EHR e",
    );
    store.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("text/plain"),
    );
    assert_eq!(
        status(&app, store).await,
        StatusCode::OK,
        "stored-query PUT"
    );

    let run = |user: &str| {
        req(
            "POST",
            &format!("/query/{STORED_NAME}"),
            Some(&basic(user)),
            "{}",
        )
    };
    assert_eq!(
        ehr_ids_of(&app, run("carol")).await,
        vec![OPEN_EHR_ID.to_owned()]
    );
    let mut both = vec![EHR_ID.to_owned(), OPEN_EHR_ID.to_owned()];
    both.sort();
    assert_eq!(ehr_ids_of(&app, run("bob")).await, both);
}

/// A `restricted` server default holds with the RBAC gate off: a setting-less
/// EHR is refused on its path and withheld from AQL for a caller without the
/// configured admin role, and served to one holding it. No openEHR spec governs
/// the server-wide default; our own design/extension.
#[tokio::test]
async fn a_restricted_default_holds_with_rbac_disabled() {
    let (_pg, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id: ferroehr::ids::EhrId = EHR_ID.parse().expect("valid ehr uuid");
    Box::pin(svc.create_ehr_with_id_meta(ehr_id, WriteRequest::new(None)))
        .await
        .expect("create ehr");

    let mut config = rest_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[
            ("alice", &[]),
            ("bob", &[]),
            ("carol", &[]),
            ("nadia", &["NURSE"]),
            ("root", &["SUPERADMIN"]),
        ],
    )
    .await;
    let mut authz = ferroehr::config::authz::AuthzConfig::default();
    authz.rbac.enabled = false;
    authz.rbac.ehr_access_default = ferroehr::config::authz::EhrAccessDefault::Restricted;
    authz.rbac.admin_role = "SUPERADMIN".to_owned();
    let handle = ferroehr_rest::extensions::access::authz::AuthzHandle::build(
        &authz,
        &config.api.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    assert!(
        handle.as_ref().is_some_and(|h| !h.rbac_active()),
        "a restricted default is carried with the coarse gate off"
    );
    let app = common::build_full(config, Arc::new(svc), handle, Observability::default()).await;

    assert_eq!(
        status(
            &app,
            req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("alice")), "")
        )
        .await,
        StatusCode::FORBIDDEN,
        "a setting-less EHR is refused to a caller without the admin role"
    );
    assert_ne!(
        status(
            &app,
            req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("root")), "")
        )
        .await,
        StatusCode::FORBIDDEN,
        "the configured admin role reaches it"
    );

    let by_where = format!("SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = '{EHR_ID}'");
    assert!(
        ehr_ids_of(&app, aql(&by_where, "alice")).await.is_empty(),
        "AQL withholds the setting-less EHR from a caller without the admin role"
    );
    assert_eq!(
        ehr_ids_of(&app, aql(&by_where, "root")).await,
        vec![EHR_ID.to_owned()]
    );
}

/// Settings that carry the scheme's `_type` but do not parse hold the EHR
/// closed: a read and an export of it are refused with `500`, and no query
/// reaches it, for the caller its access list would have named too. Before,
/// the EHR was served as if it carried no settings.
#[tokio::test]
async fn unreadable_settings_hold_the_ehr_closed() {
    let (_pg, app) = query_app_with(Some("bogus")).await;

    for user in ["bob", "carol"] {
        assert_eq!(
            status(
                &app,
                req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic(user)), "")
            )
            .await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "a read of the EHR is refused to {user}"
        );
        assert_eq!(
            status(
                &app,
                req(
                    "GET",
                    &format!("/message/export/{EHR_ID}"),
                    Some(&basic(user)),
                    ""
                )
            )
            .await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "an export of the EHR is refused to {user}"
        );
        assert_eq!(
            ehr_ids_of(&app, aql("SELECT e/ehr_id/value FROM EHR e", user)).await,
            vec![OPEN_EHR_ID.to_owned()],
            "the population query serves {user} the readable EHR only"
        );
    }
    assert_eq!(
        status(
            &app,
            req(
                "GET",
                &format!("/ehr/{OPEN_EHR_ID}"),
                Some(&basic("carol")),
                ""
            )
        )
        .await,
        StatusCode::OK,
        "the EHR beside it is served as before"
    );
}

/// A CONTRIBUTION that writes settings of the scheme which do not parse is
/// refused with `400` naming the setting, and nothing is committed; the same
/// write with settings that parse commits. Stored, the first would hold the
/// EHR closed.
#[tokio::test]
async fn a_write_of_settings_that_do_not_parse_is_refused() {
    let (_pg, app) = app(true, None).await;
    let (code, ehr) = common::send_json(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("alice")), ""),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{ehr}");
    let commit = |default_access: &str| {
        let scheme = json!({
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "default_access": default_access,
            "access_list": [ { "principal": "user:alice", "access": "full" } ]
        });
        req(
            "POST",
            &format!("/ehr/{EHR_ID}/contribution"),
            Some(&basic("alice")),
            &access_contribution(&ehr, &scheme).to_string(),
        )
    };

    let (code, refusal) = common::send_json(&app, commit("bogus")).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{refusal}");
    assert!(
        refusal
            .to_string()
            .contains("EHR_ACCESS.settings.default_access"),
        "the refusal names the setting that does not parse: {refusal}"
    );
    assert_eq!(
        status(
            &app,
            req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("carol")), "")
        )
        .await,
        StatusCode::OK,
        "nothing was committed: the EHR is still default-open"
    );

    assert_eq!(
        status(&app, commit("restricted")).await,
        StatusCode::CREATED,
        "settings that parse commit"
    );
    assert_eq!(
        status(
            &app,
            req("GET", &format!("/ehr/{EHR_ID}"), Some(&basic("carol")), "")
        )
        .await,
        StatusCode::FORBIDDEN,
        "and the committed settings now restrict the EHR"
    );
}

/// A `POST /message/export` manifest naming `ehr_ids`, for `user`.
fn manifest_export(ehr_ids: &[&str], user: &str) -> Request<Body> {
    let entities: Vec<Value> = ehr_ids
        .iter()
        .map(|id| json!({ "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": id, "ehr_id": id }))
        .collect();
    manifest_of(&entities, user)
}

/// A `POST /message/export` of a manifest carrying `entities`, for `user`.
fn manifest_of(entities: &[Value], user: &str) -> Request<Body> {
    let spec = json!({
        "_type": "EXTRACT_SPEC",
        "extract_type": coded("803", "openEHR EHR"),
        "include_multimedia": false,
        "priority": 0,
        "link_depth": 0,
        "manifest": { "_type": "EXTRACT_MANIFEST", "entities": entities }
    });
    req(
        "POST",
        "/message/export",
        Some(&basic(user)),
        &spec.to_string(),
    )
}

/// Every EHR a manifest export names passes the gate a read of it passes:
/// one the caller may not reach refuses the whole export with `403`, whatever
/// else the manifest names, and a caller the access list names is served
/// every extract. Before, the manifest form passed no `EHR_ACCESS` gate.
#[tokio::test]
async fn a_manifest_export_passes_the_gate_for_every_named_ehr() {
    let (_pg, app) = query_app_with(None).await;

    assert_eq!(
        status(&app, manifest_export(&[OPEN_EHR_ID, EHR_ID], "carol")).await,
        StatusCode::FORBIDDEN,
        "carol may not read the restricted EHR, so the export is refused"
    );
    assert_eq!(
        status(&app, manifest_export(&[EHR_ID], "carol")).await,
        StatusCode::FORBIDDEN,
        "the restricted EHR alone is refused too"
    );

    let (code, extracts) =
        common::send_json(&app, manifest_export(&[OPEN_EHR_ID, EHR_ID], "bob")).await;
    assert_eq!(code, StatusCode::OK, "{extracts}");
    assert_eq!(
        extracts.as_array().map(Vec::len),
        Some(2),
        "bob is listed: one extract per entity"
    );
    let (code, extracts) = common::send_json(&app, manifest_export(&[OPEN_EHR_ID], "carol")).await;
    assert_eq!(code, StatusCode::OK, "{extracts}");
    assert_eq!(extracts.as_array().map(Vec::len), Some(1));
}

/// A manifest naming an EHR held closed by settings that do not parse is
/// refused with `500` for every caller, as a read of that EHR is.
#[tokio::test]
async fn a_manifest_export_naming_a_held_closed_ehr_is_refused() {
    let (_pg, app) = query_app_with(Some("bogus")).await;

    for user in ["bob", "carol"] {
        assert_eq!(
            status(&app, manifest_export(&[OPEN_EHR_ID, EHR_ID], user)).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "the export naming the held-closed EHR is refused to {user}"
        );
    }
    assert_eq!(
        status(&app, manifest_export(&[OPEN_EHR_ID], "carol")).await,
        StatusCode::OK,
        "a manifest naming only the EHR beside it is served"
    );
}

/// A manifest entity named by `subject_id` passes the same gate: the EHR the
/// subject resolves to is refused to a caller off its access list, with `403`
/// for the whole export, and served to a listed one.
#[tokio::test]
async fn a_manifest_export_by_subject_id_passes_the_gate() {
    const SUBJECT: &str = "subject-4871";
    let (_pg, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id: ferroehr::ids::EhrId = EHR_ID.parse().expect("valid ehr uuid");
    let ehr_status: openehr_rm::prelude::EhrStatus =
        openehr_its::json::from_canonical_value(&json!({
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
                    "id": { "_type": "HIER_OBJECT_ID", "value": SUBJECT },
                    "namespace": "test.patients",
                    "type": "PERSON"
                }
            },
            "is_queryable": true,
            "is_modifiable": true
        }))
        .expect("a canonical EHR_STATUS");
    Box::pin(svc.create_ehr_with_id_meta(ehr_id, WriteRequest::new(Some(ehr_status))))
        .await
        .expect("create ehr");
    let scheme = json!({
        "_type": "FERROEHR_ACCESS_CONTROL_V1",
        "default_access": "restricted",
        "access_list": [ { "principal": "user:bob", "access": "full" } ]
    });
    seed_scheme(&svc, ehr_id, &scheme).await;
    common::grant_basic_users(&svc, &pool, USERS).await;
    let app = common::build_full(
        rest_config(true),
        Arc::new(svc),
        None,
        Observability::default(),
    )
    .await;
    let by_subject = || {
        vec![json!({
            "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": SUBJECT, "subject_id": SUBJECT
        })]
    };

    assert_eq!(
        status(&app, manifest_of(&by_subject(), "carol")).await,
        StatusCode::FORBIDDEN,
        "carol may not read the EHR the subject resolves to"
    );
    let (code, extracts) = common::send_json(&app, manifest_of(&by_subject(), "bob")).await;
    assert_eq!(code, StatusCode::OK, "{extracts}");
    assert_eq!(
        extracts.as_array().map(Vec::len),
        Some(1),
        "bob is listed: the subject's EHR is exported"
    );
}
