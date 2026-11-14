// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The patient-data authentication gate over the real axum app: the
//! `[authz.assurance]` step-up refusal (RFC 9470 §3), the refusal of a
//! `persons` token through a client its issuer does not list, and the access
//! record carrying the acting mode, the assurance level and the professional
//! (EHDS Annex II 3.1 and 3.2(b), `docs/law/eu/ehds/text.html Annex II 3`).
//!
//! The EHR the requests name does not exist, so a request the gate lets
//! through answers `404` from the service; a gate refusal answers `401`/`403`
//! before the service is reached.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig};
use ferroehr::system_log::sender::start;
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::IssuerEntry;
use http::{StatusCode, header};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::common::{self, BASE};

const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

/// The assurance block every test runs under: substantial or higher.
fn assurance() -> AssuranceSection {
    AssuranceSection {
        levels: [
            ("loa-low".to_owned(), AssuranceLevel::Low),
            ("loa-substantial".to_owned(), AssuranceLevel::Substantial),
            ("loa-high".to_owned(), AssuranceLevel::High),
        ]
        .into_iter()
        .collect(),
        minimum: Some(AssuranceLevel::Substantial),
        ..AssuranceSection::default()
    }
}

/// The app whose CDR grants each client of `delegates` `act_for_persons`,
/// auditing into the store when `audited`.
async fn app(delegates: &[&str], audited: bool) -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let mut config = common::api_config(false);
    config.auth = common::trusting(vec![IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(ISSUER, AUDIENCE)
    }]);
    config.assurance = assurance();
    let mut service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    for client in delegates {
        common::grant_standing(&service, ISSUER, client, Operation::ActForPersons).await;
    }
    if audited {
        let audit = AuditConfig {
            enabled: true,
            store: StoreConfig {
                enabled: true,
                ..StoreConfig::default()
            },
            syslog: None,
            suppress_login_events: true,
            fail_mode: FailMode::Open,
            ..AuditConfig::default()
        };
        let (sender, _handle) = start(audit, None, Some(pool.clone()))
            .await
            .expect("audit start");
        service = service.with_audit(sender);
    }
    let router = common::router_with(config, Arc::new(service)).await;
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
    common::bearer(&claims)
}

fn ehr_path() -> String {
    format!("{BASE}/ehr/{EHR}")
}

/// RFC 9470 §3: a token below the minimum is refused on patient data with
/// `401` and the step-up challenge naming the accepted values.
#[tokio::test]
async fn a_token_below_the_minimum_gets_the_step_up_challenge() {
    let (_db, _pool, app) = app(&[], false).await;
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
                r#"Bearer realm="ferroehr", error="insufficient_user_authentication", acr_values="loa-high loa-substantial""#
            ),
            "{extra}"
        );
        let body: Value = serde_json::from_str(&body).expect("problem details");
        assert!(body.get("detail").is_some(), "{body}");
    }
}

/// A token at or above the minimum reaches the service, and a low token still
/// reaches a definition route, which carries no patient data.
#[tokio::test]
async fn a_token_at_or_above_the_minimum_passes() {
    let (_db, _pool, app) = app(&[], false).await;
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

/// A person's token through a client its issuer lists neither as first-party
/// nor as a delegate is refused, and the same person through the first-party
/// client passes.
#[tokio::test]
async fn a_person_token_through_an_unlisted_client_is_refused() {
    let (_db, _pool, app) = app(&[], false).await;
    let token = bearer(&json!({ "sub": "dr-a", "client_id": "app-1", "acr": "loa-high" }));
    let (status, _headers, body) =
        common::send(&app, common::get_authorized(&ehr_path(), &token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let person = bearer(&json!({ "sub": "dr-a", "acr": "loa-high" }));
    let status = common::send_status(&app, common::get_authorized(&ehr_path(), &person)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A third-party client the CDR grants `act_for_persons` acts for the person
/// its token names, and the access record carries the delegated mode, the
/// professional and the level.
#[tokio::test]
async fn a_delegate_acting_for_a_professional_passes_and_is_recorded() {
    let (_db, pool, app) = app(&["app-1"], true).await;
    let token = bearer(&json!({ "sub": "prof-007", "client_id": "app-1", "acr": "loa-high" }));
    let status = common::send_status(&app, common::get_authorized(&ehr_path(), &token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let mut row = None;
    for _ in 0..50 {
        row = sqlx::query(
            "SELECT principal, acting_mode, assurance_level, assurance_value, professional_id, \
             chain, chain_seq FROM audit.access_trail WHERE operation = 'ehr_get_by_id'",
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
        Some("prof-007")
    );
    assert_eq!(
        row.get::<Option<String>, _>("acting_mode").as_deref(),
        Some("delegated")
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
    // BALP 1.1.4 §3:5.7.5.4: the token's subject is the requesting
    // `agent:user`, with the professional's identifier as its `ihe-otherId`.
    let fhir: Value =
        common::rendered_audit_event(&pool, row.get("chain"), row.get("chain_seq")).await;
    let user = fhir["agent"]
        .as_array()
        .expect("agents")
        .iter()
        .find(|agent| agent["requestor"] == true)
        .expect("the agent:user");
    assert_eq!(user["who"]["identifier"]["value"], "prof-007", "{fhir}");
    let other_id = user["extension"]
        .as_array()
        .expect("the agent:user extensions")
        .iter()
        .find(|x| x["url"] == "https://profiles.ihe.net/ITI/BALP/StructureDefinition/ihe-otherId")
        .expect("the professional identifier");
    assert_eq!(other_id["valueIdentifier"]["value"], "prof-007", "{fhir}");
}
