// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The patient-data authentication gate over the real axum app: the
//! `[auth.oidc.assurance]` step-up refusal (RFC 9470 §3), the
//! `[auth.oidc.professional]` natural-person refusal, and the access record
//! carrying the acting mode, the assurance level and the professional (EHDS
//! Annex II 3.1 and 3.2(b), `docs/law/eu/ehds/text.html Annex II 3`).
//!
//! The EHR the requests name does not exist, so a request the gate lets
//! through answers `404` from the service; a gate refusal answers `401`/`403`
//! before the service is reached.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferroehr::config::auth::{AssuranceConfig, AssuranceLevel, ProfessionalConfig};
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig};
use ferroehr::system_log::sender::start;
use http::{StatusCode, header};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::common::{self, BASE};

const SECRET: &str = "patient-data-gate-test-secret-0123456789";
const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

/// The assurance block every test runs under: substantial or higher.
fn assurance() -> AssuranceConfig {
    AssuranceConfig {
        levels: [
            ("loa-low".to_owned(), AssuranceLevel::Low),
            ("loa-substantial".to_owned(), AssuranceLevel::Substantial),
            ("loa-high".to_owned(), AssuranceLevel::High),
        ]
        .into_iter()
        .collect(),
        minimum: Some(AssuranceLevel::Substantial),
        ..AssuranceConfig::default()
    }
}

/// The app with the given gate configuration, auditing into the store when
/// `pool` is given.
async fn app(
    professional: Option<ProfessionalConfig>,
    audited: bool,
) -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let mut config = common::api_config(false);
    config.auth = common::hs256_auth_config(ISSUER, AUDIENCE, SECRET);
    if let Some(oidc) = config.auth.oidc.as_mut() {
        oidc.assurance = assurance();
        oidc.professional = professional;
    }
    let mut service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    if audited {
        let audit = AuditConfig {
            enabled: true,
            store: StoreConfig {
                enabled: true,
                ..StoreConfig::default()
            },
            syslog: SyslogConfig {
                enabled: false,
                ..SyslogConfig::default()
            },
            suppress_login_events: true,
            fail_mode: FailMode::Open,
            ..AuditConfig::default()
        };
        let (sender, _handle) = start(audit, None, Some(pool.clone()))
            .await
            .expect("audit start");
        service = service.with_audit(sender);
    }
    let router = common::router_with(config, Arc::new(service));
    (db, pool, router)
}

/// A bearer credential carrying `extra` on top of the mandatory claims.
fn bearer(extra: &Value) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).unwrap() + 3600;
    let mut claims = json!({
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": ["USER"],
    });
    for (key, value) in extra.as_object().expect("object") {
        claims[key] = value.clone();
    }
    common::hs256_bearer(SECRET, &claims)
}

fn ehr_path() -> String {
    format!("{BASE}/ehr/{EHR}")
}

/// RFC 9470 §3: a token below the minimum is refused on patient data with
/// `401` and the step-up challenge naming the accepted values.
#[tokio::test]
async fn a_token_below_the_minimum_gets_the_step_up_challenge() {
    let (_db, _pool, app) = app(None, false).await;
    for extra in [
        json!({ "sub": "dr-a", "acr": "loa-low" }),
        json!({ "sub": "dr-a", "acr": "not-mapped" }),
        json!({ "sub": "dr-a" }),
    ] {
        let (status, headers, body) =
            common::send(&app, common::get_authorized(&ehr_path(), &bearer(&extra))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{extra}: {body}");
        assert_eq!(
            headers
                .get(header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok()),
            Some(
                r#"Bearer realm="ferroehr", error="insufficient_user_authentication", error_description="patient data requires authentication at assurance level substantial or higher", acr_values="loa-high loa-substantial""#
            ),
            "{extra}"
        );
        let body: Value = serde_json::from_str(&body).expect("openEHR error body");
        assert!(body.get("message").is_some(), "{body}");
    }
}

/// A token at or above the minimum reaches the service, and a low token still
/// reaches a definition route, which carries no patient data.
#[tokio::test]
async fn a_token_at_or_above_the_minimum_passes() {
    let (_db, _pool, app) = app(None, false).await;
    for acr in ["loa-substantial", "loa-high"] {
        let status = common::send_status(
            &app,
            common::get_authorized(&ehr_path(), &bearer(&json!({ "sub": "dr-a", "acr": acr }))),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{acr}");
    }
    let status = common::send_status(
        &app,
        common::get_authorized(
            &format!("{BASE}/definition/template/adl1.4"),
            &bearer(&json!({ "sub": "dr-a", "acr": "loa-low" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// A client token naming no professional is refused on patient data with
/// `403` and the openEHR error body.
#[tokio::test]
async fn a_client_token_without_acting_for_is_refused() {
    let (_db, _pool, app) = app(Some(ProfessionalConfig::default()), false).await;
    let token = bearer(&json!({ "sub": "app-1", "client_id": "app-1", "acr": "loa-high" }));
    let (status, headers, body) =
        common::send(&app, common::get_authorized(&ehr_path(), &token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(headers.get(header::WWW_AUTHENTICATE).is_none());
    let body: Value = serde_json::from_str(&body).expect("openEHR error body");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("natural person")),
        "{body}"
    );
    // A person token passes the same gate.
    let person = bearer(&json!({ "sub": "dr-a", "client_id": "app-1", "acr": "loa-high" }));
    let status = common::send_status(&app, common::get_authorized(&ehr_path(), &person)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A client token acting for a named professional passes, and the access
/// record carries the client mode, the professional and the level.
#[tokio::test]
async fn a_client_acting_for_a_professional_passes_and_is_recorded() {
    let professional = ProfessionalConfig {
        acting_for_claim: Some("act.sub".to_owned()),
        ..ProfessionalConfig::default()
    };
    let (_db, pool, app) = app(Some(professional), true).await;
    let token = bearer(&json!({
        "sub": "app-1", "azp": "app-1", "acr": "loa-high",
        "act": { "sub": "prof-007" },
    }));
    let status = common::send_status(&app, common::get_authorized(&ehr_path(), &token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let mut row = None;
    for _ in 0..50 {
        row = sqlx::query(
            "SELECT principal, acting_mode, assurance_level, assurance_value, professional_id, \
             fhir FROM audit.audit_event WHERE operation = 'ehr_get_by_id'",
        )
        .fetch_optional(&pool)
        .await
        .expect("query");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let row = row.expect("the access record was stored");
    assert_eq!(
        row.get::<Option<String>, _>("principal").as_deref(),
        Some("app-1")
    );
    assert_eq!(
        row.get::<Option<String>, _>("acting_mode").as_deref(),
        Some("client")
    );
    assert_eq!(
        row.get::<Option<String>, _>("assurance_level").as_deref(),
        Some("high")
    );
    assert_eq!(
        row.get::<Option<String>, _>("assurance_value").as_deref(),
        Some("loa-high")
    );
    assert_eq!(
        row.get::<Option<String>, _>("professional_id").as_deref(),
        Some("prof-007")
    );
    let fhir: Value = row.get("fhir");
    assert_eq!(fhir["agent"][0]["who"]["identifier"]["value"], "prof-007");
    assert_eq!(fhir["agent"][0]["requestor"], true);
    assert_eq!(fhir["agent"][1]["who"]["identifier"]["value"], "app-1");
    assert_eq!(fhir["agent"][1]["requestor"], false);
}
