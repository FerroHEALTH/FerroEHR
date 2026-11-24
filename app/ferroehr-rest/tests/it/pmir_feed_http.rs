// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-93 feed route `POST /pmir/feed` through the assembled router
//! (#5693): only the client `[pix]` trusts may post, by its access token
//! (PMIR 1.6.0 §2:3.93.5, IUA), a message is answered with the Mobile
//! Patient Identity Feed Response once it is stored (§2:3.93.4.2.2), a
//! redelivery is answered again, and a body the PMIR profiles refuse is a
//! `400` with its `OperationOutcome`. Who may post is our own design.

use axum::Router;
use ferroehr::service::demographic::identity_feed::config::PixConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use fhir_types::r4::identifier::Identifier;
use fhir_types::r4::patient::Patient;
use http::{StatusCode, header};
use ihe_iti::pmir::server::notification::{self, Addressing, Notice};
use serde_json::json;
use url::Url;

use crate::common;

/// The deployment's authorization server, which issues the Registry's tokens.
const MACHINES: &str = "https://as.example";
const AUDIENCE: &str = "ferroehr";
/// The Registry's client.
const REGISTRY_CLIENT: &str = "ferropix";
/// The feed route's URL under the public root.
const FEED: &str = "https://cdr.example.com/pmir/feed";

fn config() -> AppConfig {
    let machines = IssuerEntry {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(MACHINES),
        audience: vec![AUDIENCE.to_owned()],
        jwks_file: common::jwks_file().display().to_string(),
        profile: ferrohealth_smart::profile::ProfileKeys {
            // NOTE: the test key signs RFC 9068 `at+jwt` tokens.
            token_types: Some(vec!["at+jwt".to_owned()]),
            ..ferrohealth_smart::profile::ProfileKeys::default()
        },
        ..IssuerEntry::default()
    };
    let mut auth = common::auth(AuthSection {
        issuer: common::named(vec![machines]),
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        server: common::server(),
        auth,
        pix: Some(std::sync::Arc::new(PixConfig {
            feed_issuer: MACHINES.to_owned(),
            feed_client_id: REGISTRY_CLIENT.to_owned(),
            ..PixConfig::default()
        })),
        ..AppConfig::default()
    };
    config.public_root = Some(
        ferroehr::config::server::PublicRoot::new("https://cdr.example.com", "")
            .expect("a public root"),
    );
    config
}

async fn app() -> (testkit::TestDb, Router) {
    let (db, service) = crate::policy_fixture::granting_service().await;
    (db, common::router_with(config(), service).await)
}

fn client_token(client_id: &str) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({ "iss": MACHINES, "aud": AUDIENCE, "exp": exp,
                            "sub": client_id, "client_id": client_id, "scope": "system/Patient.r" }))
}

/// An ITI-93 message `id` announcing one Patient, as the Registry writes it.
fn message(id: &str) -> Vec<u8> {
    let registry = Url::parse("https://pix.example/fhir").expect("a URL");
    let destination = Url::parse(FEED).expect("a URL");
    let patient = Patient {
        id: Some("0190a1bc-3a1e-72e4-b7c5-f78a24c01308".to_owned()),
        identifier: vec![Identifier {
            system: Some("urn:oid:2.999.4.1".into()),
            value: Some("NAT-1".into()),
            ..Identifier::default()
        }],
        active: Some(true.into()),
        ..Patient::default()
    };
    let bundle = notification::message(
        &[Notice::Updated(Box::new(patient))],
        &Addressing {
            message_id: id,
            history_id: "0190a1bc-3a1e-72e4-b7c5-f78a24c01309",
            registry: &registry,
            destination: &destination,
        },
    );
    serde_json::to_vec(&bundle).expect("the message writes")
}

async fn post(
    app: &Router,
    token: Option<&str>,
    body: Vec<u8>,
) -> (StatusCode, http::HeaderMap, String) {
    let mut request = common::req("POST", "/pmir/feed")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(body);
    if let Some(token) = token {
        request = request.auth(token);
    }
    common::send(app, request.build()).await
}

#[tokio::test]
async fn only_the_trusted_registry_client_may_post() {
    let (_db, app) = app().await;
    let (status, headers, _) = post(&app, None, message("m-0")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        headers
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Bearer")
    );
    let other = client_token("someone-else");
    let (status, _, _) = post(&app, Some(&other), message("m-0")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a client [pix] does not name is refused"
    );
}

#[tokio::test]
async fn a_message_is_answered_once_stored_and_again_when_redelivered() {
    let (_db, app) = app().await;
    let token = client_token(REGISTRY_CLIENT);
    for _ in 0..2 {
        let (status, headers, body) = post(&app, Some(&token), message("m-1")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/fhir+json")
        );
        let response: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        assert_eq!(response["type"], "message");
        assert_eq!(
            response["entry"][0]["resource"]["response"]["identifier"],
            "m-1"
        );
        assert_eq!(response["entry"][0]["resource"]["response"]["code"], "ok");
    }
}

#[tokio::test]
async fn a_body_the_profiles_refuse_is_a_400_with_its_outcome() {
    let (_db, app) = app().await;
    let token = client_token(REGISTRY_CLIENT);
    let (status, _, body) = post(
        &app,
        Some(&token),
        br#"{"resourceType":"Patient"}"#.to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let outcome: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    assert_eq!(outcome["resourceType"], "OperationOutcome");
}
