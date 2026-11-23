// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART `composition-` scope of a COMPOSITION delete (master08 §Resource
//! Scopes), judged on the template of the version the delete addresses, read by
//! the delete's gate read (#6271): the scope's `403` comes before the `400`,
//! `404`, `409` and `412` of the delete itself.

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::json;

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;

use crate::common;
use crate::smart_http::{config, issuer, token};

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A grant that covers the IPS template ("International Patient Summary").
const COVERED: &str = "user/composition-International*.cruds";

/// A grant that covers another template only.
const UNCOVERED: &str = "user/composition-another.template.v1.cruds";

/// The IPS COMPOSITION in two versions in one EHR and one version in a
/// second EHR: `(first EHR, its v1 uid, its v2 uid, second EHR, the other
/// EHR's v1 uid)`.
struct Seeded {
    ehr: String,
    v1: String,
    v2: String,
    other_ehr: String,
    other: String,
}

/// Seeds [`Seeded`] through the service, which no credential gates.
async fn seed(db: &testkit::TestDb) -> Seeded {
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    svc.template_adl14_upload(&common::ips_opt_xml())
        .await
        .expect("the IPS template uploads");
    let composition = || {
        openehr_its::json::from_canonical_value(&common::ips_canonical_composition())
            .expect("the IPS composition decodes")
    };
    let mut ehrs = Vec::new();
    for _ in 0..2 {
        let ehr = EhrId(uuid::Uuid::now_v7());
        Box::pin(svc.create_ehr_with_id_meta(ehr, WriteRequest::new(None)))
            .await
            .expect("the EHR is created");
        ehrs.push(ehr);
    }
    let first = Box::pin(svc.create_composition(ehrs[0], WriteRequest::new(composition())))
        .await
        .expect("v1 commits");
    let mut update = WriteRequest::new(composition());
    update.preceding = Some(first.version_uid().parse().expect("an OBJECT_VERSION_ID"));
    let second = Box::pin(svc.update_composition(ehrs[0], first.vo_id, update))
        .await
        .expect("v2 commits");
    let other = Box::pin(svc.create_composition(ehrs[1], WriteRequest::new(composition())))
        .await
        .expect("the other EHR's v1 commits");
    Seeded {
        ehr: ehrs[0].to_string(),
        v1: first.version_uid(),
        v2: second.version_uid(),
        other_ehr: ehrs[1].to_string(),
        other: other.version_uid(),
    }
}

/// `DELETE` of `uid` in `ehr` under `scope`, with `if_match` volunteered:
/// the status and whether the answer carries the `insufficient_scope`
/// challenge of a scope refusal.
async fn delete(
    app: &Router,
    issuer: &str,
    scope: &str,
    path: &str,
    if_match: Option<&str>,
) -> (StatusCode, bool, String) {
    let mut builder = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}{path}"))
        .header(header::AUTHORIZATION, token(issuer, scope, &json!({})));
    if let Some(tag) = if_match {
        builder = builder.header(header::IF_MATCH, format!("\"{tag}\""));
    }
    let (status, headers, body) = common::send(app, builder.body(Body::empty()).unwrap()).await;
    let challenged = headers
        .get(header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains(r#"error="insufficient_scope""#));
    (status, challenged, body)
}

/// A delete addressing a version that is not the tip answers `409` under a
/// grant covering its template and the scope's `403` under one that does
/// not, as when the pre-check read that version's template.
#[tokio::test]
async fn a_delete_of_a_version_behind_the_tip_answers_as_before() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (db, app) = common::app(config(&uri, false, false)).await;
    let s = seed(&db).await;
    let path = format!("/ehr/{}/composition/{}", s.ehr, s.v1);
    let (status, challenged, body) = delete(&app, &uri, COVERED, &path, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!challenged, "{body}");
    let (status, challenged, body) = delete(&app, &uri, UNCOVERED, &path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(challenged, "{body}");
}

/// A delete addressing another EHR's version answers `404` under a grant
/// covering that version's template, which the pre-check read in whichever
/// EHR it lived, and the scope's `403` under one that does not.
#[tokio::test]
async fn a_delete_of_another_ehrs_version_answers_as_before() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (db, app) = common::app(config(&uri, false, false)).await;
    let s = seed(&db).await;
    let path = format!("/ehr/{}/composition/{}", s.ehr, s.other);
    let (status, challenged, body) = delete(&app, &uri, COVERED, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!challenged, "{body}");
    let (status, challenged, body) = delete(&app, &uri, UNCOVERED, &path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(challenged, "{body}");
    let (status, _, body) = delete(
        &app,
        &uri,
        COVERED,
        &format!("/ehr/{}/composition/{}", s.other_ehr, s.other),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// A version the store does not hold names no template, which only a grant
/// on every template covers: the scope's `403` under a grant on one template,
/// and the delete's own `409` under `*`.
#[tokio::test]
async fn a_delete_of_a_version_the_store_lacks_answers_as_before() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (db, app) = common::app(config(&uri, false, false)).await;
    let s = seed(&db).await;
    let absent = s.v2.replace("::2", "::9");
    let path = format!("/ehr/{}/composition/{absent}", s.ehr);
    let (status, challenged, body) = delete(&app, &uri, COVERED, &path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(challenged, "{body}");
    let (status, challenged, body) =
        delete(&app, &uri, "user/composition-*.cruds", &path, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!challenged, "{body}");
}

/// A stale `If-Match` answers `412` under a covering grant, and a malformed
/// request, refused before its gate read, the scope's `403` under a grant
/// that does not cover the template its uid names and its `400` otherwise.
#[tokio::test]
async fn a_stale_condition_and_a_malformed_delete_answer_as_before() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (db, app) = common::app(config(&uri, false, false)).await;
    let s = seed(&db).await;
    let tip = format!("/ehr/{}/composition/{}", s.ehr, s.v2);
    let (status, challenged, body) = delete(&app, &uri, COVERED, &tip, Some(&s.v1)).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert!(!challenged, "{body}");
    let (status, challenged, body) = delete(&app, &uri, UNCOVERED, &tip, Some(&s.v1)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(challenged, "{body}");
    let object = s.v2.split("::").next().expect("an object id");
    let bare = format!("/ehr/{}/composition/{object}", s.ehr);
    let (status, challenged, body) = delete(&app, &uri, COVERED, &bare, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!challenged, "{body}");
    let (status, challenged, body) = delete(&app, &uri, UNCOVERED, &bare, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(challenged, "{body}");
}
