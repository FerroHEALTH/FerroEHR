// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every route that names an EHR holds a representative to the EHR of the
//! subject it represents, and records a refusal the same way (#5609).
//!
//! The route set is the one `access_evaluator_patient_routes_http` walks,
//! enumerated from the router itself: every route whose path names
//! `{ehr_id}`, driven by a representative whose recorded representation
//! `PARTY_RELATIONSHIP` names the subject of one EHR, against another EHR,
//! answers `403` with `refused-not-represented`, and its access-log entry
//! names that code and step 0 (representation). On the represented
//! subject's EHR no route refuses it. The routes that name the EHR outside
//! their path and those that name no EHR answer the same. The SMART layer
//! checks the grant's family and permission and the access evaluator alone
//! holds the representative to that EHR (EHDS Art. 4(2) names the
//! representative's access and no layer): our own design.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::start;
use ferroehr::system_log::store::AuditStore;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::access_evaluator_patient_http::{Answer, status};
use crate::access_evaluator_patient_routes_http::{concrete, ehr_routes};
use crate::common;
use crate::representation_http::{CHILD, Recorded, STRANGER, config, ehr, representative, router};

/// What the walk drives: the app, its database, the represented subject's
/// EHR and another subject's.
struct Walk {
    _db: testkit::TestDb,
    pool: PgPool,
    app: Router,
    child: String,
    stranger: String,
}

/// The representative's app over a fresh database, with the access log
/// stored, the subject it represents and a stranger each given an EHR.
async fn walk() -> Walk {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle) = start(common::local_audit(), None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_binding_mapping(crate::party_binding_http::mapping())
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let child = ehr(&svc, Some(CHILD)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let source = Arc::new(Recorded {
        subjects: vec![CHILD],
    });
    let app = router(svc, source).await;
    Walk {
        _db: db,
        pool,
        app,
        child,
        stranger,
    }
}

/// A request as the representative of [`CHILD`].
fn as_representative(method: &str, uri: &str) -> http::request::Builder {
    Request::builder()
        .method(method)
        .uri(common::full(uri))
        .header("authorization", representative(CHILD))
        .header("content-type", "application/json")
}

/// The answer to a representative on an EHR other than the represented
/// subject's: `403` with `refused-not-represented`.
fn not_represented() -> Answer {
    Answer {
        status: StatusCode::FORBIDDEN,
        refusal: json!("refused-not-represented"),
    }
}

/// Sends `req`, the `n`th refused request of the test, asserts it answers
/// `403` with `refused-not-represented`, and that the `n`th refused
/// access-log entry records that code at step 0 (representation).
async fn assert_refused(walk: &Walk, req: Request<Body>, n: usize, label: &str) {
    assert_eq!(status(&walk.app, req).await, not_represented(), "{label}");
    let refused = crate::access_log_http::entries(&walk.pool, n, |stored| {
        stored.result.as_deref() == Some("refused")
    })
    .await;
    assert_eq!(refused.len(), n, "{label}: one refused entry per request");
    let entry: Value = serde_json::from_str(&refused[n - 1].content).expect("a canonical entry");
    assert_eq!(
        entry["outcome"]["reason"]["defining_code"]["code_string"], "refused-not-represented",
        "{label}: {entry}"
    );
    let step = entry["outcome"]["description"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(
        step.starts_with("step 0 (representation): "),
        "{label}: {step}"
    );
}

#[tokio::test]
async fn every_route_naming_another_subjects_ehr_refuses_and_records_alike() {
    let walk = walk().await;
    let routes = ehr_routes(&config());
    assert!(
        routes.iter().any(|(_, p)| p.ends_with("/directory"))
            && routes.iter().any(|(_, p)| p.ends_with("/tags"))
            && routes.iter().any(|(_, p)| p.contains("/message/export/"))
            && routes.iter().any(|(_, p)| p.contains("/message/import/")),
        "the walk covers the directory, tag, export and import routes: {routes:?}"
    );
    for (n, (method, template)) in routes.iter().enumerate() {
        let req = as_representative(method, &concrete(template, &walk.stranger))
            .body(Body::from("{}"))
            .expect("request");
        assert_refused(&walk, req, n + 1, &format!("{method} {template}")).await;
    }
}

/// On the represented subject's EHR the evaluator's steps admit the
/// representative on every route: whatever a route answers, it is no refusal.
/// The one `403` is the access-delay release, which its handler refuses to
/// anyone acting for the person (EHDS Art. 3(3)), with no refusal code.
#[tokio::test]
async fn no_route_naming_the_represented_subjects_ehr_refuses() {
    let walk = walk().await;
    for (method, template) in ehr_routes(&config()) {
        let req = as_representative(&method, &concrete(&template, &walk.child))
            .body(Body::from("{}"))
            .expect("request");
        let answer = status(&walk.app, req).await;
        assert_eq!(answer.refusal, Value::Null, "{method} {template}");
        let released = template.ends_with("/access-delay/release");
        assert_eq!(
            answer.status == StatusCode::FORBIDDEN,
            released,
            "{method} {template}: {answer:?}"
        );
    }
}

#[tokio::test]
async fn an_ehr_named_outside_the_path_refuses_and_records_alike() {
    let walk = walk().await;
    let stranger = &walk.stranger;
    let aql = json!({ "q": "SELECT e/ehr_id/value FROM EHR e" }).to_string();
    let query = format!("/query/aql?q=SELECT%20e/ehr_id/value%20FROM%20EHR%20e&ehr_id={stranger}");
    let import = format!("/message/import?ehr_id={stranger}");
    let named = [
        (
            "a query scoped by its ehr_id parameter",
            as_representative("GET", &query).body(Body::empty()),
        ),
        (
            "a query scoped by its openehr-ehr-id header",
            as_representative("POST", "/query/aql")
                .header("openehr-ehr-id", stranger)
                .body(Body::from(aql)),
        ),
        (
            "an import naming the EHR by its ehr_id parameter",
            as_representative("POST", &import).body(Body::from("{}")),
        ),
        (
            "a manifest export naming another subject's EHR",
            as_representative("POST", "/message/export")
                .body(Body::from(manifest(&[&walk.child, stranger]).to_string())),
        ),
    ];
    for (n, (label, req)) in named.into_iter().enumerate() {
        assert_refused(&walk, req.expect("request"), n + 1, label).await;
    }
}

/// An operation that reaches EHR data and names no EHR names none the
/// representative represents: an EHR create, a lookup by subject, a query
/// with no single-EHR scope and an import that names no EHR.
#[tokio::test]
async fn an_operation_naming_no_ehr_refuses_and_records_alike() {
    let walk = walk().await;
    let aql = json!({ "q": "SELECT e/ehr_id/value FROM EHR e" }).to_string();
    let lookup = format!("/ehr?subject_id={CHILD}&subject_namespace=demographic");
    let unnamed = [
        (
            "an EHR create",
            as_representative("POST", "/ehr").body(Body::from("{}")),
        ),
        (
            "a lookup by subject",
            as_representative("GET", &lookup).body(Body::empty()),
        ),
        (
            "a query with no single-EHR scope",
            as_representative("POST", "/query/aql").body(Body::from(aql)),
        ),
        (
            "an import naming no EHR",
            as_representative("POST", "/message/import").body(Body::from("{}")),
        ),
    ];
    for (n, (label, req)) in unnamed.into_iter().enumerate() {
        assert_refused(&walk, req.expect("request"), n + 1, label).await;
    }
}

/// A `POST /message/export` manifest naming `ehr_ids`.
fn manifest(ehr_ids: &[&str]) -> Value {
    let entities: Vec<Value> = ehr_ids
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
}
