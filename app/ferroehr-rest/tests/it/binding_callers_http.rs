// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller classification at the assembled router (#5011), over a real
//! `FerroEhrService` on a real `PostgreSQL` 18 with the local party source.
//!
//! A token that fits no step of the classification is refused with `401`,
//! and the gate logs the typed reason, never a claim value. A user is named
//! by their `fhirUser` (SMART App Launch 2.2.0, Scopes for requesting
//! identity data), a client credentials token by its `sub` equal to its
//! `client_id` (RFC 9068 §2.2), and an actor by `act` (RFC 8693 §4.1); the
//! rest is our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::party::config::PartySection;
use http::StatusCode;
use serde_json::json;

use crate::caller_kinds_http::{audience, bearer, machines};
use crate::common::{self, BASE};
use crate::party_binding_http::{
    Capture, audited_service, list, professional_of, router_with, service,
};

/// An issuer of client credentials tokens.
const MACHINES: &str = "https://machines.callers.example";
/// The issuer the persons sign in at.
const PERSONS: &str = "https://persons.callers.example";
/// The `identifier_issuer` of that issuer's `[auth.issuer.<name>.party]` table.
const PERSONS_LOGIN: &str = "https://persons.callers.example/users";
/// The onward client a person's token names, such as `FerroBRIDGE`.
const ONWARD: &str = "bridge";

/// A `[auth.issuer.<name>.party]` table binding the logins of `issuer`.
fn party(issuer: &str, identifier_issuer: &str) -> PartySection {
    PartySection {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer),
        identifier_issuer: identifier_issuer.to_owned(),
        identifier_type: "guid".to_owned(),
        ..PartySection::default()
    }
}

/// The configuration trusting the machines issuer and the bound persons
/// issuer.
fn config() -> AppConfig {
    let persons: IssuerEntry = machines(PERSONS);
    let mut auth = common::auth(AuthSection {
        issuer: common::bound(
            vec![machines(MACHINES), persons],
            vec![party(PERSONS, PERSONS_LOGIN)],
        ),
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    }
}

/// The router over `svc` under `config`, binding through the local source.
async fn router(svc: Arc<FerroEhrService>, config: AppConfig) -> Router {
    let source = Arc::new(svc.party_source("demographic"));
    router_with(svc, config, source).await
}

/// A token that fits no step of the classification, with the reason code
/// the classification gives it.
fn unfit() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "a user token that names no client",
            bearer(
                PERSONS,
                &audience(PERSONS),
                &json!({ "sub": "a-person", "client_id": null, "scope": "user/template-*.r" }),
            ),
            "client-missing",
        ),
        (
            "a fhirUser that names no user resource",
            bearer(
                PERSONS,
                &audience(PERSONS),
                &json!({ "sub": "a-person", "client_id": "app", "fhirUser": "Group/stranger",
                         "scope": "user/template-*.r" }),
            ),
            "fhir-user-refused",
        ),
        (
            "an actor chain deeper than admitted",
            bearer(
                PERSONS,
                &audience(PERSONS),
                &json!({ "sub": "a-person", "client_id": "app",
                         "act": { "sub": "stranger", "act": { "sub": "gateway" } },
                         "scope": "user/template-*.r" }),
            ),
            "actor-chain-refused",
        ),
    ]
}

/// A token that fits no step of the classification is refused with `401`,
/// and the gate logs why by its stable reason code, never a claim value. The
/// federation kind is in `federation_http`.
#[tokio::test]
async fn a_token_that_fits_no_step_is_refused_naming_why() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let (_db, svc) = service().await;
    let app = router(svc, config()).await;
    for (case, credential, reason) in unfit() {
        let capture = Capture::default();
        let lines = Arc::clone(&capture.0);
        {
            let _guard =
                tracing::subscriber::set_default(tracing_subscriber::registry().with(capture));
            let (status, body) = list(&app, &credential).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{case}: {body}");
        }
        let lines = lines.lock().unwrap().clone();
        let logged = lines.iter().any(|line| {
            line.contains("a request was refused") && line.contains(&format!("reason={reason}"))
        });
        assert!(logged, "{case}: the reason {reason:?} is logged: {lines:?}");
        for line in &lines {
            assert!(
                !line.contains("a-person") && !line.contains("stranger"),
                "{case}: a claim value was logged: {line}"
            );
        }
    }
}

/// A person signs in through any client of their issuer whose token names
/// this server: the authorization server's registry is the one list of
/// clients (RFC 9068 §4).
#[tokio::test]
async fn a_person_signs_in_through_any_client_of_their_issuer() {
    let (_db, svc) = service().await;
    let key = "4f3a2b1c-0d9e-4f8a-b7c6-d5e4f3a2b1c0";
    professional_of(&svc, PERSONS_LOGIN, key, "532").await;
    let app = router(Arc::clone(&svc), config()).await;
    let token = bearer(
        PERSONS,
        &audience(PERSONS),
        &json!({ "sub": key, "client_id": "portal", "scope": common::FULL_SCOPES }),
    );
    let (status, body) = list(&app, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Creates an EHR as `credential`; the committer of its `EHR_STATUS`.
async fn committer_of_new_ehr(app: &Router, credential: &str) -> serde_json::Value {
    let (status, body) = common::send_body(
        app,
        common::req("POST", format!("{BASE}/ehr"))
            .auth(credential)
            .prefer_representation()
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr: serde_json::Value = serde_json::from_str(&body).expect("an EHR");
    let ehr_id = ehr
        .pointer("/ehr_id/value")
        .and_then(serde_json::Value::as_str)
        .expect("the EHR id")
        .to_owned();
    let (status, version) = common::send_json(
        app,
        common::req(
            "GET",
            format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"),
        )
        .auth(credential)
        .accept("application/json")
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    version["commit_audit"]["committer"].clone()
}

/// The `(acting_mode, client_id)` of the `ehr_create` record of `principal`.
async fn delegated_record(
    pool: &sqlx::PgPool,
    principal: &str,
) -> (Option<String>, Option<String>) {
    for _ in 0..50 {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT acting_mode, client_id FROM audit.access_trail \
             WHERE operation = 'ehr_create' AND principal = $1",
        )
        .bind(principal)
        .fetch_optional(pool)
        .await
        .expect("query");
        if let Some(row) = row {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no ehr_create record of {principal} was stored");
}

/// An onward token names the person in `sub` and the onward client in
/// `client_id` (RFC 7523 §3, RFC 8693 §2.2): the request is bound to the
/// person, commits as their `PERSON`, and the record carries the person's
/// mode and the client.
#[tokio::test]
async fn an_onward_token_is_the_persons_with_its_client() {
    let (db, _audit, svc) = audited_service().await;
    let sub = "5e4f3a2b-1c0d-4e9f-8a7b-6c5d4e3f2a1b";
    professional_of(&svc, PERSONS_LOGIN, sub, "532").await;
    let app = router(svc, config()).await;
    let exchanged = bearer(
        PERSONS,
        &audience(PERSONS),
        &json!({ "sub": sub, "client_id": ONWARD, "scope": common::FULL_SCOPES }),
    );
    let committer = committer_of_new_ehr(&app, &exchanged).await;
    assert_eq!(committer["external_ref"]["type"], "PERSON", "{committer}");
    assert_eq!(committer["identifiers"][0]["id"], sub, "{committer}");
    let (mode, client) = delegated_record(&db.pool(), sub).await;
    assert_eq!(mode.as_deref(), Some("person"));
    assert_eq!(client.as_deref(), Some(ONWARD));
}
