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
//! another tenant is a guest by `idp`. An `access_without_scope` grant names
//! its issuer, so the same `sub` of another issuer gets nothing. RFC 9068 §2.2
//! and §5 name the person and the app-only token; the rest is our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::{Marker, Preset};
use http::StatusCode;
use serde_json::json;

use crate::caller_kinds_http::{audience, bearer, machines};
use crate::common::{self, BASE};
use crate::party_binding_http::{
    Capture, audited_service, list, professional_of, refused, router_with, service,
};

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
/// The onward client both persons issuers' grants let act for a person, such
/// as `FerroBRIDGE`.
const ONWARD: &str = "bridge";

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
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer),
        identifier_issuer: identifier_issuer.to_owned(),
        identifier_type: "guid".to_owned(),
        ..PartySection::default()
    }
}

/// The configuration trusting one issuer of each local kind and a second
/// machines issuer.
fn config() -> AppConfig {
    let entra = persons(ENTRA, Preset::Entra);
    let keycloak = IssuerEntry {
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
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    }
}

/// The router over `svc` under `config`, binding through the local source,
/// with the clients' standing granted in the CDR: the realm's viewer is first
/// party (the test client is through `router_with`), and the onward client
/// acts for both issuers' persons.
async fn router(svc: Arc<FerroEhrService>, config: AppConfig) -> Router {
    common::grant_standing(&svc, KEYCLOAK, VIEWER, Operation::SignInPersons).await;
    for issuer in [ENTRA, KEYCLOAK] {
        common::grant_standing(&svc, issuer, ONWARD, Operation::ActForPersons).await;
    }
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

/// A person signs in only through a client the CDR grants `sign_in_persons`:
/// a token through another client is refused with `401`, and once an
/// administrator grants that client, the next request is the person's, with
/// no restart; a ceiling that no longer admits the operation refuses it again.
#[tokio::test]
async fn a_person_signs_in_only_through_a_client_the_cdr_grants() {
    let (_db, svc) = service().await;
    let key = "4f3a2b1c-0d9e-4f8a-b7c6-d5e4f3a2b1c0";
    professional_of(&svc, KEYCLOAK_LOGIN, key, "532").await;
    let app = router(Arc::clone(&svc), config()).await;
    let token = bearer(
        KEYCLOAK,
        &audience(KEYCLOAK),
        &json!({ "sub": key, "azp": "portal", "scope": common::FULL_SCOPES }),
    );
    let (status, body) = list(&app, &token).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an ungranted client: {body}"
    );
    common::grant_standing(&svc, KEYCLOAK, "portal", Operation::SignInPersons).await;
    let (status, body) = list(&app, &token).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the granted client signs in: {body}"
    );
    svc.grants().set_ceiling(
        ferroehr::party_binding::grants::operation::GrantCeiling::new([Operation::ActForPersons]),
    );
    let (status, body) = list(&app, &token).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a grant no longer honoured: {body}"
    );
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
    let app = router(svc, config()).await;
    let (status, body) = list(&app, &entra(oid, &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "a member without idp binds: {body}");
    let home = json!({ "idp": "https://sts.windows.example/home-tenant/" });
    refused(&app, &entra(oid, &home), "refused-guest").await;
    refused(&app, &entra(oid, &json!({ "acct": 1 })), "refused-guest").await;
}

/// An `access_without_scope` grant names its issuer: the granted client
/// reads with a token carrying no resource scope, and the same `sub` of
/// another issuer is held to its scopes and refused.
#[tokio::test]
async fn an_access_without_scope_grant_admits_the_subject_of_its_own_issuer_only() {
    let (_db, svc) = service().await;
    common::grant_standing(&svc, MACHINES, "lab", Operation::AccessWithoutScope).await;
    let app = router(svc, config()).await;
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

/// An Entra ID on-behalf-of token (the middle tier's application id in
/// `azp`, Microsoft "On-behalf-of flow") and a Keycloak standard token
/// exchange token (the requesting client in `azp`, Keycloak "Token
/// exchange") each name the person in their key claim and the onward client
/// the CDR grants `act_for_persons`: the request is bound to the person, commits
/// as their `PERSON`, and the record carries the delegated mode and the client.
#[tokio::test]
async fn an_onward_token_of_entra_and_of_keycloak_is_the_persons_with_its_client() {
    let (db, _audit, svc) = audited_service().await;
    let oid = "6d5e4f3a-2b1c-4d0e-9f8a-7b6c5d4e3f2a";
    let sub = "5e4f3a2b-1c0d-4e9f-8a7b-6c5d4e3f2a1b";
    professional_of(&svc, ENTRA_LOGIN, oid, "532").await;
    professional_of(&svc, KEYCLOAK_LOGIN, sub, "532").await;
    let app = router(svc, config()).await;
    let obo = entra(oid, &json!({ "azp": ONWARD }));
    let exchanged = bearer(
        KEYCLOAK,
        &audience(KEYCLOAK),
        &json!({ "sub": sub, "azp": ONWARD }),
    );
    for (case, credential, key, principal) in [
        ("entra on-behalf-of", &obo, oid, "pairwise-subject"),
        ("keycloak standard exchange", &exchanged, sub, sub),
    ] {
        let committer = committer_of_new_ehr(&app, credential).await;
        assert_eq!(
            committer["external_ref"]["type"], "PERSON",
            "{case}: {committer}"
        );
        assert_eq!(
            committer["identifiers"][0]["id"], key,
            "{case}: {committer}"
        );
        let (mode, client) = delegated_record(&db.pool(), principal).await;
        assert_eq!(mode.as_deref(), Some("delegated"), "{case}");
        assert_eq!(client.as_deref(), Some(ONWARD), "{case}");
    }
}
