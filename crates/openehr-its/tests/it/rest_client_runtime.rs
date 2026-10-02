// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::panic_in_result_fn,
    clippy::panic,
    reason = "test assertions/diagnostics/fixtures: Result-returning tests assert and name \
              the unexpected outcome (.claude/rules/testing.md §Test shapes); `allow`, not \
              `expect`, because #[tokio::test] moves each body into an async block, which \
              these lints do not reach uniformly"
)]
//! Contract tests for the client runtime beneath the generated clients: the
//! raw passthrough (`Client::forward`), the credentials provider asked per
//! attempt, and per-call options (a deadline, extra headers).
//!
//! No openEHR spec governs these mechanisms; the general statuses they leave
//! unclassified (`401`, `403`, `5xx`) are the ITS-REST docs text
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
//! §HTTP status codes). Every mock carries `.expect(n)`, verified when the
//! server drops.

use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use http::{HeaderValue, Method, StatusCode};
use openehr_its::rest::client::{
    CallOptions, Client, ClientError, Credentials, CredentialsError, CredentialsProvider,
    InvalidCredentials, Request, ReqwestTransport, RetryPolicy, TransportError,
};
use openehr_its::rest::generated::ehr;
use wiremock::matchers::{body_bytes, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A test's plumbing error: every fallible step propagates with `?`.
type TestResult = Result<(), Box<dyn Error>>;

/// The EHR id every request names.
const EHR_ID: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// A client for the mock server's root, over the `reqwest` engine.
fn client_for(server: &MockServer) -> Result<Client<ReqwestTransport>, Box<dyn Error>> {
    let transport = ReqwestTransport::with_timeout(Duration::from_secs(10))?;
    Ok(Client::new(transport, server.uri().parse()?)?)
}

/// `ehr_get_by_id` parameters for the fixture EHR with `accept`.
fn get_ehr(accept: Option<&str>) -> ehr::EhrGetByIdParams {
    ehr::EhrGetByIdParams {
        ehr_id: EHR_ID.to_owned(),
        accept: accept.map(str::to_owned),
    }
}

// ── raw passthrough ─────────────────────────────────────────────────────────

/// A forwarded request reaches the service byte for byte (body, query,
/// headers), and the answer comes back with its status, `Location`, `ETag`
/// and body bytes untouched.
#[tokio::test]
async fn forward_passes_bytes_through_unchanged() -> TestResult {
    let server = MockServer::start().await;
    let sent = b"{ \"_type\":\"COMPOSITION\",  \"name\" : {\"value\":\"x\"}\n}".to_vec();
    let answered = b"{\"uid\" :  \"8849182c::cdr.example.org::1\"}".to_vec();
    let location = format!(
        "https://upstream.example.org/openehr/v1/ehr/{EHR_ID}/composition/8849182c::cdr.example.org::1"
    );
    Mock::given(method("POST"))
        .and(path(format!("/ehr/{EHR_ID}/composition")))
        .and(body_bytes(sent.clone()))
        .and(header("content-type", "application/json"))
        .and(header("prefer", "return=identifier"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("Location", location.as_str())
                .insert_header("ETag", "\"8849182c::cdr.example.org::1\"")
                .set_body_raw(answered.clone(), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let mut request = Request::new(Method::POST, format!("/ehr/{EHR_ID}/composition"));
    request.raw_query("templateId=Vital%20Signs&x");
    request
        .headers_mut()
        .insert("prefer", HeaderValue::from_static("return=identifier"));
    request.raw_body(sent, Some(HeaderValue::from_static("application/json")));
    let answer = client.forward(request).await?;
    assert_eq!(answer.status(), StatusCode::CREATED);
    assert_eq!(answer.header("Location"), Some(location));
    assert_eq!(
        answer.header("ETag").as_deref(),
        Some("\"8849182c::cdr.example.org::1\"")
    );
    assert_eq!(answer.body(), answered.as_slice());
    let received = server.received_requests().await.unwrap_or_default();
    let query = received
        .first()
        .and_then(|r| r.url.query().map(str::to_owned));
    assert_eq!(query.as_deref(), Some("templateId=Vital%20Signs&x"));
    Ok(())
}

/// A `500` and a `401` come back as answers from `forward`, sent once each:
/// nothing is classified and nothing is retried.
#[tokio::test]
async fn forward_returns_every_status_as_an_answer() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(500).set_body_raw("upstream down", "text/plain"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ehr"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Bearer realm=\"cdr\""),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_retry(RetryPolicy {
        max_attempts: 5,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(1),
    });
    let failed = client
        .forward(Request::new(Method::GET, format!("/ehr/{EHR_ID}")))
        .await?;
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failed.into_body(), b"upstream down");
    let refused = client
        .forward(Request::new(Method::GET, "/ehr".to_owned()))
        .await?;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        refused.header("WWW-Authenticate").as_deref(),
        Some("Bearer realm=\"cdr\"")
    );
    Ok(())
}

/// Every public `ReqwestTransport` constructor, built with defaults.
fn every_transport() -> Result<Vec<(&'static str, ReqwestTransport)>, Box<dyn Error>> {
    Ok(vec![
        ("new", ReqwestTransport::new(reqwest::Client::builder())?),
        (
            "with_builder_timeout",
            ReqwestTransport::with_builder_timeout(
                reqwest::Client::builder(),
                Duration::from_secs(10),
            )?,
        ),
        (
            "with_timeout",
            ReqwestTransport::with_timeout(Duration::from_secs(10))?,
        ),
    ])
}

/// A `303` and a `307` come back from `forward` as received, over every
/// transport constructor, and the `Location` they name is never requested:
/// a followed redirect would re-send the request, credentials and all, to a
/// host the caller did not name.
#[tokio::test]
async fn forward_returns_a_redirect_as_received_over_every_constructor() -> TestResult {
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&elsewhere)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&elsewhere)
        .await;
    let target = format!("{}/ehr/{EHR_ID}", elsewhere.uri());
    for (name, transport) in every_transport()? {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/ehr/{EHR_ID}")))
            .respond_with(ResponseTemplate::new(303).insert_header("Location", target.as_str()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/ehr"))
            .respond_with(ResponseTemplate::new(307).insert_header("Location", target.as_str()))
            .expect(1)
            .mount(&server)
            .await;
        let client = Client::new(transport, server.uri().parse()?)?
            .with_credentials(Credentials::bearer("onward-token".to_owned()));
        let seen = client
            .forward(Request::new(Method::GET, format!("/ehr/{EHR_ID}")))
            .await?;
        assert_eq!(seen.status(), StatusCode::SEE_OTHER, "{name}");
        assert_eq!(
            seen.header("Location").as_deref(),
            Some(target.as_str()),
            "{name}"
        );
        let mut post = Request::new(Method::POST, "/ehr".to_owned());
        post.raw_body(
            b"{}".to_vec(),
            Some(HeaderValue::from_static("application/json")),
        );
        let seen = client.forward(post).await?;
        assert_eq!(seen.status(), StatusCode::TEMPORARY_REDIRECT, "{name}");
        assert_eq!(
            seen.header("Location").as_deref(),
            Some(target.as_str()),
            "{name}"
        );
    }
    let followed = elsewhere
        .received_requests()
        .await
        .ok_or("request recording is off")?;
    assert!(followed.is_empty(), "a redirect was followed: {followed:?}");
    Ok(())
}

/// A credential that breaks RFC 7617 §2 fails the call before anything is
/// sent, and the cause downcasts to the crate's own `InvalidCredentials`.
#[tokio::test]
async fn invalid_credentials_fail_before_sending() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_credentials(Credentials::basic("al:ice", "pw"));
    let failed = client
        .forward(Request::new(Method::GET, format!("/ehr/{EHR_ID}")))
        .await;
    match failed {
        Err(error @ ClientError::InvalidCredentials { .. }) => {
            let cause = error
                .source()
                .and_then(|source| source.downcast_ref::<InvalidCredentials>());
            assert!(
                matches!(cause, Some(InvalidCredentials::ColonInUserId)),
                "{cause:?}"
            );
        }
        other => panic!("expected InvalidCredentials, got {other:?}"),
    }
    Ok(())
}

// ── credentials provider ────────────────────────────────────────────────────

/// A provider that hands out `t1`, `t2`, … and counts what it was told.
#[derive(Debug, Default)]
struct RotatingTokens {
    asked: AtomicUsize,
    refused: AtomicUsize,
}

#[async_trait::async_trait]
impl CredentialsProvider for RotatingTokens {
    async fn credentials(&self) -> Result<Credentials, CredentialsError> {
        let n = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Credentials::bearer(format!("t{n}")))
    }

    fn refused(&self) {
        self.refused.fetch_add(1, Ordering::SeqCst);
    }
}

/// The provider is asked once per attempt, so the retry after a failure
/// carries the next token.
#[tokio::test]
async fn the_provider_is_asked_per_attempt_and_a_retry_sends_the_new_token() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("authorization", "Bearer t2"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let provider = std::sync::Arc::new(RotatingTokens::default());
    let client = client_for(&server)?
        .with_retry(RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
        })
        .with_credentials_provider(std::sync::Arc::clone(&provider));
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(None))
        .await?;
    assert!(matches!(
        outcome,
        ehr::client::EhrGetByIdOutcome::NotFound { .. }
    ));
    assert_eq!(provider.asked.load(Ordering::SeqCst), 2);
    assert_eq!(provider.refused.load(Ordering::SeqCst), 0);
    Ok(())
}

/// A `401` tells the provider its credential was refused, and the next
/// request carries the token it rotated to.
#[tokio::test]
async fn a_401_reports_the_refusal_and_the_next_request_rotates() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("authorization", "Bearer t2"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let provider = std::sync::Arc::new(RotatingTokens::default());
    let client = client_for(&server)?.with_credentials_provider(std::sync::Arc::clone(&provider));
    let refused = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(None))
        .await;
    assert!(matches!(refused, Err(ClientError::Unauthorized { .. })));
    assert_eq!(provider.refused.load(Ordering::SeqCst), 1);
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(None))
        .await?;
    assert!(matches!(
        outcome,
        ehr::client::EhrGetByIdOutcome::NotFound { .. }
    ));
    Ok(())
}

/// The token endpoint a provider could not reach.
#[derive(Debug, thiserror::Error)]
#[error("the token endpoint answered 503")]
struct TokenEndpointDown;

/// A provider that never obtains a token.
#[derive(Debug)]
struct Unreachable;

#[async_trait::async_trait]
impl CredentialsProvider for Unreachable {
    async fn credentials(&self) -> Result<Credentials, CredentialsError> {
        Err(CredentialsError::new(TokenEndpointDown))
    }
}

/// A provider failure is a typed error carrying the provider's own error as
/// its cause, and nothing is sent.
#[tokio::test]
async fn a_provider_failure_carries_its_cause_and_sends_nothing() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_credentials_provider(Unreachable);
    let Err(error) = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(None))
        .await
    else {
        panic!("a provider failure must fail the call");
    };
    assert!(
        matches!(error, ClientError::Credentials { .. }),
        "{error:?}"
    );
    let provider_error = error
        .source()
        .ok_or("the error names its provider failure")?;
    assert!(provider_error.downcast_ref::<CredentialsError>().is_some());
    let cause = provider_error
        .source()
        .ok_or("the provider failure names its cause")?;
    assert!(
        cause.downcast_ref::<TokenEndpointDown>().is_some(),
        "the cause downcasts to the provider's own error type: {cause:?}"
    );
    Ok(())
}

/// A client's `Debug` never prints the secret of its static credentials.
#[test]
fn client_debug_redacts_static_credentials() -> TestResult {
    let transport = ReqwestTransport::with_timeout(Duration::from_secs(1))?;
    let client = Client::new(transport, "https://cdr.example.org/openehr/v1".parse()?)?
        .with_credentials(Credentials::basic("alice", "hunter2"));
    let printed = format!("{client:?}");
    assert!(!printed.contains("hunter2"), "{printed}");
    assert!(printed.contains("alice"));
    Ok(())
}

// ── per-call options ────────────────────────────────────────────────────────

/// An option header reaches the service on a generated method, replacing a
/// same-named header the method would send.
#[tokio::test]
async fn option_headers_reach_the_service_and_replace_generated_ones() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("x-request-id", "req-42"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let options = CallOptions::default()
        .with_header("x-request-id", "req-42")?
        .with_header("Accept", "application/json")?;
    let outcome = ehr::client::EhrClient::new(&client)
        .with_options(options)
        .ehr_get_by_id(&get_ehr(Some("application/xml")))
        .await?;
    assert!(matches!(
        outcome,
        ehr::client::EhrGetByIdOutcome::NotFound { .. }
    ));
    let received = server.received_requests().await.unwrap_or_default();
    let accepts: Vec<_> = received
        .first()
        .map(|r| r.headers.get_all("accept").iter().cloned().collect())
        .unwrap_or_default();
    assert_eq!(accepts, [HeaderValue::from_static("application/json")]);
    Ok(())
}

/// A deadline shorter than the service's answer ends the call with the
/// engine's timeout, well before the answer would have arrived.
#[tokio::test]
async fn a_deadline_shorter_than_the_answer_times_out() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ehr"))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_secs(3)))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::EhrCreateParams {
        prefer: None,
        accept: None,
        content_type: None,
        openehr_version: None,
        openehr_audit_details: None,
    };
    let started = Instant::now();
    let result = ehr::client::EhrClient::new(&client)
        .with_options(CallOptions::default().with_timeout(Duration::from_millis(200)))
        .ehr_create(&params, None)
        .await;
    assert!(
        matches!(
            result,
            Err(ClientError::Transport {
                source: TransportError::Timeout { .. },
                ..
            })
        ),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    Ok(())
}

/// A deadline already passed fails the call before anything is sent.
#[tokio::test]
async fn an_elapsed_deadline_fails_before_sending() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let result = ehr::client::EhrClient::new(&client)
        .with_options(CallOptions::default().with_deadline(Instant::now()))
        .ehr_get_by_id(&get_ehr(None))
        .await;
    assert!(
        matches!(result, Err(ClientError::DeadlineElapsed { .. })),
        "{result:?}"
    );
    let mut request = Request::new(Method::GET, format!("/ehr/{EHR_ID}"));
    request.set_deadline(Instant::now());
    assert!(matches!(
        client.forward(request).await,
        Err(ClientError::DeadlineElapsed { .. })
    ));
    Ok(())
}

/// Retries stop at the deadline: a backoff that would end after it is not
/// waited out, and the last failure is returned.
#[tokio::test]
async fn retries_stop_at_the_deadline() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(2..=6)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_retry(RetryPolicy {
        max_attempts: 20,
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_millis(100),
    });
    let started = Instant::now();
    let result = ehr::client::EhrClient::new(&client)
        .with_options(CallOptions::default().with_timeout(Duration::from_millis(450)))
        .ehr_get_by_id(&get_ehr(None))
        .await;
    assert!(
        matches!(
            result,
            Err(ClientError::ServiceFailure {
                status: StatusCode::SERVICE_UNAVAILABLE,
                ..
            })
        ),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "twenty attempts would take two seconds"
    );
    Ok(())
}

/// A request deadline only ever shrinks: options apply the earlier of their
/// deadline and the one already set.
#[test]
fn a_request_deadline_only_shrinks() {
    let soon = Instant::now();
    let later = soon + Duration::from_secs(60);
    let mut request = Request::new(Method::GET, "/ehr".to_owned());
    request.set_deadline(soon);
    request.apply_options(&CallOptions::default().with_deadline(later));
    assert_eq!(request.deadline(), Some(soon));
}

/// An `Authorization` option header is marked sensitive, so `Debug` hides it.
#[test]
fn an_authorization_option_is_hidden_from_debug() -> TestResult {
    let options = CallOptions::default().with_header("Authorization", "Bearer secret-token")?;
    assert!(!format!("{options:?}").contains("secret-token"));
    Ok(())
}
