// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every route that names an EHR refuses a patient on another patient's EHR
//! the same way, and records it the same way (#5606).
//!
//! The route set is enumerated from the router itself
//! (`extensions_document`, the composition that mounts the routes): every
//! route whose path names `{ehr_id}`, driven by the person in the launch
//! context of their own EHR against another patient's EHR, answers `403`
//! with `refused-not-subject`, and its access-log entry names that code and
//! step 0 (subject). The routes that name the EHR outside their path (a
//! query's single-EHR scope, an import's `ehr_id`, a manifest export) and
//! those that name no EHR answer the same. The SMART layer checks the
//! grant's family and permission and the access evaluator alone holds the
//! caller to its own EHR: SMART App Launch requires the server to enforce
//! the patient context and names no layer, so that split is our own design.
//! The access log's own routes are left out, as their access control is
//! apart from any clinical decision (the access-log draft's recording rule
//! 8), and so are the controller's admin routes, which need the product
//! scope and which no directive binds.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::start;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::management::Observability;
use http::Request;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::access_evaluator_patient_http::{
    EHR_OTHER, EHR_OWN, manifest_export, not_subject, patient, rest_config, seed_ehr, status,
};
use crate::common;

/// The one value substituted for every path capture but `{ehr_id}`: the
/// evaluator refuses on the EHR before any object the path names is read.
const PROBE: &str = "0b8f7c5e-0000-4000-8000-000000000001";

/// The person's app over a fresh database, with the access log stored, and
/// both EHRs seeded.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle) = start(common::local_audit(), None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let seed = common::build_with(common::api_config(false), Arc::clone(&svc)).await;
    seed_ehr(&seed, EHR_OWN).await;
    seed_ehr(&seed, EHR_OTHER).await;
    let app = common::build_full(rest_config(), svc, None, Observability::default()).await;
    (db, pool, app)
}

/// Every `(method, path)` the router mounts whose path names `{ehr_id}`,
/// the access log's own routes and the controller's admin routes left out.
fn ehr_routes() -> Vec<(String, String)> {
    let doc = ferroehr_rest::extensions::openapi::extensions_document(&rest_config());
    let mut out = Vec::new();
    for (path, item) in &doc.paths.paths {
        let segments = || path.split('/');
        if !path.contains("{ehr_id}") || segments().any(|s| s == "access_log" || s == "admin") {
            continue;
        }
        for (method, operation) in [
            ("GET", &item.get),
            ("PUT", &item.put),
            ("POST", &item.post),
            ("DELETE", &item.delete),
            ("PATCH", &item.patch),
        ] {
            if operation.is_some() {
                out.push((method.to_owned(), path.clone()));
            }
        }
    }
    out
}

/// `template` with `{ehr_id}` as `ehr` and every other capture as [`PROBE`].
fn concrete(template: &str, ehr: &str) -> String {
    template
        .split('/')
        .map(|segment| match segment {
            "{ehr_id}" => ehr,
            s if s.starts_with('{') && s.ends_with('}') => PROBE,
            s => s,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A request as the person in the launch context of their own EHR.
fn as_patient(method: &str, uri: &str) -> http::request::Builder {
    Request::builder()
        .method(method)
        .uri(common::full(uri))
        .header("authorization", patient(Some(EHR_OWN)))
        .header("content-type", "application/json")
}

/// Sends `req`, the `n`th refused request of the test, asserts it answers
/// `403` with `refused-not-subject`, and that the `n`th refused access-log
/// entry records that code at step 0 (subject).
async fn assert_refused(app: &Router, pool: &PgPool, req: Request<Body>, n: usize, label: &str) {
    assert_eq!(status(app, req).await, not_subject(), "{label}");
    let refused = crate::access_log_http::entries(pool, n, |stored| {
        stored.result.as_deref() == Some("refused")
    })
    .await;
    assert_eq!(refused.len(), n, "{label}: one refused entry per request");
    let entry: Value = serde_json::from_str(&refused[n - 1].content).expect("a canonical entry");
    assert_eq!(
        entry["outcome"]["reason"]["defining_code"]["code_string"], "refused-not-subject",
        "{label}: {entry}"
    );
    let step = entry["outcome"]["description"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(step.starts_with("step 0 (subject): "), "{label}: {step}");
}

#[tokio::test]
async fn every_route_naming_another_patients_ehr_refuses_and_records_alike() {
    let (_db, pool, app) = app().await;
    let routes = ehr_routes();
    assert!(
        routes.iter().any(|(_, p)| p.ends_with("/directory"))
            && routes.iter().any(|(_, p)| p.ends_with("/tags"))
            && routes.iter().any(|(_, p)| p.contains("/message/export/"))
            && routes.iter().any(|(_, p)| p.contains("/message/import/")),
        "the walk covers the directory, tag, export and import routes: {routes:?}"
    );
    for (n, (method, template)) in routes.iter().enumerate() {
        let req = as_patient(method, &concrete(template, EHR_OTHER))
            .body(Body::from("{}"))
            .expect("request");
        assert_refused(&app, &pool, req, n + 1, &format!("{method} {template}")).await;
    }
}

#[tokio::test]
async fn an_ehr_named_outside_the_path_refuses_and_records_alike() {
    let (_db, pool, app) = app().await;
    let aql = json!({ "q": "SELECT e/ehr_id/value FROM EHR e" }).to_string();
    let query = format!("/query/aql?q=SELECT%20e/ehr_id/value%20FROM%20EHR%20e&ehr_id={EHR_OTHER}");
    let import = format!("/message/import?ehr_id={EHR_OTHER}");
    let named = [
        (
            "a query scoped by its ehr_id parameter",
            as_patient("GET", &query).body(Body::empty()),
        ),
        (
            "a query scoped by its openehr-ehr-id header",
            as_patient("POST", "/query/aql")
                .header("openehr-ehr-id", EHR_OTHER)
                .body(Body::from(aql)),
        ),
        (
            "an import naming the EHR by its ehr_id parameter",
            as_patient("POST", &import).body(Body::from("{}")),
        ),
        (
            "a manifest export naming another patient's EHR",
            Ok(manifest_export(&[EHR_OWN, EHR_OTHER], EHR_OWN)),
        ),
    ];
    for (n, (label, req)) in named.into_iter().enumerate() {
        assert_refused(&app, &pool, req.expect("request"), n + 1, label).await;
    }
}

/// An operation that reaches EHR data and names no EHR names none of the
/// caller's own: an EHR create, a lookup by subject, a query with no
/// single-EHR scope and an import that names no EHR.
#[tokio::test]
async fn an_operation_naming_no_ehr_refuses_and_records_alike() {
    let (_db, pool, app) = app().await;
    let aql = json!({ "q": "SELECT e/ehr_id/value FROM EHR e" }).to_string();
    let unnamed = [
        (
            "an EHR create",
            as_patient("POST", "/ehr").body(Body::from("{}")),
        ),
        (
            "a lookup by subject",
            as_patient(
                "GET",
                "/ehr?subject_id=PATIENT-2&subject_namespace=test.patients",
            )
            .body(Body::empty()),
        ),
        (
            "a query with no single-EHR scope",
            as_patient("POST", "/query/aql").body(Body::from(aql)),
        ),
        (
            "an import naming no EHR",
            as_patient("POST", "/message/import").body(Body::from("{}")),
        ),
    ];
    for (n, (label, req)) in unnamed.into_iter().enumerate() {
        assert_refused(&app, &pool, req.expect("request"), n + 1, label).await;
    }
}
