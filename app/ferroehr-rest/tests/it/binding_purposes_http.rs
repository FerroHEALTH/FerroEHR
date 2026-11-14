// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The purpose of use and the professional role of an access token at the
//! assembled router (#5323), over a real `FerroEhrService` on a real
//! `PostgreSQL` 18 with the local party source and the audit store.
//!
//! An issuer's `[[auth.attributes]]` table with `mapping = "iua"` reads the IHE
//! IUA `purpose_of_use` and `subject_role` claims (IHE ITI TF-2
//! §3.71.4.2.2.1.1). The access record carries them; the `[audit]
//! purpose_header` keeps working alone; and a header naming a purpose the
//! token does not state refuses the request. The refusal rule is our own
//! design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferrohealth_smart::federation::Mapping;
use ferrohealth_smart::federation::person::AttributesSection;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{
    ISSUER, audited_service, config, professional, router_with, token,
};

/// The `[audit] purpose_header` of the default audit configuration.
const PURPOSE_HEADER: &str = "x-purpose-of-use";
/// The HL7 v3 `ActReason` code system.
const ACT_REASON: &str = "http://terminology.hl7.org/CodeSystem/v3-ActReason";
/// The SNOMED CT concept the professional's role is coded by (medical
/// practitioner).
const PRACTITIONER: &str = "158965000"; // privacy-allow: a SNOMED CT concept id, not a BSN
/// The login of the professional the cases bind.
const KEY: &str = "5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b";

/// The router over `svc`, the issuer reading its tokens' attributes by the
/// IUA mapping.
async fn router(svc: Arc<FerroEhrService>) -> Router {
    let mut config = config(None);
    config.auth.shared.attributes = vec![AttributesSection {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
        mapping: Some(Mapping::Iua),
        ..AttributesSection::default()
    }];
    let source = Arc::new(svc.party_source("demographic"));
    router_with(svc, config, source).await
}

/// The professional's token, its IUA extension stating `purposes`.
fn stating(purposes: &[&str]) -> String {
    let purposes: Vec<Value> = purposes
        .iter()
        .map(|code| json!({ "system": ACT_REASON, "code": code }))
        .collect();
    token(
        KEY,
        &json!({ "extensions": { "ihe_iua": {
            "purpose_of_use": purposes,
            "subject_role": [{ "system": "urn:oid:2.16.840.1.113883.6.96", "code": PRACTITIONER }]
        } } }),
    )
}

/// The status of a template listing as `credential`, declaring `purpose`.
async fn list(app: &Router, credential: &str, purpose: Option<&str>) -> StatusCode {
    let mut request = common::req("GET", format!("{BASE}/definition/template/adl1.4"))
        .auth(credential)
        .accept("application/json");
    if let Some(purpose) = purpose {
        request = request.header(PURPOSE_HEADER, purpose);
    }
    common::send_body(app, request.build()).await.0
}

/// The `(outcome, purpose, token_attributes)` of the stored records of `KEY`,
/// waiting for `count`.
async fn records(pool: &sqlx::PgPool, count: usize) -> Vec<(i16, Option<String>, Option<Value>)> {
    for _ in 0..50 {
        let rows: Vec<(i16, Option<String>, Option<Value>)> = sqlx::query_as(
            "SELECT outcome, purpose, token_attributes FROM audit.access_trail \
             WHERE principal = $1 ORDER BY chain_seq",
        )
        .bind(KEY)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} records of the professional were stored");
}

/// The `token_attributes` a token stating `purposes` is recorded with.
fn recorded(purposes: &[&str]) -> Value {
    json!({
        "purposes": purposes.iter().map(|code| json!({ "system": ACT_REASON, "code": code })).collect::<Vec<_>>(),
        "roles": [{ "system": "urn:oid:2.16.840.1.113883.6.96", "code": PRACTITIONER }]
    })
}

/// A token stating its purpose alone: the record carries the token's purpose
/// and role, and no declared purpose.
#[tokio::test]
async fn a_token_purpose_alone_is_recorded() {
    let (db, _audit, svc) = audited_service().await;
    professional(&svc, KEY, "532").await;
    let app = router(svc).await;
    assert_eq!(list(&app, &stating(&["TREAT"]), None).await, StatusCode::OK);
    let rows = records(&db.pool(), 1).await;
    assert_eq!(rows, vec![(0, None, Some(recorded(&["TREAT"])))]);
}

/// A token of the issuer stating no attributes, with the header: the header
/// keeps working as before, and the record carries no token attributes.
#[tokio::test]
async fn a_header_purpose_alone_keeps_working() {
    let (db, _audit, svc) = audited_service().await;
    professional(&svc, KEY, "532").await;
    let app = router(svc).await;
    assert_eq!(
        list(&app, &token(KEY, &json!({})), Some("TREAT")).await,
        StatusCode::OK
    );
    let rows = records(&db.pool(), 1).await;
    assert_eq!(rows, vec![(0, Some("TREAT".to_owned()), None)]);
}

/// A header naming one of the token's purposes is admitted and recorded with
/// both.
#[tokio::test]
async fn a_header_purpose_the_token_states_is_admitted() {
    let (db, _audit, svc) = audited_service().await;
    professional(&svc, KEY, "532").await;
    let app = router(svc).await;
    assert_eq!(
        list(&app, &stating(&["ETREAT", "TREAT"]), Some("TREAT")).await,
        StatusCode::OK
    );
    let rows = records(&db.pool(), 1).await;
    assert_eq!(
        rows,
        vec![(
            0,
            Some("TREAT".to_owned()),
            Some(recorded(&["ETREAT", "TREAT"]))
        )]
    );
}

/// A header naming a purpose the token does not state refuses the request
/// with `403`, and the refusal is recorded naming the caller and the token's
/// purposes.
#[tokio::test]
async fn a_header_purpose_the_token_contradicts_is_refused_and_recorded() {
    let (db, _audit, svc) = audited_service().await;
    professional(&svc, KEY, "532").await;
    let app = router(svc).await;
    assert_eq!(
        list(&app, &stating(&["TREAT"]), Some("ETREAT")).await,
        StatusCode::FORBIDDEN
    );
    let rows = records(&db.pool(), 1).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (outcome, _, attributes) = &rows[0];
    assert_ne!(*outcome, 0, "the record is a failure");
    assert_eq!(attributes.as_ref(), Some(&recorded(&["TREAT"])));
}
