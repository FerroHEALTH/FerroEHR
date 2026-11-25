// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party binding across two issuers and the two binding markers (#5011),
//! driven through the assembled router over a real `FerroEhrService` on a real
//! `PostgreSQL` 18, with the local party source.
//!
//! Each issuer binds on `sub` (OpenID Connect Core 1.0 §5.7) under its own
//! `identifier_issuer` and audience, so the same identifier value under one
//! issuer never reaches the other's party. A login whose caller kind needs
//! the other marker than its party carries is refused with `403`
//! (`refused-wrong-marker`). A GUID key stored in upper case binds its claim
//! in lower case. Every bound access record names the declared healthcare
//! provider. No openEHR spec governs resolving a login to a party: our own
//! design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::binding::ProviderId;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::party::config::PartySection;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common;
use crate::party_binding_http::{
    audited_service, list, professional, professional_of, refused, service, token, vo,
};

const ENTRA: &str = "https://login.a.example";
const ENTRA_AUDIENCE: &str = "ferroehr-a";
/// The `identifier_issuer` of the first issuer's `[auth.issuer.<name>.party]` table.
const ENTRA_LOGIN: &str = "https://login.a.example/users";
const KEYCLOAK: &str = "https://login.b.example";
const KEYCLOAK_AUDIENCE: &str = "ferroehr-b";
/// The `identifier_issuer` of the second issuer's `[auth.issuer.<name>.party]` table.
const KEYCLOAK_LOGIN: &str = "https://login.b.example/users";
/// The one identifier value both issuers issue, to different persons.
const SHARED: &str = "6c7d8e9f-0a1b-4c2d-8e3f-4a5b6c7d8e9f";

/// A `[auth.issuer.<name>.party]` table binding the logins of `issuer` under
/// `identifier_issuer`.
fn party(issuer: &str, identifier_issuer: &str) -> PartySection {
    PartySection {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer),
        identifier_issuer: identifier_issuer.to_owned(),
        identifier_type: "guid".to_owned(),
        ..PartySection::default()
    }
}

/// The configuration trusting two issuers, each bound on `sub`.
fn two_issuers() -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: common::bound(
            vec![
                IssuerEntry {
                    jwks_file: common::jwks_file().display().to_string(),
                    ..common::entry(ENTRA, ENTRA_AUDIENCE)
                },
                IssuerEntry {
                    jwks_file: common::jwks_file().display().to_string(),
                    ..common::entry(KEYCLOAK, KEYCLOAK_AUDIENCE)
                },
            ],
            vec![party(ENTRA, ENTRA_LOGIN), party(KEYCLOAK, KEYCLOAK_LOGIN)],
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

/// The router over `svc` under `config`, looking every caller up in the local
/// party source.
async fn router(svc: Arc<FerroEhrService>, config: AppConfig) -> Router {
    let source = Arc::new(svc.party_source("demographic"));
    common::open_configured_key_spaces(&config).await;
    let authenticator = Authenticator::new(
        &config,
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

/// A user's token of the first issuer whose `sub` is `sub`.
fn entra(sub: &str) -> String {
    bearer(ENTRA, ENTRA_AUDIENCE, &json!({ "sub": sub }))
}

/// A user's token of the second issuer whose `sub` is `sub`.
fn keycloak(sub: &str) -> String {
    bearer(KEYCLOAK, KEYCLOAK_AUDIENCE, &json!({ "sub": sub }))
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
}

#[tokio::test]
async fn an_upper_case_guid_key_binds_its_lower_case_claim() {
    let (_db, svc) = service().await;
    // NOTE: RFC 9562 §4: a UUID is case-insensitive on input and output
    // in lower case, so a key typed `guid` matches its claim in any case.
    let entra_key = "7E8F9A0B-1C2D-4E3F-9A4B-5C6D7E8F9A0B";
    let keycloak_key = "8F9A0B1C-2D3E-4F4A-8B5C-6D7E8F9A0B1C";
    professional_of(&svc, ENTRA_LOGIN, entra_key, "532").await;
    professional_of(&svc, KEYCLOAK_LOGIN, keycloak_key, "532").await;
    let app = router(Arc::clone(&svc), two_issuers()).await;

    let (status, body) = list(&app, &entra(&entra_key.to_lowercase())).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a lower-case sub binds the upper-case key: {body}"
    );
    let (status, body) = list(&app, &keycloak(&keycloak_key.to_lowercase())).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a lower-case sub binds the upper-case key: {body}"
    );
}

/// The issuer of the healthcare provider identifiers the tests declare.
const PROVIDER_ISSUER: &str = "urn:oid:2.16.528.1.1007.3.3";

/// Creates an EHR as `credential`, asserting it is created.
async fn create_ehr(app: &Router, credential: &str) {
    let (status, body) = common::send_body(
        app,
        common::req("POST", format!("{}/ehr", common::BASE))
            .auth(credential)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// The `(issuer, id)` of the provider on the `bound` access records of the
/// `PERSON` `person`, waiting for `count`.
async fn bound_providers(
    pool: &sqlx::PgPool,
    person: &str,
    count: usize,
) -> Vec<(Option<String>, Option<String>)> {
    for _ in 0..50 {
        let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT provider ->> 'issuer', provider ->> 'id' FROM audit.access_trail \
             WHERE binding = 'bound' AND party ->> 'id' = $1 ORDER BY recorded_at",
        )
        .bind(person)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} bound records of {person} were stored");
}

/// A deployment acting for one healthcare provider names it on every bound
/// record (EHDS Annex II 3.2(a), "identification of the healthcare
/// provider"; the record's fields are our own design).
#[tokio::test]
async fn a_bound_record_names_the_declared_provider() {
    let (db, _audit, svc) = audited_service().await;
    let key = "9a0b1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d";
    let (uid, _) = professional(&svc, key, "532").await;
    let mut config = crate::party_binding_http::config();
    config.party_binding.provider = Some(ProviderId {
        issuer: PROVIDER_ISSUER.to_owned(),
        id: "90000010".to_owned(),
    });
    let app = router(Arc::clone(&svc), config).await;

    create_ehr(&app, &token(key, &json!({}))).await;
    let person = vo(&uid).0.to_string();
    assert_eq!(
        bound_providers(&db.pool(), &person, 1).await,
        vec![(
            Some(PROVIDER_ISSUER.to_owned()),
            Some("90000010".to_owned())
        )]
    );
}

#[tokio::test]
async fn a_representative_login_on_a_professional_row_is_refused_as_the_wrong_marker() {
    let (_db, svc) = service().await;
    let key = "4c5d6e7f-8a9b-4c0d-9e1f-3a4b5c6d7e8f";
    professional(&svc, key, "532").await;
    let app = router(Arc::clone(&svc), crate::party_binding_http::config()).await;

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
            &json!({ "fhirUser": "RelatedPerson/r-1",
                     "patient": "5d6e7f8a-9b0c-4d1e-8f2a-4b5c6d7e8f9a" }),
        ),
        "refused-wrong-marker",
    )
    .await;
}

/// An EHR export's access-log entry names the request it answered and the
/// bound accessor, its `PERSON` and the declared provider, as every other
/// access entry does (EHDS Annex II 3.2, #5638).
#[tokio::test]
async fn an_export_entry_names_the_request_and_the_bound_accessor() {
    let (db, _audit, svc) = audited_service().await;
    let key = "0b1c2d3e-4f5a-4b6c-8d7e-9f0a1b2c3d4e";
    let (uid, _) = professional(&svc, key, "532").await;
    let mut config = crate::party_binding_http::config();
    config.party_binding.provider = Some(ProviderId {
        issuer: PROVIDER_ISSUER.to_owned(),
        id: "90000010".to_owned(),
    });
    let app = router(Arc::clone(&svc), config).await;
    let credential = token(key, &json!({}));
    let (status, body) = common::send_body(
        &app,
        common::req("POST", format!("{}/ehr", common::BASE))
            .auth(&credential)
            .prefer_representation()
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("an EHR")["ehr_id"]["value"]
        .as_str()
        .expect("an EHR id")
        .to_owned();
    let (status, headers, body) = common::send(
        &app,
        common::req("GET", format!("{}/message/export/{ehr}", common::EXT))
            .auth(&credential)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("the response names its request")
        .to_owned();
    let exported = crate::access_log_http::entries(&db.pool(), 1, |stored| {
        stored.ehr_id.as_deref() == Some(ehr.as_str()) && stored.action.as_deref() == Some("export")
    })
    .await;
    assert_eq!(exported.len(), 1, "the export's one entry");
    assert_eq!(exported[0].request_id.as_deref(), Some(request_id.as_str()));
    let entry: Value = serde_json::from_str(&exported[0].content).expect("an entry");
    let person = vo(&uid).0.to_string();
    let accessor = &entry["accessor"];
    assert_eq!(
        accessor["party"]["external_ref"]["id"]["value"], person,
        "the bound PERSON: {entry}"
    );
    assert_eq!(
        accessor["provider"]["identifiers"][0]["id"], "90000010",
        "the provider: {entry}"
    );
    assert_eq!(
        entry["channel"]["request_id"],
        request_id.as_str(),
        "{entry}"
    );
}
