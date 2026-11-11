// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What each local caller kind may do (#5100), driven through the assembled
//! router over a real `FerroEhrService` on a real `PostgreSQL` 18: a system
//! caller holds `system/` scopes only and commits only as a declared client,
//! a guest is admitted only where its issuer's `[[auth.party]]` table admits
//! guests, and a delegated person is bound to their own `PERSON`, which
//! covers a Keycloak impersonation (#5167).
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
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::{Delegate, Marker, Preset};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{
    AUDIENCE, ISSUER, LOGIN, audited_service, list, professional, professional_of, refusal_rows,
    refused, service,
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
/// The Keycloak realm whose staff sign in, a `persons` issuer.
const KEYCLOAK_STAFF: &str = "https://keycloak.example/realms/staff";
/// The `identifier_issuer` of that realm's `[[auth.party]]` table.
const KEYCLOAK_LOGIN: &str = "https://keycloak.example/realms/staff/users";
/// The realm's first-party client, named in `azp`.
const VIEWER: &str = "ferroehr-viewer";
/// The Keycloak user id of the administrator the realm declares may
/// impersonate.
const SUPPORT_ADMIN: &str = "0e1f2a3b-4c5d-4e6f-8a7b-9c0d1e2f3a4b";
/// The SMART scopes of a backend client that reads templates and writes EHRs.
const SYSTEM_SCOPES: &str = "system/composition-*.cruds system/template-*.r";

/// The audience of the machines issuer `issuer`: each issuer has its own.
pub(crate) fn audience(issuer: &str) -> String {
    format!("{AUDIENCE}-{}", issuer.trim_start_matches("https://"))
}

/// A `machines` issuer at `issuer` under `profile`.
pub(crate) fn machines(issuer: &str, profile: Preset) -> IssuerEntry {
    IssuerEntry {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer.to_owned()),
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

/// The Keycloak `persons` issuer whose staff log in through [`VIEWER`],
/// declaring [`SUPPORT_ADMIN`] an actor, with `impersonation_marker` set
/// over the profile.
fn keycloak_staff(impersonation_marker: Option<&str>) -> IssuerEntry {
    IssuerEntry {
        kind: Some(IssuerKind::Persons),
        first_party_clients: Some(vec![VIEWER.to_owned()]),
        app_only: Some(vec![Marker::Subjects(vec![
            "service-account-viewer".to_owned(),
        ])]),
        delegates: Some(vec![Delegate::Actor {
            issuer: None,
            sub: SUPPORT_ADMIN.to_owned(),
        }]),
        impersonation_marker: impersonation_marker.map(str::to_owned),
        ..machines(KEYCLOAK_STAFF, Preset::Keycloak)
    }
}

/// The configuration trusting the persons issuer, whose guests the
/// `[[auth.party]]` table admits when `guests`, and the three machines
/// issuers, with `clients` declared.
fn config(guests: bool, clients: Vec<SystemClient>) -> AppConfig {
    config_marked(guests, clients, None)
}

/// [`config`], with the Keycloak staff realm's `impersonation_marker`.
fn config_marked(
    guests: bool,
    clients: Vec<SystemClient>,
    impersonation_marker: Option<&str>,
) -> AppConfig {
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
        issuer: vec![
            persons,
            machines(MACHINES, Preset::Rfc9068),
            machines(ENTRA, Preset::Entra),
            keycloak,
            keycloak_staff(impersonation_marker),
        ],
        party: vec![
            PartySection {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
                identifier_issuer: LOGIN.to_owned(),
                identifier_type: "guid".to_owned(),
                guests,
                ..PartySection::default()
            },
            PartySection {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(KEYCLOAK_STAFF),
                identifier_issuer: KEYCLOAK_LOGIN.to_owned(),
                identifier_type: "guid".to_owned(),
                ..PartySection::default()
            },
        ],
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
pub(crate) fn bearer(issuer: &str, audience: &str, claims: &Value) -> String {
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
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer),
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

/// A Keycloak 26.8.2 token of an impersonation session for `key`: the
/// impersonator in `act` (`sub`, `preferred_username`), and, when
/// `mapped`, the `impersonator` claim of the built-in `IMPERSONATOR_ID` and
/// `IMPERSONATOR_USERNAME` user session note mappers beside it.
fn impersonated(key: &str, impersonator: &str, mapped: bool) -> String {
    let mut claims = json!({
        "sub": key, "azp": VIEWER,
        "act": { "sub": impersonator, "preferred_username": "support" },
    });
    if mapped && let Some(claims) = claims.as_object_mut() {
        claims.insert(
            "impersonator".to_owned(),
            json!({ "id": impersonator, "username": "support" }),
        );
    }
    bearer(KEYCLOAK_STAFF, &audience(KEYCLOAK_STAFF), &claims)
}

/// The `(acting_mode, actor_chain, party id)` of the stored records of
/// `operation` whose principal is `principal`, waiting for `count`.
async fn delegated_rows(
    pool: &sqlx::PgPool,
    operation: &str,
    principal: &str,
    count: usize,
) -> Vec<(Option<String>, Option<Value>, Option<String>)> {
    for _ in 0..50 {
        let rows: Vec<(Option<String>, Option<Value>, Option<String>)> = sqlx::query_as(
            "SELECT acting_mode, actor_chain, party ->> 'id' FROM audit.audit_event \
             WHERE operation = $1 AND principal = $2 ORDER BY recorded_at",
        )
        .bind(operation)
        .bind(principal)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} {operation} records of {principal} were stored");
}

/// Keycloak 26.8.2 names the impersonator of an impersonation session in an
/// RFC 8693 §4.1 `act` claim, "always present and cannot be disabled"
/// (Keycloak 26.8.0 release notes, "Track impersonation across token
/// lifecycle and downstream services"). An impersonator the realm declares
/// in `delegates` acts as a delegate: the request is bound to the
/// impersonated person, commits as that person, and the access record names
/// the impersonator as the actor, whether or not the `impersonator` claim
/// rides along.
#[tokio::test]
async fn a_keycloak_impersonation_is_recorded_with_the_impersonator_as_actor() {
    let (db, _audit, svc) = audited_service().await;
    let key = "9e0f1a2b-3c4d-4e5f-8a6b-7c8d9e0f1a2b";
    let (person_uid, _) = professional_of(&svc, KEYCLOAK_LOGIN, key, "532").await;
    let person = person_uid
        .split("::")
        .next()
        .expect("the PERSON id")
        .to_owned();
    let app = router(Arc::clone(&svc), config(false, Vec::new())).await;
    for mapped in [false, true] {
        let token = impersonated(key, SUPPORT_ADMIN, mapped);
        let (status, body) = create_ehr(&app, &token).await;
        assert_eq!(status, StatusCode::CREATED, "mapped {mapped}: {body}");
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
            .auth(&token)
            .accept("application/json")
            .build(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{version}");
        let committer = &version["commit_audit"]["committer"];
        assert_eq!(
            committer["external_ref"]["id"]["value"], person,
            "{committer}"
        );
        assert_eq!(committer["identifiers"][0]["id"], key, "{committer}");
    }
    let rows = delegated_rows(&db.pool(), "ehr_create", key, 2).await;
    let actor = json!([format!("{KEYCLOAK_STAFF} {SUPPORT_ADMIN}")]);
    for (mode, chain, party) in rows {
        assert_eq!(mode.as_deref(), Some("delegated"));
        assert_eq!(
            chain.as_ref(),
            Some(&actor),
            "the impersonator is the actor"
        );
        assert_eq!(
            party.as_deref(),
            Some(person.as_str()),
            "bound to the person"
        );
    }
}

/// An impersonation by a user the realm does not declare in `delegates` is
/// refused as any undeclared actor is, and a realm that names Keycloak's
/// `impersonator` claim in `impersonation_marker` refuses every
/// impersonation that carries it, a declared impersonator's included.
#[tokio::test]
async fn an_undeclared_or_marked_keycloak_impersonation_is_refused() {
    let (_db, svc) = service().await;
    let key = "a0b1c2d3-e4f5-4a6b-9c7d-8e9f0a1b2c3d";
    professional_of(&svc, KEYCLOAK_LOGIN, key, "532").await;
    let app = router(Arc::clone(&svc), config(false, Vec::new())).await;
    let stranger = "1f2a3b4c-5d6e-4f7a-8b9c-0d1e2f3a4b5c";
    for mapped in [false, true] {
        let (status, body) = list(&app, &impersonated(key, stranger, mapped)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "mapped {mapped}: {body}");
    }
    let marked = router(svc, config_marked(false, Vec::new(), Some("impersonator"))).await;
    let (status, body) = list(&marked, &impersonated(key, SUPPORT_ADMIN, true)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, body) = list(&marked, &impersonated(key, SUPPORT_ADMIN, false)).await;
    assert_eq!(status, StatusCode::OK, "without the marker claim: {body}");
}

/// Someone acting for a patient (RFC 8693 §4.1 `act` on a `patients` token)
/// is refused `403` where the issuer sets `act_represents_patient`, since
/// FerroEHR binds the representative to no party, and such a token is refused
/// `401` by `ferrohealth-smart` on an issuer that does not set it.
#[tokio::test]
async fn a_token_acting_for_a_patient_is_refused() {
    const PATIENTS: &str = "https://patients.example";
    let (_db, svc) = service().await;
    for (represents, expected) in [
        (Some(true), StatusCode::FORBIDDEN),
        (None, StatusCode::UNAUTHORIZED),
    ] {
        let mut config = config(false, Vec::new());
        config.auth.shared.issuer.push(IssuerEntry {
            kind: Some(IssuerKind::Patients),
            jwks_file: common::jwks_file().display().to_string(),
            act_represents_patient: represents,
            ..common::entry(PATIENTS, AUDIENCE)
        });
        let app = router(Arc::clone(&svc), config).await;
        let token = bearer(
            PATIENTS,
            AUDIENCE,
            &json!({ "sub": "patient-1", "act": { "sub": "carer-7" }, "scope": "patient/template-*.r" }),
        );
        let (status, body) = list(&app, &token).await;
        assert_eq!(
            status, expected,
            "act_represents_patient {represents:?}: {body}"
        );
        if represents.is_some() {
            assert!(body.contains("acting for a patient"), "{body}");
        }
    }
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

/// The `(binding, acting_mode, client_id, provider id)` of a stored record.
type Stored = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The `(binding, acting_mode, client_id, provider id)` of the stored records
/// of `operation` whose principal is `principal`, waiting for `count`.
async fn stored_rows(
    pool: &sqlx::PgPool,
    operation: &str,
    principal: &str,
    count: usize,
) -> Vec<Stored> {
    for _ in 0..50 {
        let rows: Vec<Stored> = sqlx::query_as(
            "SELECT binding, acting_mode, client_id, provider ->> 'id' \
                 FROM audit.audit_event WHERE operation = $1 AND principal = $2 \
                 ORDER BY recorded_at",
        )
        .bind(operation)
        .bind(principal)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} {operation} records of {principal} were stored");
}

/// Every access record of a system caller carries `binding = system`, the
/// acting mode `system`, the client and the healthcare provider its
/// `[[authz.system_client]]` table names, under each profile (EHDS Annex II
/// 3.2(a), "identification of the healthcare provider"; the fields are our
/// own design, #5008). The test tokens all carry `client_id`, so under the
/// `entra` and `keycloak` profiles the record names the `azp` client the
/// profile reads, never that stray claim (#5194).
#[tokio::test]
async fn a_system_callers_records_carry_its_client_and_provider() {
    let (db, _audit, svc) = audited_service().await;
    let declared = [MACHINES, ENTRA, KEYCLOAK]
        .into_iter()
        .map(|issuer| SystemClient {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer),
            client_id: "lab".to_owned(),
            provider: Some(ferroehr::config::binding::ProviderId {
                issuer: "urn:oid:2.16.528.1.1007.3.3".to_owned(),
                id: "90000001".to_owned(),
            }),
            origins: vec![Origin::Relay],
            ..SystemClient::default()
        })
        .collect();
    let app = router(svc, config(false, declared)).await;
    let tokens = app_only(SYSTEM_SCOPES);
    for (profile, credential) in &tokens {
        let (status, body) = create_ehr(&app, credential).await;
        assert_eq!(status, StatusCode::CREATED, "{profile}: {body}");
    }
    for principal in ["lab", "8f1c", SERVICE_ACCOUNT] {
        let rows = stored_rows(&db.pool(), "ehr_create", principal, 1).await;
        assert_eq!(
            rows,
            vec![(
                Some("system".to_owned()),
                Some("system".to_owned()),
                Some("lab".to_owned()),
                Some("90000001".to_owned())
            )],
            "{principal}"
        );
    }
}

/// An RFC 7523 JWT-bearer onward token names the person in `sub` and the
/// middle-tier client in its client claim, with no `act`: the client the
/// issuer lists in `delegates` acts for the person, who is bound and commits
/// as their own `PERSON`, and the record carries the delegated mode and the
/// client (#5008).
#[tokio::test]
async fn a_jwt_bearer_onward_token_commits_as_the_person_and_records_its_client() {
    let (db, _audit, svc) = audited_service().await;
    let key = "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e";
    professional(&svc, key, "532").await;
    let mut config = config(false, Vec::new());
    if let Some(persons) = config.auth.shared.issuer.first_mut() {
        persons.delegates = Some(vec![Delegate::Client("bridge".to_owned())]);
    }
    let app = router(Arc::clone(&svc), config).await;
    let onward = bearer(
        ISSUER,
        AUDIENCE,
        &json!({ "sub": key, "client_id": "bridge" }),
    );
    let (status, body) = create_ehr(&app, &onward).await;
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
        .auth(&onward)
        .accept("application/json")
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["external_ref"]["type"], "PERSON", "{committer}");
    assert_eq!(committer["identifiers"][0]["id"], key, "{committer}");
    let rows = stored_rows(&db.pool(), "ehr_create", key, 1).await;
    assert_eq!(
        rows[0].1.as_deref(),
        Some("delegated"),
        "the delegated mode"
    );
    assert_eq!(rows[0].2.as_deref(), Some("bridge"), "the client");
}
