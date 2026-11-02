// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The person's own access trail while a composition is held from them
//! (#4351): EHDS Art. 9 with Art. 3(3).
//!
//! Regulation (EU) 2025/327 Art. 9(1) gives the person the record of every
//! access to their data, and Art. 3(3) delays their access to newly
//! registered data "until a health professional is able to properly
//! communicate and explain" it (`docs/law/eu/ehds/text.html`). The tests
//! assert that the subject-scoped ITI-81 read, the person's own trail, leaves
//! out the records about a held composition until it is released or its
//! limit passes, and then serves them with their original time, while the
//! controller's read shows them throughout. No openEHR spec governs it: our
//! own design/extension.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_delay::config::{AccessDelayConfig, AccessDelayPolicy};
use ferroehr::system_log::categories::CategoryMapConfig;
use ferroehr::system_log::sender::{
    AuditHandle, ResolvedSubject, SubjectLookup, SubjectResolver, start,
};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;

use crate::common;
use crate::common::{BASE, EXT};

/// The patient every EHR of the deployment resolves to.
const PATIENT: &str = "patient-7";

/// A deployment that holds patient summaries for 72 hours, audits to the
/// local store with every EHR resolving to [`PATIENT`], and gates by role:
/// `dr` a professional, `root` the controller, `portal` the person's portal
/// holding the subject-scoped audit role.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = common::local_audit();
    let resolver: SubjectResolver = Arc::new(|_ehr_id| {
        Box::pin(async {
            SubjectLookup::Found(ResolvedSubject {
                id: PATIENT.to_owned(),
                namespace: Some("urn:ferroehr:test-subjects".to_owned()),
            })
        })
    });
    let (sender, _handle): (_, AuditHandle) = start(audit, Some(resolver), Some(pool.clone()))
        .await
        .expect("the audit sender");
    let classifier = CategoryMapConfig::default()
        .compile()
        .expect("the shipped map compiles");
    let section = AccessDelayConfig {
        enabled: true,
        categories: vec!["Patient-Summaries".to_owned()],
        hours: 72,
    };
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone()))
            .with_access_delay(AccessDelayPolicy::compile(&section, &classifier)),
    );
    svc.mirror_category_map(&classifier)
        .await
        .expect("mirror the category map");
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &[
            ("dr", &["USER"]),
            ("root", &["ADMIN"]),
            ("portal", &["PORTAL"]),
        ],
    );
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    authz.rbac.subject_audit_role = Some("PORTAL".to_owned());
    let handle = AuthzHandle::build(
        &authz,
        &config.rest.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = ferroehr_rest::build_full(config, svc, handle, Observability::default())
        .await
        .expect("build app");
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/definition/template/adl1.4"),
        Some(("application/xml", common::ips_opt_xml())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (db, pool, app)
}

/// Sends `method uri` as the professional `dr` with an optional body.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, common::basic_credential("dr"))
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    let req = match body {
        Some((content_type, body)) => req
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .unwrap();
    common::send_body(app, req).await
}

/// Creates an EHR as `dr`; returns its id.
async fn ehr(app: &Router) -> String {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// Commits the IPS composition into `ehr` as `dr`, which the deployment holds
/// from the person; returns its object id once the access record of the
/// commit is in the trail.
async fn commit(app: &Router, pool: &PgPool, ehr: &str) -> String {
    let (status, headers, body) = common::send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::AUTHORIZATION, common::basic_credential("dr"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(common::ips_canonical_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let vo = common::etag_uid(&headers)
        .split("::")
        .next()
        .expect("object id")
        .to_owned();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let stored: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit.audit_event WHERE resource_id LIKE $1 || '::%'",
        )
        .bind(&vo)
        .fetch_one(pool)
        .await
        .expect("read the trail");
        if stored > 0 {
            return vo;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no access record of the commit of {vo}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The `recorded` instants of the records naming `vo` in the trail of
/// [`PATIENT`] as `user` reads it.
async fn recorded(app: &Router, user: &str, vo: &str) -> Vec<String> {
    let req = Request::get(common::full(&format!(
        "/fhir/r4/AuditEvent?date=ge2020-01-01&patient={PATIENT}&_count=1000"
    )))
    .header(header::AUTHORIZATION, common::basic_credential(user))
    .body(Body::empty())
    .unwrap();
    let (status, body) = common::send_body(app, req).await;
    assert_eq!(status, StatusCode::OK, "ITI-81 read: {body}");
    let bundle: Value = serde_json::from_str(&body).expect("searchset");
    bundle["entry"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|entry| entry["resource"].to_string().contains(vo))
        .map(|entry| {
            entry["resource"]["recorded"]
                .as_str()
                .expect("recorded")
                .to_owned()
        })
        .collect()
}

/// A record about a held composition is absent from the person's trail until
/// a professional releases it, and then present with its original time; the
/// controller reads it throughout.
#[tokio::test]
async fn a_record_about_a_held_composition_appears_in_the_persons_trail_at_release() {
    let (_db, pool, app) = app().await;
    let ehr = ehr(&app).await;
    let vo = commit(&app, &pool, &ehr).await;

    let controller = recorded(&app, "root", &vo).await;
    assert!(!controller.is_empty(), "the controller reads the record");
    assert_eq!(
        recorded(&app, "portal", &vo).await,
        Vec::<String>::new(),
        "the person does not see a record about the held composition"
    );

    let (status, body) = send(
        &app,
        "POST",
        &format!("{EXT}/ehr/{ehr}/versioned_composition/{vo}/access-delay/release"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "release: {body}");

    let person = recorded(&app, "portal", &vo).await;
    for at in &controller {
        assert!(
            person.contains(at),
            "{at} with its original time: {person:?}"
        );
    }
}

/// A hold that passes its limit unreleased shows the person the records it
/// held back, each with its original time.
#[tokio::test]
async fn a_record_about_a_held_composition_appears_in_the_persons_trail_at_expiry() {
    let (_db, pool, app) = app().await;
    let ehr = ehr(&app).await;
    let vo = commit(&app, &pool, &ehr).await;
    let controller = recorded(&app, "root", &vo).await;
    assert!(!controller.is_empty(), "the controller reads the record");
    assert_eq!(
        recorded(&app, "portal", &vo).await,
        Vec::<String>::new(),
        "the person does not see a record about the held composition"
    );

    // The limit passing, without waiting 72 hours for it.
    sqlx::query(
        "UPDATE clinical.access_delay SET withheld_at = now() - interval '73 hours', \
         withheld_until = now() - interval '1 hour' WHERE vo_id = $1::uuid",
    )
    .bind(&vo)
    .execute(&pool)
    .await
    .expect("age the hold");

    let person = recorded(&app, "portal", &vo).await;
    for at in &controller {
        assert!(
            person.contains(at),
            "{at} with its original time: {person:?}"
        );
    }
}
