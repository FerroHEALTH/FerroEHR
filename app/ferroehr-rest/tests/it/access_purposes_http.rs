// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET {extensions}/access/purposes` (#6105): the purposes of use a caller
//! may declare. The list names no personal data, so any authenticated
//! caller reads it, one holding no role included, and an unauthenticated one
//! is refused `401`. No openEHR spec governs the purpose of use: our own
//! design.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::common;
use crate::common::EXT;

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app accepting `TREAT` and `ETREAT`, with RBAC on and a Basic
/// user holding no role beside one holding `USER`.
async fn app() -> (testkit::TestDb, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(common::binding_service(&pool).with_audit(sender));
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[("nobody", &[]), ("clinician", &["USER"])],
    )
    .await;
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
    let app = common::build_full(config, svc, authz, Observability::default()).await;
    (db, app)
}

async fn purposes(app: &Router, credential: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::get(format!("{EXT}/access/purposes"));
    if let Some(credential) = credential {
        request = request.header(header::AUTHORIZATION, credential);
    }
    common::send_body(app, request.body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn a_caller_with_no_role_reads_the_accepted_purposes() {
    let (_db, app) = app().await;
    for user in ["nobody", "clinician"] {
        let (status, body) = purposes(&app, Some(&basic(user))).await;
        assert_eq!(status, StatusCode::OK, "{user}: {body}");
        let read: Value = serde_json::from_str(&body).expect("a JSON body");
        assert_eq!(
            read,
            json!({
                "header": "x-purpose-of-use",
                "reason_header": "x-access-reason",
                "system": "http://terminology.hl7.org/CodeSystem/v3-ActReason",
                "codes": [{ "code": "TREAT" }, { "code": "ETREAT" }]
            }),
            "{user}: with no terminology server a code carries no display text"
        );
    }
}

#[tokio::test]
async fn an_unauthenticated_caller_is_refused() {
    let (_db, app) = app().await;
    let (status, body) = purposes(&app, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}
