// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one credential path to the terminology server: the OAuth 2.0 client
//! credentials grant (RFC 6749 §4.4) under `[terminology.server.oauth]`,
//! driven by `wiremock` — an authorization server and a FHIR R4B terminology
//! server in one process, no network.
//!
//! No openEHR spec governs terminology-server authentication — our own
//! design/extension (`BASE/docs/architecture_overview/master12-terminology.adoc`
//! models the backend only as an external "terminology query server").

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use ferroehr::config::client_credentials::{ClientAuthMethod, ClientCredentialsConfig};
use ferroehr::config::secret::{Secret, SecretUrl};
use ferroehr::service::FerroEhrService;
use ferroehr::service::terminology::config::TerminologyServerConfig;
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SNOMED: &str = "http://snomed.info/sct";

/// An enabled server at `base`, authenticated by `oauth` when given.
fn server_cfg(base: &str, oauth: Option<ClientCredentialsConfig>) -> TerminologyServerConfig {
    TerminologyServerConfig {
        enabled: true,
        url: SecretUrl::new(base),
        fail_on_error: false,
        request_timeout_ms: 800,
        oauth,
    }
}

/// A service over a throwaway database with the server `cfg` describes
/// installed. The database comes from the shared harness — never a per-test
/// container.
async fn service_with(cfg: TerminologyServerConfig) -> FerroEhrService {
    let server = FhirTerminologyProvider::new(&cfg).expect("build the terminology client");
    let db = testkit::db().await.expect("testkit database");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_terminology_server(Arc::new(server))
}

/// With no terminology server installed — the shipped default — terminology
/// stays on the in-process `openehr-term` bundle and no remote call can happen.
#[tokio::test]
async fn the_disabled_default_calls_nothing() {
    let unreachable = MockServer::start().await;
    assert!(
        !TerminologyServerConfig::default().enabled,
        "the terminology server is off by default"
    );

    // A bare service answers the bundle terminologies exactly as before and
    // never reaches out.
    let db = testkit::db().await.expect("testkit database");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    assert!(
        service
            .has_term("ISO_3166-1", "NL", None)
            .await
            .expect("bundle lookup"),
        "the openEHR bundle still answers with no terminology server"
    );
    let unknown = service
        .get_term("http://snomed.info/sct", "38341003", None, None)
        .await;
    assert!(
        unknown.is_err(),
        "an unknown terminology stays a bundle precondition failure, not a remote call"
    );
    assert_eq!(
        unreachable.received_requests().await.unwrap().len(),
        0,
        "nothing was sent to a server the service does not hold"
    );
}

/// A server with an `oauth` grant obtains a client-credentials token
/// (RFC 6749 §4.4) and sends it as a bearer credential on every FHIR
/// operation; the token is cached, so a second operation costs no second
/// grant.
#[tokio::test]
async fn oauth2_client_credentials_token_is_obtained_and_reused() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        // client_secret_basic (RFC 6749 §2.3.1) — credentials in the header,
        // never the body.
        .and(header_exists("authorization"))
        .and(body_string_contains("grant_type=client_credentials"))
        // The configured scope reaches the token request (RFC 6749 §4.4.2);
        // the matcher stops at the scope name because the form encoder's
        // treatment of `/` and `*` is its own business, not this test's.
        .and(body_string_contains("scope=system"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-abc",
            "token_type": "bearer",
            "expires_in": 3600
        })))
        .mount(&idp)
        .await;

    let ts = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$lookup"))
        .and(header("authorization", "Bearer tok-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "Hypertensive disorder"}]
        })))
        .mount(&ts)
        .await;

    let grant = ClientCredentialsConfig {
        token_url: Some(SecretUrl::new(format!("{}/token", idp.uri()))),
        client_id: "ferroehr-cdr".to_owned(),
        client_secret: Some(Secret::new("s3cret")),
        client_auth: ClientAuthMethod::ClientSecretBasic,
        scope: Some("system/*.read".to_owned()),
        ..ClientCredentialsConfig::default()
    };
    let service = service_with(server_cfg(&ts.uri(), Some(grant))).await;

    for _ in 0..2 {
        service
            .get_term(SNOMED, "38341003", None, None)
            .await
            .expect("an authenticated lookup succeeds");
    }
    assert_eq!(
        ts.received_requests().await.unwrap().len(),
        2,
        "both lookups carried the bearer credential (an unauthenticated one would not match)"
    );
    assert_eq!(
        idp.received_requests().await.unwrap().len(),
        1,
        "the token is cached — a second operation must not re-run the grant"
    );
}

/// A token endpoint that refuses the grant surfaces as a typed error
/// — never a silent unauthenticated request.
#[tokio::test]
async fn a_refused_grant_is_a_typed_error_not_an_anonymous_request() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "invalid_client"
        })))
        .mount(&idp)
        .await;

    let ts = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "should never be reached"}]
        })))
        .mount(&ts)
        .await;

    let grant = ClientCredentialsConfig {
        token_url: Some(SecretUrl::new(format!("{}/token", idp.uri()))),
        client_id: "ferroehr-cdr".to_owned(),
        client_secret: Some(Secret::new("wrong")),
        client_auth: ClientAuthMethod::ClientSecretBasic,
        scope: None,
        ..ClientCredentialsConfig::default()
    };
    let service = service_with(server_cfg(&ts.uri(), Some(grant))).await;

    let err = service
        .get_term(SNOMED, "38341003", None, None)
        .await
        .expect_err("a refused grant must fail the call");
    // The refused grant is a TYPED failure — never an anonymous retry — while
    // the operator detail (which client, and the authorization server's own
    // error) stays on the trace record, off the wire body.
    assert_eq!(
        err.status,
        ferroehr::service::status::CallStatusType::Exception,
        "got {err:?}"
    );
    assert!(
        !err.message.contains("client-credentials grant failed")
            && !err.message.contains("ferroehr-cdr"),
        "the deployment's credential configuration must not reach the wire body, got: {}",
        err.message
    );
    assert_eq!(
        ts.received_requests().await.unwrap().len(),
        0,
        "no request reaches the terminology server without a token"
    );
}

/// `client_secret_post` puts the credentials in the token-request body
/// instead of the `Authorization` header (RFC 6749 §2.3.1).
#[tokio::test]
async fn client_secret_post_sends_credentials_in_the_body() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("client_id=ferroehr-cdr"))
        .and(body_string_contains("client_secret=s3cret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-post",
            "token_type": "bearer",
            "expires_in": 3600
        })))
        .mount(&idp)
        .await;

    let ts = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer tok-post"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "ok"}]
        })))
        .mount(&ts)
        .await;

    let grant = ClientCredentialsConfig {
        token_url: Some(SecretUrl::new(format!("{}/token", idp.uri()))),
        client_id: "ferroehr-cdr".to_owned(),
        client_secret: Some(Secret::new("s3cret")),
        client_auth: ClientAuthMethod::ClientSecretPost,
        scope: None,
        ..ClientCredentialsConfig::default()
    };
    let service = service_with(server_cfg(&ts.uri(), Some(grant))).await;
    service
        .get_term(SNOMED, "38341003", None, None)
        .await
        .expect("the post-form grant authenticates the lookup");
    assert_eq!(idp.received_requests().await.unwrap().len(), 1);
}
