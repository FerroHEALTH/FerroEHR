// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller classification of each issuer kind at the assembled router
//! (#5011), over a real `FerroEhrService` on a real `PostgreSQL` 18 with the
//! local party source.
//!
//! A token that fits no step of its issuer's kind is refused with `401`, and
//! the gate logs the typed reason, never a claim value. The Entra ID guest
//! markers are its `idp` and `acct` claims (Microsoft, "Access token claims
//! reference", `idp`; "Optional claims reference", `acct`), so a member of
//! another tenant is a guest by `idp`. A `scopeless_subjects` entry names its
//! issuer, so the same `sub` of another issuer gets nothing. RFC 9068 §2.2
//! and §5 name the person and the app-only token; the rest is our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerEntry, ScopelessSubject};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::{Marker, Preset};
use http::StatusCode;
use serde_json::json;

use crate::caller_kinds_http::{audience, bearer, machines};
use crate::common;
use crate::party_binding_http::{Capture, list, professional_of, refused, router_with, service};

/// A standards-only `machines` issuer.
const MACHINES: &str = "https://machines.callers.example";
/// A second standards-only `machines` issuer, whose client has the same `sub`.
const OTHER_MACHINES: &str = "https://other-machines.callers.example";
/// A standards-only `patients` issuer.
const PATIENTS: &str = "https://patients.callers.example";
/// An Entra ID `persons` issuer.
const ENTRA: &str = "https://login.microsoftonline.example/callers/v2.0";
/// The `identifier_issuer` of the Entra issuer's `[[auth.party]]` table.
const ENTRA_LOGIN: &str = "https://login.microsoftonline.example/callers";
/// A Keycloak `persons` realm.
const KEYCLOAK: &str = "https://keycloak.example/realms/callers";
/// The `identifier_issuer` of the realm's `[[auth.party]]` table.
const KEYCLOAK_LOGIN: &str = "https://keycloak.example/realms/callers/users";
/// The realm's first-party client.
const VIEWER: &str = "ferroehr-viewer";

/// A `persons` issuer at `issuer` under `profile`.
fn persons(issuer: &str, profile: Preset) -> IssuerEntry {
    IssuerEntry {
        kind: Some(IssuerKind::Persons),
        ..machines(issuer, profile)
    }
}

/// A `[[auth.party]]` table binding the logins of `issuer`.
fn party(issuer: &str, identifier_issuer: &str) -> PartySection {
    PartySection {
        issuer: issuer.to_owned(),
        identifier_issuer: identifier_issuer.to_owned(),
        identifier_type: "guid".to_owned(),
        ..PartySection::default()
    }
}

/// The configuration trusting one issuer of each local kind and a second
/// machines issuer, with `scopeless` on `[auth] scopeless_subjects`.
fn config(scopeless: Vec<ScopelessSubject>) -> AppConfig {
    let entra = IssuerEntry {
        first_party_clients: Some(vec![common::TEST_CLIENT.to_owned()]),
        ..persons(ENTRA, Preset::Entra)
    };
    let keycloak = IssuerEntry {
        first_party_clients: Some(vec![VIEWER.to_owned()]),
        app_only: Some(vec![Marker::Subjects(vec![
            "service-account-viewer".to_owned(),
        ])]),
        ..persons(KEYCLOAK, Preset::Keycloak)
    };
    let patients = IssuerEntry {
        kind: Some(IssuerKind::Patients),
        ..machines(PATIENTS, Preset::Rfc9068)
    };
    let mut auth = common::auth(AuthSection {
        issuer: vec![
            machines(MACHINES, Preset::Rfc9068),
            machines(OTHER_MACHINES, Preset::Rfc9068),
            patients,
            entra,
            keycloak,
        ],
        party: vec![party(ENTRA, ENTRA_LOGIN), party(KEYCLOAK, KEYCLOAK_LOGIN)],
        scopeless_subjects: scopeless,
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

/// An Entra member user's token whose `oid` is `oid`, with `extra` claims.
fn entra(oid: &str, extra: &serde_json::Value) -> String {
    let mut claims = json!({
        "sub": "pairwise-subject", "oid": oid, "tid": "callers", "uti": "u1", "ver": "2.0",
        "acct": 0, "azp": common::TEST_CLIENT, "scp": common::FULL_SCOPES,
    });
    if let (Some(claims), Some(extra)) = (claims.as_object_mut(), extra.as_object()) {
        for (name, value) in extra {
            if value.is_null() {
                claims.remove(name);
            } else {
                claims.insert(name.clone(), value.clone());
            }
        }
    }
    bearer(ENTRA, &audience(ENTRA), &claims)
}

/// A token of each issuer kind that fits no step of that kind, with the
/// reason the classification gives it.
fn unfit() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "a machines token that is not app-only",
            bearer(
                MACHINES,
                &audience(MACHINES),
                &json!({ "sub": "a-person", "client_id": "lab", "scope": "system/template-*.r" }),
            ),
            "a machines token is not app-only",
        ),
        (
            "a persons token of a client neither first-party nor a delegate",
            bearer(
                KEYCLOAK,
                &audience(KEYCLOAK),
                &json!({ "sub": "9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d", "azp": "stranger" }),
            ),
            "the client is neither first-party nor a declared delegate",
        ),
        (
            "an Entra user token without acct",
            entra(
                "8b7c6d5e-4f3a-4b2c-9d1e-0f9a8b7c6d5e",
                &json!({ "acct": null }),
            ),
            "a user token lacks the claim acct",
        ),
        (
            "a patients token with a user/ scope",
            bearer(
                PATIENTS,
                &audience(PATIENTS),
                &json!({ "sub": "a-patient", "client_id": "app",
                         "scope": "patient/composition-*.r user/composition-*.r" }),
            ),
            "a patients token does not name only the record subject",
        ),
    ]
}

/// Every local issuer kind refuses a token that fits none of its steps with
/// `401`, and the gate logs why by the typed reason, naming a claim and never
/// a claim value. The federation kind is in `federation_http`.
#[tokio::test]
async fn a_token_that_fits_no_kind_of_its_issuer_is_refused_naming_why() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let (_db, svc) = service().await;
    let app = router(svc, config(Vec::new())).await;
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
            line.contains("an access token names no caller kind") && line.contains(reason)
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

/// An Entra member's token carries no `idp`, so it is no guest and binds. A
/// member-type user of another tenant carries `acct = 0` and an `idp` naming
/// its home tenant, so the profile's `idp` marker makes them a guest, refused
/// where the issuer's table admits no guests, like `acct = 1`.
#[tokio::test]
async fn an_entra_member_binds_and_a_member_of_another_tenant_is_a_guest() {
    let (_db, svc) = service().await;
    let oid = "7c6d5e4f-3a2b-4c1d-8e0f-9a8b7c6d5e4f";
    professional_of(&svc, ENTRA_LOGIN, oid, "532").await;
    let app = router(svc, config(Vec::new())).await;
    let (status, body) = list(&app, &entra(oid, &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "a member without idp binds: {body}");
    let home = json!({ "idp": "https://sts.windows.example/home-tenant/" });
    refused(&app, &entra(oid, &home), "refused-guest").await;
    refused(&app, &entra(oid, &json!({ "acct": 1 })), "refused-guest").await;
}

/// A `scopeless_subjects` entry names its issuer: the listed client reads
/// with a token carrying no resource scope, and the same `sub` of another
/// issuer is held to its scopes and refused.
#[tokio::test]
async fn a_scopeless_entry_admits_the_subject_of_its_own_issuer_only() {
    let (_db, svc) = service().await;
    let scopeless = vec![ScopelessSubject {
        issuer: MACHINES.to_owned(),
        sub: "lab".to_owned(),
    }];
    let app = router(svc, config(scopeless)).await;
    let claims = json!({ "sub": "lab", "client_id": "lab", "scope": "openid" });
    let (status, body) = list(&app, &bearer(MACHINES, &audience(MACHINES), &claims)).await;
    assert_eq!(status, StatusCode::OK, "the listed subject reads: {body}");
    let other = bearer(OTHER_MACHINES, &audience(OTHER_MACHINES), &claims);
    let (status, body) = list(&app, &other).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "another issuer's lab: {body}"
    );
}
