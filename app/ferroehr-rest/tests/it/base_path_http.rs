// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The whole served surface at a NON-default `server.base_path`.
//!
//! ITS-REST locates the API at a deployment-chosen base followed by the
//! version segment (`docs/specs/openehr/ITS-REST/specifications/docs/overview/
//! Resources.md` §Resource identification), so a deployment may shorten the
//! base path — for instance behind a path-prefixed reverse proxy, where the
//! proxy's own prefix stacks on top of it. This module drives the assembled
//! router at `/ferroehr/v1` and asserts that everything moved with it: the API
//! nest, the `Location` header a commit returns, the System Options manifest,
//! the product-root status document, and the served `OpenAPI` document, whose
//! every declared path must be one this deployment actually serves.
//!
//! No openEHR spec governs where the non-API surfaces root — our own
//! design/extension.

use axum::Router;
use ferroehr::config::FerroEhrConfig;
use ferroehr_rest::config::AppConfig;
use http::{StatusCode, header};
use serde_json::Value;

use crate::common;

/// The shortened base path this module runs the whole server at.
const SHORT_BASE: &str = "/ferroehr/v1";

/// The product root that base path derives, where `/status`, the `OpenAPI`
/// documents and the SMART discovery document hang.
const SHORT_ROOT: &str = "/ferroehr";

/// The assembled router at [`SHORT_BASE`], with the Swagger surface on so the
/// served documents are reachable.
async fn short_base_router() -> (testkit::TestDb, Router) {
    // `api_config` supplies the unauthenticated baseline; only the base path
    // and the Swagger surface differ from it here.
    let config = AppConfig {
        server: ferroehr::config::server::ServerConfig {
            bind: "127.0.0.1:0".to_owned(),
            base_path: SHORT_BASE.to_owned(),
            max_in_flight: 1024,
            swagger_ui: ferroehr::config::management::AccessLevel::Public,
            cors_permissive: false,
            ..Default::default()
        },
        ..common::api_config(false)
    };
    let (db, service) = common::test_service().await;
    (db, common::router_with(config, service))
}

/// The shortened base path is a value boot validation ACCEPTS — the wire
/// behaviour below only matters for a configuration a server would start on.
#[test]
fn the_shortened_base_path_passes_boot_validation() {
    let mut cfg = FerroEhrConfig::default();
    cfg.server.base_path = SHORT_BASE.to_owned();
    assert!(cfg.validate().is_ok());
    assert_eq!(cfg.server.rest_root(), SHORT_ROOT);
}

/// The API answers under the configured base path, the `Location` it returns
/// points back into it, and the default path is gone.
#[tokio::test]
async fn the_api_moves_with_the_base_path() {
    let (_pg, app) = short_base_router().await;

    let (status, headers, _body) =
        common::send(&app, common::request("POST", &format!("{SHORT_BASE}/ehr"))).await;
    assert_eq!(status, StatusCode::CREATED);
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location on a created EHR");
    assert!(
        location.starts_with(&format!("{SHORT_BASE}/ehr/")),
        "Location {location} must point into the configured base path"
    );

    // The created EHR is readable at the Location the server just handed out.
    assert_eq!(
        common::send_status(&app, common::get(location)).await,
        StatusCode::OK
    );

    // Nothing answers at the default base path any more.
    assert_eq!(
        common::send_status(&app, common::get(&format!("{}/ehr", common::BASE))).await,
        StatusCode::NOT_FOUND
    );
}

/// The System Options manifest stays at the API base-path root itself
/// (ITS-REST System API), and the product-root status document follows the
/// derived REST root rather than the default `/ferroehr/rest`.
#[tokio::test]
async fn the_root_surfaces_follow_the_derived_rest_root() {
    let (_pg, app) = short_base_router().await;

    let (status, _headers, body) = common::send(&app, common::request("OPTIONS", SHORT_BASE)).await;
    assert_eq!(status, StatusCode::OK);
    let manifest: Value = serde_json::from_str(&body).expect("Options manifest is JSON");
    assert_eq!(
        manifest.get("solution").and_then(Value::as_str),
        Some("FerroEHR")
    );

    assert_eq!(
        common::send_status(&app, common::get(&format!("{SHORT_ROOT}/status"))).await,
        StatusCode::OK
    );
    assert_eq!(
        common::send_status(&app, common::get("/ferroehr/rest/status")).await,
        StatusCode::NOT_FOUND
    );
    // The health family is mounted at the process root and never moves.
    assert_eq!(
        common::send_status(&app, common::get("/health")).await,
        StatusCode::OK
    );
}

/// Every path in the served document is one this deployment actually mounts:
/// the API under the configured base path, the operational surfaces under the
/// derived product root, the health family at the process root.
#[tokio::test]
async fn the_served_openapi_declares_only_paths_this_deployment_serves() {
    let (_pg, app) = short_base_router().await;

    let (status, _headers, body) = common::send(
        &app,
        common::get(&format!("{SHORT_ROOT}/api-docs/openapi.json")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let doc: Value = serde_json::from_str(&body).expect("served openapi json");
    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .expect("the served document declares paths");
    assert!(!paths.is_empty());

    let allowed = [
        SHORT_BASE.to_owned(),
        format!("{SHORT_ROOT}/status"),
        format!("{SHORT_ROOT}/api-docs"),
        format!("{SHORT_ROOT}/swagger-ui"),
        format!("{SHORT_ROOT}/.well-known"),
        "/health".to_owned(),
        "/management".to_owned(),
    ];
    for path in paths.keys() {
        assert!(
            allowed.iter().any(|prefix| path.starts_with(prefix)),
            "served path {path} is outside this deployment's mounted surfaces"
        );
        assert!(
            !path.starts_with(common::BASE),
            "served path {path} still spells the default base path"
        );
    }

    // The Swagger UI moved with the root too.
    assert_eq!(
        common::send_status(&app, common::get(&format!("{SHORT_ROOT}/swagger-ui"))).await,
        StatusCode::TEMPORARY_REDIRECT
    );
}
