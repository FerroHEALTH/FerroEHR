// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FerroEHR behind FerroSMART: FerroSMART-shaped tokens read from their
//! standard claims, over the real axum app.
//!
//! One `[auth.issuer.<name>]` table names the FerroSMART issuer alone. A token
//! whose `aud` names this resource server admits its client, since the
//! authorization server's registry is the one list of clients (RFC 9068 §4),
//! and `scope` is required (SMART App Launch 2.2.0, Obtain access token,
//! Response). A Backend Services token, whose `sub` is its `client_id`
//! (RFC 9068 §2.2), reads as the system.
//!
//! The EHR the requests name does not exist, so a request the gate lets
//! through answers `404` from the service; a gate refusal answers `401`.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferrohealth_smart::config::IssuerEntry;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};

const ISSUER: &str = "https://smart.example";
const AUDIENCE: &str = "ferroehr";
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

/// A SMART app registered at FerroSMART.
const APP: &str = "growth-chart";

/// The app trusting FerroSMART by its issuer alone.
async fn app() -> (testkit::TestDb, Router) {
    let (db, pool) = common::migrated_pool().await;
    let mut config = common::api_config(false);
    config.auth = common::trusting(vec![IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(ISSUER, AUDIENCE)
    }]);
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let router = common::router_with(config, Arc::new(service)).await;
    (db, router)
}

/// A FerroSMART token for `audience` carrying `extra` on top of the claims
/// every token has.
fn token_for(audience: &str, extra: &Value) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).unwrap() + 3600;
    let mut claims = json!({ "iss": ISSUER, "aud": audience, "exp": exp });
    for (key, value) in extra.as_object().expect("object") {
        claims[key] = value.clone();
    }
    common::bearer(&claims)
}

/// A FerroSMART token for this resource server carrying `extra`.
fn token(extra: &Value) -> String {
    token_for(AUDIENCE, extra)
}

fn ehr_path() -> String {
    format!("{BASE}/ehr/{EHR}")
}

#[tokio::test]
async fn a_person_through_an_app_for_this_audience_is_read_and_another_audience_is_refused() {
    let (_db, app) = app().await;
    let through = token(&json!({ "sub": "pairwise-1", "client_id": APP }));
    let status = common::send_status(&app, common::get_authorized(&ehr_path(), &through)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a person through an app whose token names this server reaches the service"
    );

    let elsewhere = token_for(
        "https://other.example",
        &json!({ "sub": "pairwise-1", "client_id": APP }),
    );
    let (status, _headers, body) =
        common::send(&app, common::get_authorized(&ehr_path(), &elsewhere)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a token for another resource server is no caller: {body}"
    );
}

#[tokio::test]
async fn a_backend_services_client_reads_as_the_system() {
    let (_db, app) = app().await;
    let backend = token(&json!({ "sub": "lab-backend", "client_id": "lab-backend" }));
    let (status, _headers, body) =
        common::send(&app, common::get_authorized(&ehr_path(), &backend)).await;
    assert_ne!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_token_without_its_scope_is_refused() {
    let (_db, app) = app().await;
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).unwrap() + 3600;
    let claims = json!({
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "sub": "lab-backend",
        "client_id": "lab-backend",
    });
    let without = common::bearer_without(&claims, "scope");
    let (status, _headers, body) =
        common::send(&app, common::get_authorized(&ehr_path(), &without)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a SMART token requires `scope`: {body}"
    );
}
