// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party binding across two issuers and the two binding markers (#5011),
//! driven through the assembled router over a real `FerroEhrService` on a real
//! `PostgreSQL` 18, with the local party source.
//!
//! An Entra-shaped issuer binds on `oid` and a Keycloak-shaped issuer on
//! `sub`, each under its own `identifier_issuer` and audience, so the same
//! identifier value under one issuer never reaches the other's party. A login
//! whose caller kind needs the other marker than its party carries is refused
//! with `403` (`refused-wrong-marker`). No openEHR spec governs resolving a
//! login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry, IssuerKey};
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::{Marker, Preset, Representation};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common;
use crate::party_binding_http::{list, professional, professional_of, refused, service, token};

const ENTRA: &str = "https://login.entra.example/tenant/v2.0";
const ENTRA_AUDIENCE: &str = "ferroehr-entra";
/// The `identifier_issuer` of the Entra-shaped issuer's `[[auth.party]]` table.
const ENTRA_LOGIN: &str = "https://login.entra.example/tenant";
const KEYCLOAK: &str = "https://keycloak.example/realms/care";
const KEYCLOAK_AUDIENCE: &str = "ferroehr-keycloak";
/// The `identifier_issuer` of the Keycloak-shaped issuer's `[[auth.party]]`
/// table.
const KEYCLOAK_LOGIN: &str = "https://keycloak.example/realms/care/users";
/// The one identifier value both issuers issue, to different persons.
const SHARED: &str = "6c7d8e9f-0a1b-4c2d-8e3f-4a5b6c7d8e9f";

/// A `[[auth.party]]` table binding the logins of `issuer` under
/// `identifier_issuer`.
fn party(issuer: &str, identifier_issuer: &str) -> PartySection {
    PartySection {
        issuer: issuer.to_owned(),
        identifier_issuer: identifier_issuer.to_owned(),
        identifier_type: "guid".to_owned(),
        ..PartySection::default()
    }
}

/// The configuration trusting the Entra-shaped issuer, bound on `oid`, and the
/// Keycloak-shaped issuer, bound on `sub`.
fn two_issuers() -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(vec![
            IssuerEntry {
                profile: Preset::Entra,
                person_key: Some("oid".to_owned()),
                jwks_file: common::jwks_file().display().to_string(),
                token_types: Some(vec!["at+jwt".to_owned()]),
                ..common::entry(ENTRA, ENTRA_AUDIENCE)
            },
            IssuerEntry {
                profile: Preset::Keycloak,
                // NOTE: the Keycloak profile names its service accounts by
                // their declared `sub`, which it has no default for.
                app_only: Some(vec![Marker::Subjects(vec![
                    "service-account-ferroehr".to_owned(),
                ])]),
                jwks_file: common::jwks_file().display().to_string(),
                token_types: Some(vec!["at+jwt".to_owned()]),
                ..common::entry(KEYCLOAK, KEYCLOAK_AUDIENCE)
            },
        ]),
        party: vec![party(ENTRA, ENTRA_LOGIN), party(KEYCLOAK, KEYCLOAK_LOGIN)],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    }
}

/// The router over `svc` under `config`, looking every caller up in the local
/// party source.
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

/// An Entra member user's token whose `oid` is `oid`, its `sub` a pairwise
/// value of its own.
fn entra(oid: &str) -> String {
    bearer(
        ENTRA,
        ENTRA_AUDIENCE,
        &json!({ "sub": "pairwise-subject", "oid": oid, "tid": "tenant", "uti": "u1", "ver": "2.0",
                 "acct": 0, "azp": common::TEST_CLIENT, "scp": common::FULL_SCOPES }),
    )
}

/// A Keycloak user's token whose `sub` is `sub`.
fn keycloak(sub: &str) -> String {
    bearer(
        KEYCLOAK,
        KEYCLOAK_AUDIENCE,
        &json!({ "sub": sub, "azp": common::TEST_CLIENT }),
    )
}

#[tokio::test]
async fn each_issuer_binds_its_own_person_and_never_the_other_issuers() {
    let (_db, svc) = service().await;
    let entra_person = "1f2a3b4c-5d6e-4f7a-8b9c-0d1e2f3a4b5c";
    let keycloak_person = "2a3b4c5d-6e7f-4a8b-9c0d-1e2f3a4b5c6d";
    professional_of(&svc, ENTRA_LOGIN, entra_person, "532").await;
    professional_of(&svc, KEYCLOAK_LOGIN, keycloak_person, "532").await;
    let app = router(Arc::clone(&svc), two_issuers()).await;

    let (status, body) = list(&app, &entra(entra_person)).await;
    assert_eq!(status, StatusCode::OK, "the Entra person is served: {body}");
    let (status, body) = list(&app, &keycloak(keycloak_person)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the Keycloak person is served: {body}"
    );
}

#[tokio::test]
async fn the_same_identifier_under_the_other_issuer_does_not_bind() {
    let (_db, svc) = service().await;
    // The Entra person is deactivated, the Keycloak person with the same
    // identifier value stays active: were the value matched without its
    // issuer, both logins would see two parties.
    professional_of(&svc, ENTRA_LOGIN, SHARED, "800").await;
    professional_of(&svc, KEYCLOAK_LOGIN, SHARED, "532").await;
    let app = router(Arc::clone(&svc), two_issuers()).await;

    refused(&app, &entra(SHARED), "refused-inactive").await;
    let (status, body) = list(&app, &keycloak(SHARED)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the Keycloak person binds its own party: {body}"
    );
    // The Entra key is `oid`: a `sub` of the same value never reaches the
    // deactivated Entra person that value names.
    let as_sub = bearer(
        ENTRA,
        ENTRA_AUDIENCE,
        &json!({ "sub": SHARED, "oid": "3b4c5d6e-7f8a-4b9c-8d0e-2f3a4b5c6d7e", "tid": "tenant",
                 "uti": "u2", "ver": "2.0", "acct": 0, "azp": common::TEST_CLIENT,
                 "scp": common::FULL_SCOPES }),
    );
    let (_, body) = list(&app, &as_sub).await;
    assert!(
        !body.contains("refused-inactive"),
        "an Entra login is bound on oid, never on sub: {body}"
    );
}

/// The claim naming the subject a representative acts for.
const REPRESENTS: &str = "represents";

/// The configuration of the single test issuer whose representation claim is
/// `represents`.
fn representing() -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(vec![IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            representation: Some(Representation {
                claim: REPRESENTS.to_owned(),
                ground_claim: None,
            }),
            ..common::entry(
                crate::party_binding_http::ISSUER,
                crate::party_binding_http::AUDIENCE,
            )
        }]),
        party: vec![party(
            crate::party_binding_http::ISSUER,
            crate::party_binding_http::LOGIN,
        )],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    }
}

#[tokio::test]
async fn a_representative_login_on_a_professional_row_is_refused_as_the_wrong_marker() {
    let (_db, svc) = service().await;
    let key = "4c5d6e7f-8a9b-4c0d-9e1f-3a4b5c6d7e8f";
    professional(&svc, key, "532").await;
    let app = router(Arc::clone(&svc), representing()).await;

    let (status, body) = list(&app, &token(key, &json!({}))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the person login binds the professional row: {body}"
    );
    refused(
        &app,
        &token(
            key,
            &json!({ REPRESENTS: "5d6e7f8a-9b0c-4d1e-8f2a-4b5c6d7e8f9a" }),
        ),
        "refused-wrong-marker",
    )
    .await;
}
