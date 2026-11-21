// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request body sent in `Content-Encoding: gzip` or `br` is decoded before
//! any route reads it (RFC 9110 §8.4, `[server] request_decompression`), so
//! a compressed IPS commit stores the same composition as the uncompressed
//! one. Another coding is refused `415` with `Accept-Encoding` (RFC 9110
//! §15.5.16, RFC 7694 §3), and `api.body_limit` holds the decoded size
//! (RFC 9110 §15.5.14). Decoding by default is our own design (#6208).

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use ferrohealth_config::units::ByteSize;
use http::{StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::Value;
use tower::ServiceExt as _;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::SizeAbove;

use ferroehr_rest::config::AppConfig;

use crate::common::{self, BASE, etag_uid, ips_canonical_composition, ips_opt_xml, send};

/// The clinical body limit of these cases: twice the IPS composition.
const API_LIMIT_KIB: u64 = 256;

/// The router with request decoding on, as FerroEHR's defaults set it, and
/// a clinical body limit, with an EHR and the IPS template uploaded.
async fn decoding_app() -> (testkit::TestDb, Router, String) {
    let (db, service) = common::test_service().await;
    let mut config = AppConfig {
        server: ferrohealth_server::section::ServerSection {
            request_decompression: true,
            ..common::server()
        },
        ..AppConfig::default()
    };
    config.auth.shared.enabled = false;
    config.api.body_limit = Some(ByteSize::from_kib(API_LIMIT_KIB));
    let app = common::router_with(config, Arc::clone(&service)).await;
    let (status, headers, body) =
        send(&app, common::req("POST", format!("{BASE}/ehr")).build()).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = etag_uid(&headers);
    let (status, _, body) = send(
        &app,
        common::req("POST", format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(ips_opt_xml())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (db, app, ehr)
}

/// `plain` in `coding`, as tower-http's response compression writes it.
async fn encoded(coding: &str, plain: Vec<u8>) -> Bytes {
    let plain = Bytes::from(plain);
    let compressing = tower::ServiceBuilder::new()
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(0)))
        .service_fn(move |_request: http::Request<Body>| {
            let plain = plain.clone();
            async move {
                Ok::<_, Infallible>(
                    http::Response::builder()
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(plain))
                        .expect("the response builds"),
                )
            }
        });
    let response = compressing
        .oneshot(
            http::Request::builder()
                .header(header::ACCEPT_ENCODING, coding)
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("compressed");
    assert_eq!(
        Some(coding),
        response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
    );
    response
        .into_body()
        .collect()
        .await
        .expect("the compressed body")
        .to_bytes()
}

/// Commits `body` to `ehr` with `coding`, and returns the answer.
async fn commit(
    app: &Router,
    ehr: &str,
    coding: Option<&str>,
    body: impl Into<Body>,
) -> (StatusCode, header::HeaderMap, String) {
    let mut request = common::req("POST", format!("{BASE}/ehr/{ehr}/composition"));
    if let Some(coding) = coding {
        request = request.header(header::CONTENT_ENCODING, coding);
    }
    send(app, request.json(body).build()).await
}

/// The stored composition of the version `version`, without its `uid`, the
/// one member each commit assigns.
async fn stored(app: &Router, ehr: &str, version: &str) -> Value {
    let (status, _, body) = send(
        app,
        common::req("GET", format!("{BASE}/ehr/{ehr}/composition/{version}"))
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read the stored version: {body}");
    let mut composition: Value = serde_json::from_str(&body).expect("a composition");
    composition
        .as_object_mut()
        .expect("a composition object")
        .remove("uid");
    composition
}

#[tokio::test]
async fn an_ips_commit_in_gzip_or_brotli_stores_the_uncompressed_composition() {
    let (_db, app, ehr) = Box::pin(decoding_app()).await;
    let plain = ips_canonical_composition().to_string().into_bytes();
    let (status, headers, body) = commit(&app, &ehr, None, plain.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "uncompressed commit: {body}");
    let uncompressed = stored(&app, &ehr, &etag_uid(&headers)).await;

    for coding in ["gzip", "br"] {
        let compressed = encoded(coding, plain.clone()).await;
        assert!(
            compressed.len() * 4 < plain.len(),
            "{coding}: {} of {}",
            compressed.len(),
            plain.len()
        );
        let (status, headers, body) = commit(&app, &ehr, Some(coding), compressed).await;
        assert_eq!(status, StatusCode::CREATED, "{coding} commit: {body}");
        assert_eq!(
            uncompressed,
            stored(&app, &ehr, &etag_uid(&headers)).await,
            "{coding}"
        );
    }
}

#[tokio::test]
async fn a_body_that_decodes_past_the_api_body_limit_is_refused_413() {
    let (_db, app, ehr) = Box::pin(decoding_app()).await;
    let limit = usize::try_from(API_LIMIT_KIB * 1024).expect("the limit fits");
    let mut padded = ips_canonical_composition().to_string().into_bytes();
    padded.resize(limit * 4, b' ');
    let bomb = encoded("gzip", padded).await;
    assert!(
        bomb.len() < limit,
        "the wire body is under the limit: {}",
        bomb.len()
    );
    let (status, _, body) = commit(&app, &ehr, Some("gzip"), bomb).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
}

#[tokio::test]
async fn an_unknown_coding_is_refused_415_with_accept_encoding() {
    let (_db, app, ehr) = Box::pin(decoding_app()).await;
    let plain = ips_canonical_composition().to_string();
    for coding in ["zstd", "deflate", "gzip, br"] {
        let (status, headers, body) = commit(&app, &ehr, Some(coding), plain.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{coding}: {body}"
        );
        assert_eq!(
            headers
                .get(header::ACCEPT_ENCODING)
                .and_then(|value| value.to_str().ok()),
            Some("gzip, br"),
            "{coding}"
        );
    }
}
