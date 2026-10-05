// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written ITS-REST client runtime.
//!
//! The HTTP engine seam, the request and answer carriers, the retry budget
//! and the typed errors the generated per-group clients
//! (`rest::generated::<group>::client`) build on. The generated half owns everything the OpenAPI declares per operation
//! (path, parameters, body, the documented statuses, `Prefer`, `If-Match`,
//! `ETag`, `Location`); this module owns what every operation shares: the
//! base URL, credentials, the `Accept` default, retries over idempotent
//! methods, the general statuses the ITS-REST overview documents for every
//! operation (`401`, `403`, `5xx`) and the refusal of an undocumented status.
//! The HTTP engine is a trait ([`Transport`]) so any `http`-speaking client
//! serves; [`ReqwestTransport`] is the engine shipped with the crate.
//!
//! A call may carry a deadline ([`CallOptions`], [`Request::set_deadline`]):
//! the client refuses an attempt once it has passed, hands the engine the
//! remaining budget as a [`RequestTimeout`], and never waits out a retry
//! backoff that would end after it. No openEHR spec governs client deadlines,
//! credential refresh or request forwarding; they are our own design.

use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, PROXY_AUTHORIZATION, WWW_AUTHENTICATE};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::generated::common::Error;

/// The canonical JSON media type, the default `Accept` and `Content-Type`.
const CANONICAL_JSON: &str = "application/json";

/// The time a request has left before its deadline, carried in the
/// `http::Request` extensions the client hands a [`Transport`].
///
/// An engine honours it as the request's timeout, the shorter of it and its
/// own; [`ReqwestTransport`] does. A request without a deadline carries none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTimeout(pub Duration);

/// An HTTP engine the client sends through.
///
/// The request and response are `http` values carrying whole bodies, so an
/// engine adapts in a few lines and the client never depends on one. The
/// engine owns the connection pool, TLS and the per-request timeout; a request
/// that carries a [`RequestTimeout`] extension must not run past it, or the
/// caller's deadline is not kept. An engine sends each request once and
/// returns the answer it read, a `3xx` included: following a redirect would
/// re-send the request, credentials and all, to a host the caller never named.
/// The client itself sends a request again only within its [`RetryPolicy`]
/// and, under [`Credentials::Dpop`] with a [`DpopProver`], once more to the
/// same URL to answer a `use_dpop_nonce` challenge (RFC 9449 §8 and §9).
///
/// The client checks the call's deadline before every send it hands an
/// engine. A deadline that passed before the first send is
/// [`ClientError::DeadlineElapsed`] with `sent: false`, and nothing reached
/// the engine; one that passed before a retry or the nonce re-send carries
/// `sent: true`, because an earlier send went out. An engine timeout inside
/// [`Transport::send`] is a [`TransportError::Timeout`], which the client
/// returns as [`ClientError::Transport`].
///
/// An engine that wraps another may add request headers. A DPoP client
/// without a [`DpopProver`] adds its `DPoP` proof this way: the request it
/// receives carries the final method and URI, and `Authorization: DPoP
/// <token>` names the access token the proof's `ath` hashes (RFC 9449 §4.2).
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Sends `request` and reads the whole response.
    ///
    /// # Errors
    /// Returns a [`TransportError`] when the request did not complete: a
    /// refused connection, a broken stream, or the engine's timeout.
    async fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, TransportError>;
}

/// A request that did not complete, as the engine reported it.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The engine's timeout elapsed before the answer was read.
    #[error("the request did not complete inside the engine's timeout")]
    Timeout {
        /// What the engine reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The request could not be sent, or the answer could not be read.
    #[error("the request could not be sent")]
    Send {
        /// What the engine reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The `reqwest` engine: a shared connection pool with rustls TLS.
///
/// The engine never follows a redirect: every constructor builds its client
/// with `reqwest::redirect::Policy::none()`, so a `3xx` reaches the caller as
/// received and nothing is re-sent to the host its `Location` names. That is
/// why the constructors take a `reqwest::ClientBuilder` rather than a built
/// client, whose redirect policy can be neither read nor changed.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    // The per-request timeout this engine was built with, kept so a request
    // deadline can shorten it and never lengthen it.
    timeout: Option<Duration>,
}

impl ReqwestTransport {
    /// An engine over a `reqwest` client the caller configures (TLS roots,
    /// proxies), built here with redirects off.
    ///
    /// A request that carries a [`RequestTimeout`] runs under that timeout;
    /// use [`ReqwestTransport::with_builder_timeout`] for a per-request
    /// timeout of the engine's own, which a deadline only ever shortens.
    ///
    /// # Errors
    /// Returns the `reqwest` error when the client cannot be built (the TLS
    /// backend cannot be initialised, or the builder's configuration is
    /// refused).
    pub fn new(builder: reqwest::ClientBuilder) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: builder
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            timeout: None,
        })
    }

    /// An engine over a `reqwest` client the caller configures, built here
    /// with redirects off and `timeout` per request; a request deadline only
    /// ever shortens it.
    ///
    /// # Errors
    /// Returns the `reqwest` error when the client cannot be built.
    pub fn with_builder_timeout(
        builder: reqwest::ClientBuilder,
        timeout: Duration,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: builder
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            timeout: Some(timeout),
        })
    }

    /// An engine with the default client, redirects off and `timeout` per
    /// request.
    ///
    /// # Errors
    /// Returns the `reqwest` error when the TLS backend cannot be initialised.
    pub fn with_timeout(timeout: Duration) -> Result<Self, reqwest::Error> {
        Self::with_builder_timeout(reqwest::Client::builder(), timeout)
    }
}

#[async_trait::async_trait]
impl Transport for ReqwestTransport {
    async fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, TransportError> {
        let budget = request.extensions().get::<RequestTimeout>().map(|t| t.0);
        let mut request =
            reqwest::Request::try_from(request).map_err(|source| TransportError::Send {
                source: Box::new(source),
            })?;
        if let Some(budget) = budget {
            let timeout = self.timeout.map_or(budget, |own| own.min(budget));
            *request.timeout_mut() = Some(timeout);
        }
        let response = self.client.execute(request).await.map_err(|source| {
            if source.is_timeout() {
                TransportError::Timeout {
                    source: Box::new(source),
                }
            } else {
                TransportError::Send {
                    source: Box::new(source),
                }
            }
        })?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|source| TransportError::Send {
                source: Box::new(source),
            })?
            .to_vec();
        let mut out = http::Response::new(body);
        *out.status_mut() = status;
        *out.headers_mut() = headers;
        Ok(out)
    }
}

/// The credentials sent on every request, as the `Authorization` header.
///
/// ITS-REST leaves the scheme to the service ("Clients MUST send valid
/// `Authorization` … headers in their requests when required",
/// `ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §Authentication and authorization); Basic, Bearer and DPoP cover the
/// shipped servers. The secret is a [`SecretString`]: `Debug` never prints it
/// and the memory is zeroed on drop.
#[derive(Clone)]
pub enum Credentials {
    /// HTTP Basic (RFC 7617).
    Basic {
        /// The user name.
        user: String,
        /// The password.
        password: SecretString,
    },
    /// A bearer token (RFC 6750), sent verbatim after `Bearer `.
    Bearer(SecretString),
    /// A DPoP-bound access token (RFC 9449), sent verbatim after `DPoP `
    /// (§7.1).
    ///
    /// Every request also needs a `DPoP` proof header: the client asks the
    /// [`DpopProver`] it was given ([`Client::with_dpop_prover`]) for one, or a
    /// wrapping [`Transport`] adds it.
    Dpop(SecretString),
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic { user, .. } => f
                .debug_struct("Basic")
                .field("user", user)
                .field("password", &"<redacted>")
                .finish(),
            Self::Bearer(_) => f.debug_tuple("Bearer").field(&"<redacted>").finish(),
            Self::Dpop(_) => f.debug_tuple("Dpop").field(&"<redacted>").finish(),
        }
    }
}

impl Credentials {
    /// HTTP Basic credentials for `user`.
    #[must_use]
    pub fn basic(user: impl Into<String>, password: impl Into<SecretString>) -> Self {
        Self::Basic {
            user: user.into(),
            password: password.into(),
        }
    }

    /// A bearer token.
    #[must_use]
    pub fn bearer(token: impl Into<SecretString>) -> Self {
        Self::Bearer(token.into())
    }

    /// A DPoP-bound access token.
    #[must_use]
    pub fn dpop(token: impl Into<SecretString>) -> Self {
        Self::Dpop(token.into())
    }

    /// The `Authorization` field value the client sends for these
    /// credentials, marked sensitive.
    ///
    /// A consumer that validates a credential before the first request (at
    /// configuration load) calls this, so it checks exactly what is sent.
    /// The basic form follows RFC 7617 §2: the user-id carries no colon, and
    /// neither the user-id nor the password carries a control character (the
    /// `CTL` of RFC 5234 Appendix B.1). The bearer form follows the `b64token`
    /// syntax of RFC 6750 §2.1, and the DPoP form the `token68` syntax of
    /// RFC 9449 §7.1, which admits the same characters.
    ///
    /// # Errors
    /// Returns [`InvalidCredentials`] naming the rule the credentials break;
    /// it never carries the secret.
    pub fn header_value(&self) -> Result<HeaderValue, InvalidCredentials> {
        use base64::Engine as _;
        let text = match self {
            Self::Basic { user, password } => {
                if user.contains(':') {
                    return Err(InvalidCredentials::ColonInUserId);
                }
                if user.chars().any(|c| c.is_ascii_control()) {
                    return Err(InvalidCredentials::ControlCharacter(BasicPart::UserId));
                }
                if password
                    .expose_secret()
                    .chars()
                    .any(|c| c.is_ascii_control())
                {
                    return Err(InvalidCredentials::ControlCharacter(BasicPart::Password));
                }
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("{user}:{}", password.expose_secret()))
                )
            }
            Self::Bearer(token) => {
                if !is_b64token(token.expose_secret()) {
                    return Err(InvalidCredentials::NotB64Token);
                }
                format!("Bearer {}", token.expose_secret())
            }
            Self::Dpop(token) => {
                if !is_b64token(token.expose_secret()) {
                    return Err(InvalidCredentials::NotToken68);
                }
                format!("DPoP {}", token.expose_secret())
            }
        };
        let mut value =
            HeaderValue::from_str(&text).map_err(InvalidCredentials::NotAHeaderValue)?;
        value.set_sensitive(true);
        Ok(value)
    }
}

/// Whether `token` matches RFC 6750 §2.1 `b64token`:
/// `1*( ALPHA / DIGIT / "-" / "." / "_" / "~" / "+" / "/" ) *"="`.
fn is_b64token(token: &str) -> bool {
    let body = token.trim_end_matches('=');
    !body.is_empty()
        && body.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        })
}

/// Why [`Credentials`] cannot form an `Authorization` value.
///
/// No variant carries the secret, so the error is safe to log.
#[derive(Debug, thiserror::Error)]
pub enum InvalidCredentials {
    /// A basic user-id contains a colon, which RFC 7617 §2 makes invalid: the
    /// first colon of a user-pass separates the user-id from the password.
    #[error("the basic user-id contains a colon (RFC 7617 §2)")]
    ColonInUserId,
    /// A basic user-id or password contains a control character, which
    /// RFC 7617 §2 forbids.
    #[error("the basic {0} contains a control character (RFC 7617 §2)")]
    ControlCharacter(BasicPart),
    /// A bearer token is not a `b64token` (RFC 6750 §2.1).
    #[error("the bearer token is not a b64token (RFC 6750 §2.1)")]
    NotB64Token,
    /// A DPoP access token is not a `token68` (RFC 9449 §7.1).
    #[error("the DPoP access token is not a token68 (RFC 9449 §7.1)")]
    NotToken68,
    /// The composed value is not a legal header value.
    #[error("the credentials do not form a legal Authorization header value")]
    NotAHeaderValue(#[source] http::header::InvalidHeaderValue),
}

/// The part of a basic credential a refusal names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicPart {
    /// The user-id.
    UserId,
    /// The password.
    Password,
}

impl fmt::Display for BasicPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UserId => "user-id",
            Self::Password => "password",
        })
    }
}

/// A source of the credentials the client sends, asked once per attempt.
///
/// A provider that obtains a token (an OAuth 2.0 grant, an RFC 7523
/// assertion) and refreshes it before expiry implements this trait; the
/// static [`Credentials`] forms implement it by answering themselves. The
/// client resolves the credential at send time, so a retry after a refresh
/// carries the new token. "No credential" is a client without a provider.
#[async_trait::async_trait]
pub trait CredentialsProvider: Send + Sync + fmt::Debug {
    /// The credentials for the next attempt.
    ///
    /// # Errors
    /// Returns a [`CredentialsError`] when no credential can be obtained; the
    /// call then fails with [`ClientError::Credentials`] before anything is
    /// sent.
    async fn credentials(&self) -> Result<Credentials, CredentialsError>;

    /// Reports that the service answered `401` to an attempt that carried the
    /// last credential, so a caching provider can drop it.
    ///
    /// Called on the answering task; it must not block. The default does
    /// nothing.
    fn refused(&self) {}
}

#[async_trait::async_trait]
impl CredentialsProvider for Credentials {
    async fn credentials(&self) -> Result<Credentials, CredentialsError> {
        Ok(self.clone())
    }
}

#[async_trait::async_trait]
impl<P: CredentialsProvider + ?Sized> CredentialsProvider for Arc<P> {
    async fn credentials(&self) -> Result<Credentials, CredentialsError> {
        P::credentials(self).await
    }

    fn refused(&self) {
        P::refused(self);
    }
}

/// A credentials provider that could not produce a credential.
#[derive(Debug, thiserror::Error)]
#[error("the credentials provider could not produce a credential")]
pub struct CredentialsError {
    /// What the provider reported.
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl CredentialsError {
    /// The failure `source` reported by a provider.
    #[must_use]
    pub fn new(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self {
            source: source.into(),
        }
    }
}

/// The `DPoP` request header carrying the proof JWT (RFC 9449 §4.1).
const DPOP: HeaderName = HeaderName::from_static("dpop");

/// The `DPoP-Nonce` response header carrying a server-provided nonce
/// (RFC 9449 §8.1 and §9).
const DPOP_NONCE: HeaderName = HeaderName::from_static("dpop-nonce");

/// A source of DPoP proofs (RFC 9449 §4), asked once per send under
/// [`Credentials::Dpop`].
///
/// The client hands the prover the final method and URI of the request and
/// the access token, and sends what it returns as the `DPoP` header. The
/// prover owns the key pair and the claims: `htm` from the method, `htu` from
/// the URI without its query and fragment (§4.2), `ath` from the access token,
/// a fresh `jti` and `iat`, and the latest nonce it was given.
///
/// When the call's deadline passes before the first send, the call fails with
/// [`ClientError::DeadlineElapsed`] and `sent: false`. When it passes after a
/// `use_dpop_nonce` challenge and before the re-send (the prover's second
/// proof included), the call fails with the same variant and `sent: true`:
/// the service received the first send and may have acted on it. A prover
/// failure is reported the same way: [`ClientError::DpopProof`] carries
/// `sent: false` when the first proof failed and `sent: true` when the proof
/// for a retry or the nonce re-send did.
#[async_trait::async_trait]
pub trait DpopProver: Send + Sync + fmt::Debug {
    /// The compact-serialized proof JWT for one request.
    ///
    /// # Errors
    /// Returns a [`CredentialsError`] when no proof can be made; the call then
    /// fails with [`ClientError::DpopProof`] before this send goes out, its
    /// `sent` saying whether an earlier send of the call did.
    async fn proof(&self, request: &DpopProofRequest<'_>) -> Result<String, CredentialsError>;

    /// Records the nonce the service supplied in a `DPoP-Nonce` header, which
    /// every later proof carries as its `nonce` claim (RFC 9449 §8 and §9).
    ///
    /// Called on the answering task for every answer that carries one, before
    /// the client re-sends a request the service refused with
    /// `use_dpop_nonce`; it must not block.
    fn nonce(&self, nonce: &str);
}

#[async_trait::async_trait]
impl<P: DpopProver + ?Sized> DpopProver for Arc<P> {
    async fn proof(&self, request: &DpopProofRequest<'_>) -> Result<String, CredentialsError> {
        P::proof(self, request).await
    }

    fn nonce(&self, nonce: &str) {
        P::nonce(self, nonce);
    }
}

/// What a [`DpopProver`] signs over: the request as it is sent.
#[derive(Debug, Clone, Copy)]
pub struct DpopProofRequest<'a> {
    method: &'a Method,
    uri: &'a http::Uri,
    access_token: &'a SecretString,
}

impl<'a> DpopProofRequest<'a> {
    /// The HTTP method, the proof's `htm`.
    #[must_use]
    pub fn method(&self) -> &'a Method {
        self.method
    }

    /// The full request URI; the proof's `htu` is this without its query and
    /// fragment (RFC 9449 §4.2).
    #[must_use]
    pub fn uri(&self) -> &'a http::Uri {
        self.uri
    }

    /// The access token whose hash is the proof's `ath` (RFC 9449 §4.2).
    #[must_use]
    pub fn access_token(&self) -> &'a SecretString {
        self.access_token
    }
}

/// Whether an answer is a `use_dpop_nonce` challenge: a `401` whose `DPoP`
/// challenge names that error (RFC 9449 §9), or a `400` whose JSON body does
/// (§8).
fn is_nonce_challenge(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> bool {
    if status == StatusCode::UNAUTHORIZED {
        return headers.get_all(WWW_AUTHENTICATE).iter().any(|value| {
            // NOTE: RFC 9110 §5.5 admits opaque octets; a challenge that is
            // not text is legitimately not a DPoP challenge.
            value.to_str().is_ok_and(|text| {
                text.split_once(' ').is_some_and(|(scheme, params)| {
                    scheme.eq_ignore_ascii_case("DPoP") && params.contains("use_dpop_nonce")
                })
            })
        });
    }
    status == StatusCode::BAD_REQUEST
        && serde_json::from_slice::<OAuthErrorBody>(body)
            .is_ok_and(|parsed| parsed.error == "use_dpop_nonce")
}

/// The `error` member of an OAuth 2.0 error response (RFC 6749 §5.2), the
/// shape RFC 9449 §8 answers a missing nonce with; other members are ignored.
#[derive(serde::Deserialize)]
struct OAuthErrorBody {
    error: String,
}

/// Per-call options: a deadline and extra request headers.
///
/// A generated group client applies its options to every call
/// (`with_options`); a raw [`Request`] takes them through
/// [`Request::apply_options`]. An option header replaces every field line of
/// the same name the call would otherwise send (the request id or the
/// conveyed client identity, set per call); the client's configured
/// credentials still set `Authorization`. `Debug` prints the header values
/// except those marked sensitive (`Authorization`, `Proxy-Authorization`).
#[derive(Debug, Clone, Default)]
pub struct CallOptions {
    deadline: Option<Instant>,
    headers: HeaderMap,
}

impl CallOptions {
    /// These options with `deadline` as the instant the call must finish by,
    /// retries included.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// These options with a deadline `timeout` from now, replacing any
    /// deadline set before; a timeout the platform clock cannot represent sets
    /// no deadline.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.deadline = Instant::now().checked_add(timeout);
        self
    }

    /// These options with one more field line of header `name`; set the same
    /// name twice to send two field lines.
    ///
    /// # Errors
    /// Returns [`ClientError::HeaderName`] or [`ClientError::HeaderValue`]
    /// when either is not legal on the wire.
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self, ClientError> {
        let (name, mut value) = header_field(name, value)?;
        if name == AUTHORIZATION || name == PROXY_AUTHORIZATION {
            value.set_sensitive(true);
        }
        self.headers.append(name, value);
        Ok(self)
    }

    /// The instant the call must finish by, when one is set.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// The extra request headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}

/// A header name and value checked for the wire.
fn header_field(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), ClientError> {
    let header_name =
        HeaderName::from_bytes(name.as_bytes()).map_err(|source| ClientError::HeaderName {
            header: name.to_owned(),
            source,
        })?;
    let header_value = HeaderValue::from_str(value).map_err(|source| ClientError::HeaderValue {
        header: name.to_owned(),
        source,
    })?;
    Ok((header_name, header_value))
}

/// The retry budget over idempotent requests.
///
/// Only a request RFC 9110 §9.2.2 makes idempotent (`GET`, `HEAD`,
/// `OPTIONS`, `PUT`, `DELETE`) is ever sent twice, and only after a failure
/// that did not reach a documented answer: a transport failure, the
/// engine's timeout, or a `5xx`. A `POST` is sent exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The total number of attempts, the first included; `1` never retries.
    pub max_attempts: usize,
    /// The delay before the second attempt; each further delay doubles.
    pub initial_backoff: Duration,
    /// The longest delay between two attempts.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(2),
        }
    }
}

/// A client for one openEHR CDR: its base URL, credentials and retry budget
/// over an HTTP engine.
///
/// The base URL is the service root the ITS-REST paths hang under, ending in
/// the API version segment (`https://cdr.example.org/openehr/v1`). The
/// per-group operations are the generated clients in
/// `rest::generated::<group>::client`; [`Client::execute`] is the raw seam
/// underneath them, for a request the typed surface does not build (a
/// Simplified Formats body, a vendor extension), and [`Client::forward`]
/// passes a request through unclassified, for an intermediary.
#[derive(Debug, Clone)]
pub struct Client<T> {
    transport: T,
    base: url::Url,
    credentials: Option<Arc<dyn CredentialsProvider>>,
    dpop: Option<Arc<dyn DpopProver>>,
    retry: RetryPolicy,
}

impl<T: Transport> Client<T> {
    /// A client over `transport` for the service rooted at `base`.
    ///
    /// # Errors
    /// Returns [`ClientError::BaseUrl`] when `base` cannot carry a path.
    pub fn new(transport: T, base: url::Url) -> Result<Self, ClientError> {
        if base.cannot_be_a_base() {
            return Err(ClientError::BaseUrl { base });
        }
        Ok(Self {
            transport,
            base,
            credentials: None,
            dpop: None,
            retry: RetryPolicy::default(),
        })
    }

    /// This client sending `credentials` on every request.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(Arc::new(credentials));
        self
    }

    /// This client asking `provider` for the credentials of every attempt
    /// and telling it when the service refused them (`401`).
    ///
    /// An `Arc<P>` is a provider too, so one provider can serve several
    /// clients.
    #[must_use]
    pub fn with_credentials_provider(
        mut self,
        provider: impl CredentialsProvider + 'static,
    ) -> Self {
        self.credentials = Some(Arc::new(provider));
        self
    }

    /// This client asking `prover` for the `DPoP` proof of every request sent
    /// under [`Credentials::Dpop`], and answering a `use_dpop_nonce`
    /// challenge with one re-send carrying a proof over the supplied nonce
    /// (RFC 9449 §8 and §9).
    ///
    /// Under other credentials the prover is never asked.
    #[must_use]
    pub fn with_dpop_prover(mut self, prover: impl DpopProver + 'static) -> Self {
        self.dpop = Some(Arc::new(prover));
        self
    }

    /// This client with `retry` as its budget.
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The service root every path is resolved under.
    #[must_use]
    pub fn base(&self) -> &url::Url {
        &self.base
    }

    /// The HTTP engine.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The retry budget.
    #[must_use]
    pub fn retry(&self) -> RetryPolicy {
        self.retry
    }

    /// Sends `request` under the base URL and reads the whole answer, retrying
    /// an idempotent request within the budget.
    ///
    /// The `Accept` header defaults to `application/json` when the request set
    /// none; the credentials, when configured, are resolved and sent on every
    /// attempt. A request deadline bounds the whole call: no attempt starts
    /// after it, each attempt's engine timeout is the time left, and a retry
    /// whose backoff would end after it is not made — the last failure is
    /// returned instead.
    ///
    /// # Errors
    /// Returns [`ClientError::Unauthorized`], [`ClientError::Forbidden`] and
    /// [`ClientError::ServiceFailure`] for the general statuses the ITS-REST
    /// overview documents for every operation, [`ClientError::Transport`] when
    /// no attempt completed, [`ClientError::DeadlineElapsed`] when the
    /// deadline passed before an attempt (its `sent` says whether an earlier
    /// attempt or a DPoP nonce re-send's first send went out), and
    /// [`ClientError::Credentials`]
    /// when the provider produced no credential. Every other status is the
    /// caller's to match.
    pub async fn execute(&self, request: Request) -> Result<Answer, ClientError> {
        use backon::Retryable as _;
        if !is_idempotent(&request.method) || self.retry.max_attempts <= 1 {
            return self.attempt(&request, false).await;
        }
        let retries = self.retry.max_attempts.saturating_sub(1);
        let backoff = backon::ExponentialBuilder::new()
            .with_min_delay(self.retry.initial_backoff)
            .with_max_delay(self.retry.max_backoff)
            .with_max_times(retries);
        let deadline = request.deadline;
        let sent = std::sync::atomic::AtomicBool::new(false);
        (|| {
            let earlier = sent.swap(true, std::sync::atomic::Ordering::Relaxed);
            self.attempt(&request, earlier)
        })
        .retry(backoff)
        .when(ClientError::is_retryable)
        .adjust(move |_, delay| {
            delay.filter(|delay| {
                deadline.is_none_or(|deadline| {
                    Instant::now()
                        .checked_add(*delay)
                        .is_some_and(|wake| wake < deadline)
                })
            })
        })
        .await
    }

    /// Sends `request` once under the base URL and returns whatever the
    /// service answered, for an intermediary passing a request through.
    ///
    /// Nothing is classified and nothing is retried, except the one re-send
    /// that answers a DPoP `use_dpop_nonce` challenge: a `401`, `403` or `5xx`
    /// is an [`Answer`] like any other, and the status, every header (`ETag`,
    /// `Location`) and the body bytes are as received. A `3xx` is returned,
    /// never followed: [`ReqwestTransport`] is always built with redirects off,
    /// and a caller's own [`Transport`] must not follow them either. The request goes out as
    /// built — path, query ([`Request::raw_query`]), headers and body
    /// ([`Request::raw_body`]) unchanged — except that the client's credentials,
    /// when configured, set `Authorization` (and `DPoP`, under a
    /// [`DpopProver`]), and `Accept` defaults to
    /// `application/json` when absent. Stripping hop-by-hop fields (RFC 9110
    /// §7.6.1) is the caller's.
    ///
    /// # Errors
    /// Returns [`ClientError::Transport`] when the request did not complete,
    /// [`ClientError::DeadlineElapsed`] when its deadline has passed — with
    /// `sent: false` before anything went out, and `sent: true` when it passed
    /// before the re-send that answers a DPoP nonce challenge —,
    /// [`ClientError::Credentials`] when the provider produced no credential,
    /// and [`ClientError::Build`] when the parts do not form a request.
    pub async fn forward(&self, request: Request) -> Result<Answer, ClientError> {
        self.send_once(&request, false).await
    }

    /// One attempt: send it, classify the general statuses; `sent` says
    /// whether an earlier attempt of the same call went out.
    async fn attempt(&self, request: &Request, sent: bool) -> Result<Answer, ClientError> {
        let answer = self.send_once(request, sent).await?;
        if answer.status == StatusCode::UNAUTHORIZED {
            return Err(ClientError::Unauthorized {
                method: answer.method,
                path: answer.path,
                challenge: header_text(&answer.headers, WWW_AUTHENTICATE.as_str()),
                body: ErrorBody::from_bytes(answer.body),
            });
        }
        if answer.status == StatusCode::FORBIDDEN {
            return Err(ClientError::Forbidden {
                method: answer.method,
                path: answer.path,
                body: ErrorBody::from_bytes(answer.body),
            });
        }
        if answer.status.is_server_error() {
            return Err(ClientError::ServiceFailure {
                method: answer.method,
                path: answer.path,
                status: answer.status,
                body: ErrorBody::from_bytes(answer.body),
            });
        }
        Ok(answer)
    }

    /// One send of `request`: resolve the credential, build, hand the engine
    /// the time left, and read the answer whole, unclassified; under DPoP, a
    /// `use_dpop_nonce` challenge is answered with one re-send.
    async fn send_once(&self, request: &Request, mut sent: bool) -> Result<Answer, ClientError> {
        request.remaining(sent)?;
        let credentials =
            match self.credentials.as_ref() {
                Some(provider) => Some(provider.credentials().await.map_err(|source| {
                    ClientError::Credentials {
                        method: request.method.clone(),
                        path: request.path.clone(),
                        source,
                    }
                })?),
                None => None,
            };
        let prover = match (&credentials, self.dpop.as_ref()) {
            (Some(Credentials::Dpop(token)), Some(prover)) => Some((prover, token)),
            _ => None,
        };
        let mut nonce_resend = prover.is_some();
        let (parts, body) = loop {
            let mut built = self.build(request, credentials.as_ref())?;
            if let Some((prover, token)) = prover {
                let proof = prover
                    .proof(&DpopProofRequest {
                        method: built.method(),
                        uri: built.uri(),
                        access_token: token,
                    })
                    .await
                    .map_err(|source| ClientError::DpopProof {
                        method: request.method.clone(),
                        path: request.path.clone(),
                        sent,
                        source,
                    })?;
                let mut value =
                    HeaderValue::from_str(&proof).map_err(|source| ClientError::HeaderValue {
                        header: DPOP.to_string(),
                        source,
                    })?;
                value.set_sensitive(true);
                built.headers_mut().insert(DPOP, value);
            }
            if let Some(left) = request.remaining(sent)? {
                built.extensions_mut().insert(RequestTimeout(left));
            }
            let response =
                self.transport
                    .send(built)
                    .await
                    .map_err(|source| ClientError::Transport {
                        method: request.method.clone(),
                        path: request.path.clone(),
                        source,
                    })?;
            sent = true;
            let (parts, body) = response.into_parts();
            if let Some((prover, _)) = prover
                && let Some(nonce) = header_text(&parts.headers, DPOP_NONCE.as_str())
            {
                prover.nonce(&nonce);
                if nonce_resend && is_nonce_challenge(parts.status, &parts.headers, &body) {
                    nonce_resend = false;
                    continue;
                }
            }
            break (parts, body);
        };
        if parts.status == StatusCode::UNAUTHORIZED
            && let Some(provider) = self.credentials.as_ref()
        {
            provider.refused();
        }
        Ok(Answer {
            method: request.method.clone(),
            path: request.path.clone(),
            status: parts.status,
            headers: parts.headers,
            body,
        })
    }

    /// The `http` request for one attempt, carrying `credentials` when set.
    fn build(
        &self,
        request: &Request,
        credentials: Option<&Credentials>,
    ) -> Result<http::Request<Vec<u8>>, ClientError> {
        let mut url = self.base.clone();
        let root = self.base.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{root}{}", request.path));
        url.set_query((!request.query.is_empty()).then_some(request.query.as_str()));
        let mut builder = http::Request::builder()
            .method(request.method.clone())
            .uri(url.as_str());
        if let Some(headers) = builder.headers_mut() {
            headers.extend(request.headers.clone());
            if !headers.contains_key(ACCEPT) {
                headers.insert(ACCEPT, HeaderValue::from_static(CANONICAL_JSON));
            }
            if let Some(credentials) = credentials {
                let value = credentials.header_value().map_err(|source| {
                    ClientError::InvalidCredentials {
                        method: request.method.clone(),
                        path: request.path.clone(),
                        source,
                    }
                })?;
                headers.insert(AUTHORIZATION, value);
            }
        }
        builder
            .body(request.body.clone().unwrap_or_default())
            .map_err(|source| ClientError::Build {
                method: request.method.clone(),
                path: request.path.clone(),
                source,
            })
    }
}

/// Whether RFC 9110 §9.2.2 makes `method` idempotent.
fn is_idempotent(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::PUT | Method::DELETE
    )
}

/// The value of header `name`, when it is usable text.
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    // NOTE: RFC 9110 §5.5 admits opaque octets in a field value; one that is
    // not visible ASCII is legitimately unusable as text, not defective.
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// The body of an error answer: the bytes as received, and the ITS-REST
/// `Error` they decode to when they are one.
///
/// ITS-REST attaches the `Error` schema to a `400` and says the body "MAY
/// contain error details" (the `400` response component); every other error
/// status describes no body. A service may still send one in any shape, so
/// the raw bytes are kept beside the decoded form and nothing the service
/// said is lost.
#[derive(Clone)]
pub struct ErrorBody {
    raw: Vec<u8>,
    // Boxed: the decoded form rides inside every `ClientError` variant.
    error: Option<Box<Error>>,
}

impl ErrorBody {
    /// The error body read from `raw`.
    #[must_use]
    pub fn from_bytes(raw: Vec<u8>) -> Self {
        // NOTE: ITS-REST (`400` response component) says "The response body
        // MAY contain error details", so a body that is not an `Error` is
        // legitimately absent detail, not a defect; the bytes stay in `raw`.
        let error = serde_json::from_slice::<Error>(&raw).ok().map(Box::new);
        Self { raw, error }
    }

    /// The bytes as received.
    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The body as text, when it is UTF-8.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        // NOTE: no openEHR spec constrains the bytes of an error body; one
        // that is not UTF-8 is legitimately not text, not a defect.
        std::str::from_utf8(&self.raw).ok()
    }

    /// The decoded ITS-REST `Error`, when the body is one.
    #[must_use]
    pub fn error(&self) -> Option<&Error> {
        self.error.as_deref()
    }

    /// The `message` of the ITS-REST `Error`, when the body is one.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.error.as_ref().map(|e| e.message.as_str())
    }

    /// The `validationErrors` of the ITS-REST `Error`; empty when the body is
    /// not one or lists none.
    #[must_use]
    pub fn validation_errors(&self) -> &[String] {
        self.error
            .as_ref()
            .map_or(&[], |e| e.validation_errors.as_slice())
    }

    /// Whether the service sent no body at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
}

impl fmt::Debug for ErrorBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorBody")
            .field("error", &self.error)
            .field("raw", &String::from_utf8_lossy(&self.raw))
            .finish()
    }
}

/// A percent-encoded path segment, for substitution into an operation's path
/// template.
///
/// Every character outside RFC 3986 §2.3 unreserved is encoded, except the
/// two §3.3 `pchar` extras `:` and `@`, which stay literal — a version uid
/// (`8849182c…::cdr.example.org::1`) is sent as the spec's own examples write
/// it. `/` is encoded, so a value can never add a segment.
#[must_use]
pub fn path_segment(value: &impl fmt::Display) -> String {
    urlencoding::encode(&value.to_string())
        .replace("%3A", ":")
        .replace("%40", "@")
}

/// One request under the client's base URL, as the generated methods build
/// it: a method, an operation path, a query string, headers, a body and an
/// optional deadline.
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    path: String,
    query: String,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
    deadline: Option<Instant>,
}

impl Request {
    /// A request of `method` to `path`, the operation's path with its
    /// parameters substituted (`/ehr/{ehr_id}` → `/ehr/7d44…`).
    #[must_use]
    pub fn new(method: Method, path: String) -> Self {
        Self {
            method,
            path,
            query: String::new(),
            headers: HeaderMap::new(),
            body: None,
            deadline: None,
        }
    }

    /// The instant the call must finish by, when one is set.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Sets `deadline`, keeping the earlier one when a deadline is already
    /// set: a budget only ever shrinks.
    pub fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = Some(self.deadline.map_or(deadline, |set| set.min(deadline)));
    }

    /// Applies `options`: its deadline as [`Request::set_deadline`] does, and
    /// its headers, each replacing every field line of the same name set so
    /// far.
    pub fn apply_options(&mut self, options: &CallOptions) {
        if let Some(deadline) = options.deadline {
            self.set_deadline(deadline);
        }
        for name in options.headers.keys() {
            self.headers.remove(name);
        }
        for (name, value) in &options.headers {
            self.headers.append(name.clone(), value.clone());
        }
    }

    /// The time left before the deadline, or `None` without one; `sent` says
    /// whether an earlier send of the same call went out.
    fn remaining(&self, sent: bool) -> Result<Option<Duration>, ClientError> {
        let Some(deadline) = self.deadline else {
            return Ok(None);
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ClientError::DeadlineElapsed {
                method: self.method.clone(),
                path: self.path.clone(),
                sent,
            });
        }
        Ok(Some(left))
    }

    /// The HTTP method.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The operation path under the base URL.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The query string, without its leading `?`.
    #[must_use]
    pub fn query_string(&self) -> &str {
        &self.query
    }

    /// The request headers set so far.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The request headers, for setting field values byte for byte — a
    /// forwarded request's headers as received, opaque octets included.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Sets the query string to `query` verbatim, without its leading `?`
    /// and without encoding, replacing every pair appended so far.
    pub fn raw_query(&mut self, query: &str) {
        query.clone_into(&mut self.query);
    }

    /// Sets `body` as the bytes to send, unchanged, with `content_type` as
    /// its `Content-Type` when given (otherwise the headers decide).
    pub fn raw_body(&mut self, body: Vec<u8>, content_type: Option<HeaderValue>) {
        if let Some(content_type) = content_type {
            self.headers.insert(CONTENT_TYPE, content_type);
        }
        self.body = Some(body);
    }

    /// The body, when one is set.
    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }

    /// Appends the query pair `name=value`, both percent-encoded.
    pub fn query(&mut self, name: &str, value: impl fmt::Display) {
        if !self.query.is_empty() {
            self.query.push('&');
        }
        self.query.push_str(&urlencoding::encode(name));
        self.query.push('=');
        self.query
            .push_str(&urlencoding::encode(&value.to_string()));
    }

    /// Appends one occurrence of header `name` with `value`.
    ///
    /// # Errors
    /// Returns [`ClientError::HeaderName`] or [`ClientError::HeaderValue`]
    /// when either is not legal on the wire.
    pub fn header(&mut self, name: &str, value: &str) -> Result<(), ClientError> {
        let (header_name, header_value) = header_field(name, value)?;
        self.headers.append(header_name, header_value);
        Ok(())
    }

    /// Sets `body` serialized as canonical JSON, with `Content-Type:
    /// application/json`.
    ///
    /// The typed surface speaks canonical JSON only: `content_type` may name
    /// `application/json` or nothing. A Simplified Formats body is a raw
    /// request through [`Request::text_body`].
    ///
    /// # Errors
    /// Returns [`ClientError::UnsupportedMediaType`] for any other
    /// `content_type` and [`ClientError::Serialize`] when `body` does not
    /// serialize.
    pub fn json_body<B: serde::Serialize + ?Sized>(
        &mut self,
        body: &B,
        content_type: Option<&str>,
    ) -> Result<(), ClientError> {
        // The media type proper, without its parameters (`; charset=utf-8`).
        let media = content_type.map(|ct| ct.split(';').next().unwrap_or_default().trim());
        if let Some(requested) = content_type
            && !media.is_some_and(|m| m.eq_ignore_ascii_case(CANONICAL_JSON))
        {
            return Err(ClientError::UnsupportedMediaType {
                requested: requested.to_owned(),
            });
        }
        let bytes = serde_json::to_vec(body).map_err(|source| ClientError::Serialize { source })?;
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static(CANONICAL_JSON));
        self.body = Some(bytes);
        Ok(())
    }

    /// Sets `text` as the body, sent as `content_type`.
    ///
    /// # Errors
    /// Returns [`ClientError::HeaderValue`] when `content_type` is not a legal
    /// header value.
    pub fn text_body(&mut self, text: &str, content_type: &str) -> Result<(), ClientError> {
        let value =
            HeaderValue::from_str(content_type).map_err(|source| ClientError::HeaderValue {
                header: CONTENT_TYPE.to_string(),
                source,
            })?;
        self.headers.insert(CONTENT_TYPE, value);
        self.body = Some(text.as_bytes().to_vec());
        Ok(())
    }
}

/// What the service answered, read whole: from [`Client::execute`] a status
/// that is neither a general refusal nor a service failure, from
/// [`Client::forward`] any status.
#[derive(Debug, Clone)]
pub struct Answer {
    method: Method,
    path: String,
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    /// The status the service answered with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The value of response header `name`, when the service sent one that is
    /// usable text (the first field line when it repeats).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<String> {
        header_text(&self.headers, name)
    }

    /// Every value of response header `name`, one per field line, the ones
    /// that are usable text.
    #[must_use]
    pub fn header_all(&self, name: &str) -> Vec<String> {
        // NOTE: RFC 9110 §5.5 admits opaque octets in a field value; one that
        // is not visible ASCII is legitimately unusable as text, not defective.
        self.headers
            .get_all(name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_owned)
            .collect()
    }

    /// The body as received.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The body as received, taken out of the answer.
    #[must_use]
    pub fn into_body(self) -> Vec<u8> {
        self.body
    }

    /// The body decoded from canonical JSON as `D`.
    ///
    /// # Errors
    /// Returns [`ClientError::Body`], naming the JSON path of the defect,
    /// when the body is not a `D`.
    pub fn json<D: serde::de::DeserializeOwned>(&self) -> Result<D, ClientError> {
        let mut deserializer = serde_json::Deserializer::from_slice(&self.body);
        serde_path_to_error::deserialize(&mut deserializer).map_err(|source| ClientError::Body {
            method: self.method.clone(),
            path: self.path.clone(),
            status: self.status,
            source,
        })
    }

    /// The body decoded as `D`, or `None` when the service sent no body.
    ///
    /// # Errors
    /// Returns [`ClientError::Body`] when a non-empty body is not a `D`.
    pub fn optional_json<D: serde::de::DeserializeOwned>(&self) -> Result<Option<D>, ClientError> {
        if self.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        self.json().map(Some)
    }

    /// The body as an error answer's body: the bytes as received, decoded as
    /// the ITS-REST `Error` when they are one.
    #[must_use]
    pub fn error_body(&self) -> ErrorBody {
        ErrorBody::from_bytes(self.body.clone())
    }

    /// This answer as the error for a status the operation does not document.
    #[must_use]
    pub fn into_undocumented(self) -> ClientError {
        ClientError::UndocumentedStatus {
            method: self.method,
            path: self.path,
            status: self.status,
            body: ErrorBody::from_bytes(self.body),
        }
    }
}

/// A call that did not reach a documented answer.
///
/// A status the operation documents is an outcome variant of that call,
/// never an error; everything here is the other half. An error names the
/// method and operation path, never the query (which can name a subject) and
/// never the request body (which can be clinical content); a status error
/// carries the service's own answer body as an [`ErrorBody`], and the
/// message text stays free of it.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The base URL cannot carry an operation path.
    #[error("{base} cannot be the base of an ITS-REST path")]
    BaseUrl {
        /// The rejected base URL.
        base: url::Url,
    },
    /// The call's deadline passed before an attempt could start.
    ///
    /// `sent` tells the two cases apart: `false` when nothing of the call left
    /// the process, `true` when an earlier send went out first — a retried
    /// attempt, or the re-send answering a DPoP `use_dpop_nonce` challenge —,
    /// so the service may have received, and acted on, the request.
    #[error(
        "the deadline of {method} {path} passed before the request was sent (an earlier send went out: {sent})"
    )]
    DeadlineElapsed {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// Whether an earlier send of this call went out before the deadline
        /// passed.
        sent: bool,
    },
    /// The DPoP prover produced no proof for the request, so this send did not
    /// go out.
    ///
    /// `sent` tells the two cases apart: `false` when the first proof failed
    /// and nothing of the call left the process, `true` when the proof for a
    /// retried attempt or for the re-send answering a DPoP `use_dpop_nonce`
    /// challenge failed, after the service had received an earlier send.
    #[error("no DPoP proof could be made for {method} {path} (an earlier send went out: {sent})")]
    DpopProof {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// Whether an earlier send of this call went out before the proof
        /// failed.
        sent: bool,
        /// What the prover reported.
        #[source]
        source: CredentialsError,
    },
    /// The credentials provider produced no credential for the request.
    #[error("no credential could be obtained for {method} {path}")]
    Credentials {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// What the provider reported.
        #[source]
        source: CredentialsError,
    },
    /// The request never completed on any attempt.
    #[error("the {method} {path} request could not be sent")]
    Transport {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// What the engine reported on the last attempt.
        #[source]
        source: TransportError,
    },
    /// The `http` request could not be built from its parts.
    #[error("the {method} {path} request could not be built")]
    Build {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// What `http` reported.
        #[source]
        source: http::Error,
    },
    /// A header name is not legal on the wire.
    #[error("{header} is not a legal header name")]
    HeaderName {
        /// The rejected name.
        header: String,
        /// What `http` reported.
        #[source]
        source: http::header::InvalidHeaderName,
    },
    /// The credentials cannot form an `Authorization` value; nothing was sent.
    #[error("the credentials for {method} {path} cannot be sent")]
    InvalidCredentials {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// The rule the credentials break.
        #[source]
        source: InvalidCredentials,
    },
    /// A header value is not legal on the wire.
    #[error("the value for the {header} header is not a legal header value")]
    HeaderValue {
        /// The header being set.
        header: String,
        /// What `http` reported.
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    /// The typed surface was asked to send a typed body as something other
    /// than canonical JSON.
    #[error("the typed client sends canonical JSON, not {requested}")]
    UnsupportedMediaType {
        /// The `Content-Type` that was requested.
        requested: String,
    },
    /// The request body did not serialize.
    #[error("the request body could not be serialized")]
    Serialize {
        /// What the serializer reported.
        #[source]
        source: serde_json::Error,
    },
    /// The service refused the credentials (`401`).
    #[error("the openEHR service refused the credentials for {method} {path}")]
    Unauthorized {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// The `WWW-Authenticate` challenge, when the service sent one.
        challenge: Option<String>,
        /// The answer body as the service sent it.
        body: ErrorBody,
    },
    /// The service refuses to authorize the request (`403`).
    #[error("the openEHR service refuses to authorize {method} {path}")]
    Forbidden {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// The answer body as the service sent it.
        body: ErrorBody,
    },
    /// The service answered `5xx`, on every attempt within the budget.
    #[error("the openEHR service answered {status} for {method} {path}")]
    ServiceFailure {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// The status of the last attempt.
        status: StatusCode,
        /// The answer body as the service sent it.
        body: ErrorBody,
    },
    /// The service answered a status outside the operation's documented set
    /// — including the general `406`, `415` and `501` the ITS-REST overview
    /// lists for every operation without naming them per operation.
    #[error(
        "the openEHR service answered {status} for {method} {path}, a status outside the operation's documented set"
    )]
    UndocumentedStatus {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// The status that was answered.
        status: StatusCode,
        /// The answer body as the service sent it.
        body: ErrorBody,
    },
    /// The body of a documented answer is not the shape the OAS declares.
    #[error("the {status} body of {method} {path} is not the documented shape")]
    Body {
        /// The HTTP method.
        method: Method,
        /// The operation path.
        path: String,
        /// The status that was answered.
        status: StatusCode,
        /// The defect, with the JSON path that reaches it.
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
}

impl ClientError {
    /// Whether a repeat of the same request could succeed: only a failure
    /// that did not reach the service, or one the service reported as its
    /// own (`5xx`, except `501 Not Implemented`, which a repeat cannot
    /// change), is transient.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { .. } => true,
            Self::ServiceFailure { status, .. } => *status != StatusCode::NOT_IMPLEMENTED,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Credentials, ErrorBody, Request, path_segment};
    use http::Method;

    #[test]
    fn a_path_segment_is_percent_encoded_with_the_pchar_extras_literal() {
        assert_eq!(path_segment(&"Vital Signs"), "Vital%20Signs");
        assert_eq!(path_segment(&"8849182c::system::1"), "8849182c::system::1");
        assert_eq!(path_segment(&"a/b?c#d%3A"), "a%2Fb%3Fc%23d%253A");
        assert_eq!(path_segment(&"user@host"), "user@host");
    }

    #[test]
    fn an_error_body_decodes_the_its_rest_error_and_keeps_the_bytes() {
        let full = ErrorBody::from_bytes(
            br#"{"message":"unknown template","validationErrors":["/content: no template"]}"#
                .to_vec(),
        );
        assert_eq!(full.message(), Some("unknown template"));
        assert_eq!(full.validation_errors(), ["/content: no template"]);
        let prose = ErrorBody::from_bytes(br#"{"message":"malformed body"}"#.to_vec());
        assert_eq!(prose.message(), Some("malformed body"));
        assert!(prose.validation_errors().is_empty());
        let other = ErrorBody::from_bytes(b"<html>gateway</html>".to_vec());
        assert!(other.error().is_none());
        assert_eq!(other.text(), Some("<html>gateway</html>"));
        assert_eq!(other.raw(), b"<html>gateway</html>");
        assert!(ErrorBody::from_bytes(Vec::new()).is_empty());
    }

    #[test]
    fn query_pairs_are_encoded_and_joined() {
        let mut request = Request::new(Method::GET, "/query/aql".to_owned());
        request.query("q", "SELECT c FROM COMPOSITION c");
        request.query("fetch", 10);
        request.query("ehr_id", "7d44/x");
        assert_eq!(
            request.query_string(),
            "q=SELECT%20c%20FROM%20COMPOSITION%20c&fetch=10&ehr_id=7d44%2Fx"
        );
    }

    #[test]
    fn a_typed_body_refuses_a_non_json_content_type() {
        let mut request = Request::new(Method::POST, "/ehr".to_owned());
        let refused = request.json_body("a typed body", Some("application/xml"));
        assert!(refused.is_err());
        assert!(request.body().is_none());
    }

    #[test]
    fn credentials_debug_redacts_the_secret() {
        let basic = Credentials::basic("alice", "hunter2");
        let bearer = Credentials::bearer("eyJ.secret".to_owned());
        assert!(!format!("{basic:?}").contains("hunter2"));
        assert!(!format!("{bearer:?}").contains("secret"));
        assert!(format!("{basic:?}").contains("alice"));
    }

    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "test assertions in the Book's Result-returning shape (.claude/rules/testing.md §Test shapes)"
    )]
    fn credentials_render_the_authorization_value() -> Result<(), Box<dyn std::error::Error>> {
        let basic = Credentials::basic("alice", "hunter2").header_value()?;
        assert_eq!(basic.to_str()?, "Basic YWxpY2U6aHVudGVyMg==");
        assert!(basic.is_sensitive());
        let bearer = Credentials::bearer("tok").header_value()?;
        assert_eq!(bearer.to_str()?, "Bearer tok");
        let padded = Credentials::bearer("mF_9.B5f-4.1JqM+/~==").header_value()?;
        assert_eq!(padded.to_str()?, "Bearer mF_9.B5f-4.1JqM+/~==");
        // A colon in the password is legal: only the first colon separates.
        let colon = Credentials::basic("alice", "a:b").header_value()?;
        assert_eq!(colon.to_str()?, "Basic YWxpY2U6YTpi");
        // RFC 9449 §7.1: a DPoP-bound token travels under the `DPoP` scheme.
        let dpop =
            Credentials::dpop("Kz~8mXK1EalYznwH-LC-1fBAo.4Ljp~zsPE_NeO.gxU").header_value()?;
        assert_eq!(
            dpop.to_str()?,
            "DPoP Kz~8mXK1EalYznwH-LC-1fBAo.4Ljp~zsPE_NeO.gxU"
        );
        assert!(dpop.is_sensitive());
        assert!(matches!(
            Credentials::dpop("not a token").header_value(),
            Err(super::InvalidCredentials::NotToken68)
        ));
        assert!(!format!("{:?}", Credentials::dpop("secret-dpop")).contains("secret"));
        Ok(())
    }

    #[test]
    fn credentials_breaking_rfc_7617_or_6750_are_refused() {
        use super::{BasicPart, InvalidCredentials};
        let refused = |c: Credentials| c.header_value().err();
        assert!(matches!(
            refused(Credentials::basic("al:ice", "pw")),
            Some(InvalidCredentials::ColonInUserId)
        ));
        assert!(matches!(
            refused(Credentials::basic("al\tice", "pw")),
            Some(InvalidCredentials::ControlCharacter(BasicPart::UserId))
        ));
        for password in ["p\u{7f}w", "p\nw", "\0"] {
            assert!(
                matches!(
                    refused(Credentials::basic("alice", password)),
                    Some(InvalidCredentials::ControlCharacter(BasicPart::Password))
                ),
                "{password:?}"
            );
        }
        for token in ["", "=", "a b", "tok\n", "a=b", "t\u{e9}k", "a,b"] {
            assert!(
                matches!(
                    refused(Credentials::bearer(token.to_owned())),
                    Some(InvalidCredentials::NotB64Token)
                ),
                "{token:?}"
            );
        }
        // A non-ASCII basic part is not a CTL: base64 makes it header-legal.
        assert!(refused(Credentials::basic("j\u{f6}rg", "p\u{e9}")).is_none());
        let shown = InvalidCredentials::ControlCharacter(BasicPart::Password).to_string();
        assert!(!shown.contains("p\nw"), "{shown}");
    }
}
