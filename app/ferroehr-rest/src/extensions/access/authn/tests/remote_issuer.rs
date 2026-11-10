// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The issuer's key set, read over its `jwks_uri`: the boot read, the
//! refetch on an unknown key, and the failures of each. No openEHR spec
//! governs auth transport hardening — the crate's design, held here at
//! FerroEHR's seam.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

use super::*;

const JWKS_PATH: &str = "/jwks";

/// The OAuth 2.0 metadata of the issuer `server` runs (RFC 8414 §3).
fn metadata(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}{JWKS_PATH}"),
    })
}

/// The committed test key set, its key named `kid`.
fn jwks(kid: &str) -> Value {
    let mut set: Value = serde_json::from_str(include_str!(
        "../../../../../../../docs/conformance/party/smart/jwks.json"
    ))
    .expect("the committed key set");
    set["keys"][0]["kid"] = json!(kid);
    set
}

/// An `[auth]` section trusting the issuer `server` runs.
fn section(
    issuer: &str,
    refresh_seconds: u64,
    timeout_ms: u64,
) -> ferroehr::config::auth_section::AuthConfig {
    test_support::trusting(ferrohealth_smart::config::IssuerEntry {
        jwks_refresh_interval: ferrohealth_config::units::Duration::from_secs(refresh_seconds),
        timeout: ferrohealth_config::units::Duration::from_millis(timeout_ms),
        ..test_support::entry(issuer)
    })
}

async fn authenticator(
    section: ferroehr::config::auth_section::AuthConfig,
) -> Result<Arc<Authenticator>, String> {
    Authenticator::new(
        &AppConfig {
            auth: section,
            ..AppConfig::default()
        },
        None,
        None,
        memory(),
    )
    .await
}

async fn mount_metadata(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(metadata(&server.uri())))
        .mount(server)
        .await;
}

/// A token of `issuer` signed with the test key under `kid`.
fn token_under(issuer: &str, kid: &str) -> Parts {
    let mut claims = test_support::claims("alice", "user/composition-*.r");
    claims["iss"] = json!(issuer);
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.typ = Some("at+jwt".to_owned());
    header.kid = Some(kid.to_owned());
    let token = jsonwebtoken::encode(
        &header,
        &claims,
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
            "../../../../../../../docs/conformance/party/smart/cnf-smart-test.key.pem"
        ))
        .expect("key"),
    )
    .expect("encode");
    headers(&format!("Bearer {token}"))
}

/// Answers the key-set reads in turn: the first with `kid` "boot", the
/// second with a `500`, every later one with `kid` "rotated".
struct Rotation(Arc<AtomicUsize>);

impl Respond for Rotation {
    fn respond(&self, _request: &MockRequest) -> ResponseTemplate {
        match self.0.fetch_add(1, Ordering::SeqCst) {
            0 => ResponseTemplate::new(200).set_body_json(jwks("boot")),
            1 => ResponseTemplate::new(500),
            _ => ResponseTemplate::new(200).set_body_json(jwks("rotated")),
        }
    }
}

/// A fetched key set that does not parse refuses the start with its
/// failure class and position, never with the parser's message, which
/// quotes the offending value.
#[tokio::test]
async fn unparsable_fetched_jwks_does_not_quote_its_input() {
    let server = MockServer::start().await;
    mount_metadata(&server).await;
    Mock::given(method("GET"))
        .and(path(JWKS_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "keys": "s3cr3t-key-material" })),
        )
        .mount(&server)
        .await;
    let Err(message) = authenticator(section(&server.uri(), 60, 5_000)).await else {
        panic!("the set does not parse");
    };
    assert!(!message.contains("s3cr3t"), "{message}");
    assert!(message.contains("line 1"), "{message}");
}

/// An issuer whose metadata cannot be read refuses the start rather
/// than the first request, so no request ever answers with its URL.
#[tokio::test]
async fn a_failed_discovery_refuses_the_start() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    assert!(
        authenticator(section(&server.uri(), 60, 5_000))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stalled_issuer_fails_within_the_configured_request_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(metadata(&server.uri()))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;
    let started = Instant::now();
    let refused = authenticator(section(&server.uri(), 60, 300)).await;
    let elapsed = started.elapsed();
    assert!(refused.is_err(), "the timeout must fire");
    // The 300 ms budget, generously slacked; without it the request
    // would park for the mock's 30 s.
    assert!(
        elapsed < Duration::from_secs(5),
        "discovery took {elapsed:?}: the request timeout did not fire"
    );
}

/// A key set that cannot be read again answers a `503` whose body names
/// neither the issuer's URL nor the transport error; a second unknown key
/// inside the refresh interval reads nothing; once the interval passes,
/// the recovered issuer's rotated key validates.
#[tokio::test]
async fn a_failed_key_set_read_is_a_503_backed_off_until_the_issuer_recovers() {
    let server = MockServer::start().await;
    mount_metadata(&server).await;
    let reads = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path(JWKS_PATH))
        .respond_with(Rotation(Arc::clone(&reads)))
        .mount(&server)
        .await;
    let auth = authenticator(section(&server.uri(), 1, 5_000))
        .await
        .expect("the boot read succeeds");
    let issuer = server.uri();

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let err = auth
        .authenticate(&token_under(&issuer, "rotated"))
        .await
        .expect_err("the refetch fails");
    assert!(matches!(err, AuthError::KeyResolution), "got {err:?}");
    let api = err.to_api_error();
    assert_eq!(api.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = api.to_string();
    assert!(
        !body.contains("127.0.0.1"),
        "the body names the URL: {body}"
    );
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "the boot read and one refetch"
    );

    // Still inside the interval since the failed read: nothing is read.
    assert!(
        auth.authenticate(&token_under(&issuer, "rotated"))
            .await
            .is_err()
    );
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "a read escaped inside the refresh interval"
    );

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let principal = auth
        .authenticate(&token_under(&issuer, "rotated"))
        .await
        .expect("the recovered issuer's rotated key validates")
        .principal;
    assert_eq!(principal.subject, "alice");
}
