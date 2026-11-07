// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What each local caller kind may do (#5100), driven through the assembled
//! router over a real `FerroEhrService` on a real `PostgreSQL` 18: a system
//! caller holds `system/` scopes only and commits only as a declared client,
//! a guest is admitted only where its issuer's `[[auth.party]]` table admits
//! guests, and a delegated person is bound to their own `PERSON`.
//!
//! RFC 9068 §2.2 and §5 name the person in `sub` and mark an app-only token;
//! RFC 8693 §4.1 names an actor in `act`; SMART on openEHR
//! `master08-scopes.adoc` §Resource Scopes gives the `patient`, `user` and
//! `system` contexts. Each case runs against the standards-only `rfc9068`
//! profile first, then the `entra` and `keycloak` example profiles. What a
//! caller kind may do here is our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::clients::{Origin, SystemClient};
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry, IssuerKey};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::{Delegate, Marker, Preset};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{
    AUDIENCE, ISSUER, LOGIN, audited_service, list, professional, refusal_rows, refused, service,
};

/// The standards-only machines issuer.
const MACHINES: &str = "https://machines.example";
/// The Entra ID machines issuer.
const ENTRA: &str = "https://login.microsoftonline.example/tenant/v2.0";
/// The Keycloak machines issuer.
const KEYCLOAK: &str = "https://keycloak.example/realms/care";
/// The Keycloak service account of the client `lab`.
const SERVICE_ACCOUNT: &str = "service-account-lab";
/// The actor a delegated token names.
const GATEWAY: &str = "gateway";
/// The SMART scopes of a backend client that reads templates and writes EHRs.
const SYSTEM_SCOPES: &str = "system/composition-*.cruds system/template-*.r";

/// The audience of the machines issuer `issuer`: each issuer has its own.
fn audience(issuer: &str) -> String {
    format!("{AUDIENCE}-{}", issuer.trim_start_matches("https://"))
}

/// A `machines` issuer at `issuer` under `profile`.
fn machines(issuer: &str, profile: Preset) -> IssuerEntry {
    IssuerEntry {
        issuer: issuer.to_owned(),
        kind: Some(IssuerKind::Machines),
        profile,
        audience: vec![audience(issuer)],
        jwks_file: common::jwks_file().display().to_string(),
        // NOTE: the test key signs RFC 9068 `at+jwt` tokens, which each
        // profile's overridable `token_types` admits here.
        token_types: Some(vec!["at+jwt".to_owned()]),
        ..IssuerEntry::default()
    }
}

/// The configuration trusting the persons issuer, whose guests the
/// `[[auth.party]]` table admits when `guests`, and the three machines
/// issuers, with `clients` declared.
fn config(guests: bool, clients: Vec<SystemClient>) -> AppConfig {
    let persons = IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        guest: Some(vec![Marker::Claim {
            claim: "acct".to_owned(),
            value: "1".to_owned(),
        }]),
        delegates: Some(vec![Delegate::Actor {
            issuer: None,
            sub: GATEWAY.to_owned(),
        }]),
        ..common::entry(ISSUER, AUDIENCE)
    };
    let keycloak = IssuerEntry {
        app_only: Some(vec![Marker::Subjects(vec![SERVICE_ACCOUNT.to_owned()])]),
        ..machines(KEYCLOAK, Preset::Keycloak)
    };
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(vec![
            persons,
            machines(MACHINES, Preset::Rfc9068),
            machines(ENTRA, Preset::Entra),
            keycloak,
        ]),
        party: vec![PartySection {
            issuer: ISSUER.to_owned(),
            identifier_issuer: LOGIN.to_owned(),
            identifier_type: "guid".to_owned(),
            guests,
            ..PartySection::default()
        }],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        system_clients: clients,
        ..AppConfig::default()
    }
}

/// The router over `svc` under `config`.
async fn router(svc: Arc<FerroEhrService>, config: AppConfig) -> Router {
    let source = Arc::new(svc.party_source("demographic"));
    let authenticator = Authenticator::new(
        &config,
        None,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await
    .expect("the authenticator builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, svc, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// A bearer of `issuer` for `audience` carrying `claims`.
fn bearer(issuer: &str, audience: &str, claims: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut all = json!({ "iss": issuer, "aud": audience, "exp": exp });
    if let (Some(all), Some(claims)) = (all.as_object_mut(), claims.as_object()) {
        all.extend(claims.clone());
    }
    common::bearer(&all)
}

/// An app-only token of each machines issuer for the client `lab`, carrying
/// `scope` in the claim its profile reads app scopes from.
fn app_only(scope: &str) -> [(&'static str, String); 3] {
    [
        (
            "rfc9068",
            bearer(
                MACHINES,
                &audience(MACHINES),
                &json!({ "sub": "lab", "client_id": "lab", "scope": scope }),
            ),
        ),
        (
            "entra",
            bearer(
                ENTRA,
                &audience(ENTRA),
                &json!({ "sub": "8f1c", "azp": "lab", "tid": "tenant", "uti": "u1",
                         "ver": "2.0", "idtyp": "app",
                         "roles": scope.split(' ').collect::<Vec<_>>() }),
            ),
        ),
        (
            "keycloak",
            bearer(
                KEYCLOAK,
                &audience(KEYCLOAK),
                &json!({ "sub": SERVICE_ACCOUNT, "azp": "lab", "scope": scope }),
            ),
        ),
    ]
}

/// Creates an EHR as `credential`; returns the status and body.
async fn create_ehr(app: &Router, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req("POST", format!("{BASE}/ehr"))
            .auth(credential)
            .prefer_representation()
            .build(),
    )
    .await
}

#[tokio::test]
async fn a_system_caller_holds_system_scopes_only() {
    let (_db, svc) = service().await;
    let app = router(svc, config(false, Vec::new())).await;
    for (profile, credential) in app_only("user/template-*.r") {
        let (status, body) = list(&app, &credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{profile}: {body}");
        assert!(body.contains("system/ scopes only"), "{profile}: {body}");
    }
    for (profile, credential) in app_only("patient/template-*.r") {
        assert_eq!(
            list(&app, &credential).await.0,
            StatusCode::FORBIDDEN,
            "{profile}"
        );
    }
    for (profile, credential) in app_only(SYSTEM_SCOPES) {
        let (status, body) = list(&app, &credential).await;
        assert_eq!(status, StatusCode::OK, "{profile} reads: {body}");
    }
}

#[tokio::test]
async fn only_a_declared_system_client_commits() {
    let (_db, svc) = service().await;
    let undeclared = router(Arc::clone(&svc), config(false, Vec::new())).await;
    for (profile, credential) in app_only(SYSTEM_SCOPES) {
        let (status, body) = create_ehr(&undeclared, &credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{profile}: {body}");
    }
    let declared = [MACHINES, ENTRA, KEYCLOAK]
        .into_iter()
        .map(|issuer| SystemClient {
            issuer: issuer.to_owned(),
            client_id: "lab".to_owned(),
            origins: vec![Origin::Relay],
            ..SystemClient::default()
        })
        .collect();
    let app = router(svc, config(false, declared)).await;
    for (profile, credential) in app_only(SYSTEM_SCOPES) {
        let (status, body) = create_ehr(&app, &credential).await;
        assert_eq!(status, StatusCode::CREATED, "{profile}: {body}");
    }
}

#[tokio::test]
async fn a_guest_is_refused_unless_its_issuer_admits_guests() {
    let (_db, svc) = service().await;
    let key = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
    professional(&svc, key, "532").await;
    let guest = bearer(
        ISSUER,
        AUDIENCE,
        &json!({ "sub": key, "acct": "1", "scope": common::FULL_SCOPES }),
    );
    let refusing = router(Arc::clone(&svc), config(false, Vec::new())).await;
    refused(&refusing, &guest, "refused-guest").await;
    let admitting = router(svc, config(true, Vec::new())).await;
    let (status, body) = list(&admitting, &guest).await;
    assert_eq!(status, StatusCode::OK, "an admitted guest binds: {body}");
}

#[tokio::test]
async fn a_delegated_person_commits_as_their_own_person() {
    let (_db, svc) = service().await;
    let key = "8d9e0f1a-2b3c-4d4e-9f5a-6b7c8d9e0f1a";
    professional(&svc, key, "532").await;
    let app = router(Arc::clone(&svc), config(false, Vec::new())).await;
    let delegated = bearer(
        ISSUER,
        AUDIENCE,
        &json!({ "sub": key, "client_id": "portal", "act": { "sub": GATEWAY } }),
    );
    let (status, body) = create_ehr(&app, &delegated).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr: Value = serde_json::from_str(&body).expect("an EHR");
    let ehr_id = ehr
        .pointer("/ehr_id/value")
        .and_then(Value::as_str)
        .expect("the EHR id")
        .to_owned();
    let (status, version) = common::send_json(
        &app,
        common::req(
            "GET",
            format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"),
        )
        .auth(&delegated)
        .accept("application/json")
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["external_ref"]["type"], "PERSON", "{committer}");
    assert_eq!(committer["identifiers"][0]["id"], key, "{committer}");
}

/// A system caller refused for a personal scope is recorded under
/// `refused-system-personal-scope`, once per refused request (#5140).
#[tokio::test]
async fn a_system_caller_with_a_personal_scope_is_recorded_with_its_code() {
    let (db, _audit, svc) = audited_service().await;
    let app = router(svc, config(false, Vec::new())).await;
    let tokens = app_only("user/template-*.r");
    for (profile, credential) in &tokens {
        assert_eq!(
            list(&app, credential).await.0,
            StatusCode::FORBIDDEN,
            "{profile}"
        );
    }
    let rows = refusal_rows(&db.pool(), "refused-system-personal-scope", tokens.len()).await;
    assert_eq!(rows.len(), tokens.len(), "one record per refused request");
}
