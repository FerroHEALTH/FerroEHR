// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authentication: HTTP Basic + OAuth2/OIDC bearer, plus the coarse RBAC gate.
//!
//! ITS-REST leaves authentication a `SHOULD` and mandates no scheme
//! (`overview/Requests_and_responses.md` §Authentication and authorization), but
//! when a framework IS present the service MUST use `WWW-Authenticate` and
//! return `401`/`403`/`407` as applicable — the bar this middleware meets:
//! missing or invalid credentials are a `401` with a challenge, and an
//! authenticated-but-refused caller a `403` without one. The CNF security suites
//! (`CNF/tests/platform/robot/SECURITY_TESTS/`) are the conformance reference.
//!
//! No openEHR spec governs role-based authorization — the SM places it out of
//! band, so the coarse RBAC gate is our own enterprise extension. It runs as one
//! axum middleware over the API router: a successful authentication puts a
//! [`Principal`] into the request extensions, and when an
//! [`crate::extensions::access::authz::AuthzHandle`] is wired the gate then
//! judges the matched operation's class, denying with a `403` that carries the
//! principal so the ATNA audit layer records it.
//!
//! Between the two, the `assurance` gate judges a patient-data request by the
//! assurance level and the natural person its credential carries, and publishes
//! what it read for the access record.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 7): RFC 7519 leaves the claim set open; \
              decided-on claims lift into typed fields"
)]

pub(crate) mod assurance;
mod basic;
mod jwt;

use ferroehr::config::auth::AuthConfig;

use std::sync::Arc;

use crate::extensions::access::authz::roles::RbacDecision;
use axum::extract::{FromRequestParts, MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::request::Parts;
use http::{HeaderValue, StatusCode, header};

use openehr_its::rest::runtime::ApiError;

use crate::extensions::access::authz::AuthzHandle;
use crate::overview::error::RestError;
use assurance::{PatientDataGate, PatientDataRefusal};
use ferroehr::system_log::event::ActorAuthentication;
use jsonwebtoken::Algorithm;
use jsonwebtoken::errors::ErrorKind;
use jwt::JwtValidator;

/// The state the [`middleware`] runs on: the authenticator, the optional
/// authorization handle (`None` is authentication-only) and the optional
/// patient-data gate.
#[derive(Clone)]
pub(crate) struct AuthLayer {
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) authz: Option<Arc<AuthzHandle>>,
    /// The patient-data assurance and natural-person gate; `None` when no
    /// bearer mechanism is configured.
    pub(crate) patient_data: Option<Arc<PatientDataGate>>,
}

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Token `sub` / Basic username.
    pub subject: String,
    /// `OAuth2` scopes granted to the caller (empty for Basic).
    pub scopes: Vec<String>,
    /// Roles granted to the caller, normalized to upper-case: for Bearer from
    /// the configured JWT claim paths (defaulting to the RFC 9068 §2.2.3.1
    /// carriers), for Basic from the user's configured role list. An OAuth2
    /// scope is not a role — see [`Self::scopes`].
    pub roles: Vec<String>,
    /// The retained, validated JWT claim set (Bearer only; empty for Basic), so
    /// the ABAC layer can resolve subject attributes without re-parsing the
    /// token.
    pub claims: serde_json::Map<String, serde_json::Value>,
    /// Which mechanism authenticated the caller.
    pub method: AuthMethod,
}

/// The mechanism that authenticated a [`Principal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// HTTP Basic: a username/password pair verified against the Argon2 store.
    Basic,
    /// OAuth2/OIDC Bearer: a JWT validated against the issuer's JWKS.
    Bearer,
}

/// A successful authentication: the [`Principal`] plus whether this request
/// performed a genuine credential verification.
///
/// `fresh` is `true` only for a Basic verified-credential cache miss, where the
/// KDF actually ran. A cache hit is a continuing session and a Bearer request is
/// federated, so both are `false`. It decides whether to emit an IHE ATNA login
/// record, which marks authentication events rather than requests.
#[derive(Debug, Clone)]
pub(crate) struct Authenticated {
    pub(crate) principal: Principal,
    pub(crate) fresh: bool,
}

/// Response-extension marker set by [`middleware`] when a request carried a
/// genuine authentication event (see [`Authenticated::fresh`]), which the
/// outermost ATNA audit layer reads to emit the login record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FreshAuthentication;

/// Why a bearer token was refused.
///
/// RFC 6750 §3.1 gives every one of these the same `invalid_token` challenge
/// code, so the distinction never reaches the client; it exists for the operator
/// and the cause chain. The variants built from `jsonwebtoken` carry that error
/// as their [`std::error::Error::source`], so a caller can walk to the concrete
/// kind instead of matching another crate's message text.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TokenRejection {
    /// The `exp` claim is in the past, beyond the configured clock-skew leeway.
    #[error("{0}")]
    Expired(#[source] jsonwebtoken::errors::Error),
    /// The `nbf` claim puts the token's validity in the future.
    #[error("{0}")]
    NotYetValid(#[source] jsonwebtoken::errors::Error),
    /// The signature does not verify against the selected key.
    #[error("{0}")]
    Signature(#[source] jsonwebtoken::errors::Error),
    /// The `iss` claim is not the configured issuer.
    #[error("{0}")]
    Issuer(#[source] jsonwebtoken::errors::Error),
    /// The `aud` claim names none of the configured audiences.
    #[error("{0}")]
    Audience(#[source] jsonwebtoken::errors::Error),
    /// The `sub` claim is not a subject the validation admits.
    #[error("{0}")]
    Subject(#[source] jsonwebtoken::errors::Error),
    /// A required claim is absent, or carries the wrong JSON type. The message
    /// names the claim, never its value.
    #[error("{0}")]
    ClaimSet(#[source] jsonwebtoken::errors::Error),
    /// The credential is not a well-formed JWT at all: a bad shape, or base64 /
    /// JSON / UTF-8 that does not decode.
    #[error("{0}")]
    Malformed(#[source] jsonwebtoken::errors::Error),
    /// The header's algorithm disagrees with the decoding key or the validation.
    #[error("{0}")]
    AlgorithmMismatch(#[source] jsonwebtoken::errors::Error),
    /// The key material the validator holds is unusable — an issuer or
    /// deployment misconfiguration rather than a client fault.
    #[error("{0}")]
    KeyMaterial(#[source] jsonwebtoken::errors::Error),
    /// A `jsonwebtoken` failure outside the classified set. Its `ErrorKind` is
    /// `#[non_exhaustive]`, so a crate upgrade can add kinds this mapping has
    /// not yet named.
    #[error("{0}")]
    Unclassified(#[source] jsonwebtoken::errors::Error),
    /// The header's `alg` is outside `auth.oidc.algorithms`.
    #[error("token algorithm {0:?} not accepted")]
    AlgorithmNotAccepted(Algorithm),
    /// The header's `typ` names a media type that is not an access token
    /// (RFC 8725 §3.11).
    #[error("token type `{0}` is not an access token")]
    TokenTypeNotAccessToken(String),
    /// `auth.oidc.require_at_jwt` is set and the token does not claim the
    /// RFC 9068 `at+jwt` profile.
    #[error(
        "the token does not claim the RFC 9068 `at+jwt` profile, which \
         auth.oidc.require_at_jwt demands"
    )]
    AtJwtProfileRequired,
    /// The token claims the `at+jwt` profile but omits a claim RFC 9068 §2.2
    /// then requires. Carries the claim name.
    #[error("an `at+jwt` access token must carry `{0}` (RFC 9068 §2.2)")]
    AtJwtProfileClaimMissing(String),
    /// The token carries no usable `sub`, so the caller would be unattributable
    /// in the ATNA audit trail.
    #[error(
        "token carries no usable `sub` claim; an audit-attributable subject is \
         required"
    )]
    SubjectMissing,
    /// The token's `kid` names no key in the issuer's JWKS.
    #[error("the issuer's JWKS carries no key `{0}`")]
    UnknownKeyId(String),
    /// The token carries no `kid` and no JWKS key can verify its algorithm.
    #[error(
        "the token carries no `kid` and the issuer's JWKS holds no key usable for \
         verifying this algorithm"
    )]
    NoUsableKey,
    /// The token carries no `kid` and several JWKS keys could verify it, so the
    /// key cannot be chosen without guessing.
    #[error(
        "the token carries no `kid` and {0} keys in the issuer's JWKS could verify \
         it — the issuer must identify the key"
    )]
    AmbiguousKey(usize),
    /// The selected JWK cannot be turned into a decoding key.
    #[error("invalid JWK: {0}")]
    UnusableJwk(#[source] jsonwebtoken::errors::Error),
}

impl From<jsonwebtoken::errors::Error> for TokenRejection {
    fn from(error: jsonwebtoken::errors::Error) -> Self {
        match error.kind() {
            ErrorKind::ExpiredSignature => Self::Expired(error),
            ErrorKind::ImmatureSignature => Self::NotYetValid(error),
            ErrorKind::InvalidSignature => Self::Signature(error),
            ErrorKind::InvalidIssuer => Self::Issuer(error),
            ErrorKind::InvalidAudience => Self::Audience(error),
            ErrorKind::InvalidSubject => Self::Subject(error),
            ErrorKind::MissingRequiredClaim(_) | ErrorKind::InvalidClaimFormat(_) => {
                Self::ClaimSet(error)
            }
            ErrorKind::InvalidToken
            | ErrorKind::Base64(_)
            | ErrorKind::Json(_)
            | ErrorKind::Utf8(_) => Self::Malformed(error),
            ErrorKind::InvalidAlgorithm
            | ErrorKind::InvalidAlgorithmName
            | ErrorKind::MissingAlgorithm
            | ErrorKind::UnsupportedAlgorithm => Self::AlgorithmMismatch(error),
            ErrorKind::InvalidEcdsaKey
            | ErrorKind::InvalidEddsaKey
            | ErrorKind::InvalidRsaKey(_)
            | ErrorKind::InvalidKeyFormat => Self::KeyMaterial(error),
            _ => Self::Unclassified(error),
        }
    }
}

impl TokenRejection {
    /// A stable, low-cardinality label naming this rejection.
    ///
    /// Carries no token bytes and no claim value, so it is safe as a structured
    /// log field or a metric label.
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            TokenRejection::Expired(_) => "expired",
            TokenRejection::NotYetValid(_) => "not_yet_valid",
            TokenRejection::Signature(_) => "signature",
            TokenRejection::Issuer(_) => "issuer",
            TokenRejection::Audience(_) => "audience",
            TokenRejection::Subject(_) => "subject",
            TokenRejection::ClaimSet(_) => "claim_set",
            TokenRejection::Malformed(_) => "malformed",
            TokenRejection::AlgorithmMismatch(_) => "algorithm_mismatch",
            TokenRejection::KeyMaterial(_) => "key_material",
            TokenRejection::Unclassified(_) => "unclassified",
            TokenRejection::AlgorithmNotAccepted(_) => "algorithm_not_accepted",
            TokenRejection::TokenTypeNotAccessToken(_) => "token_type",
            TokenRejection::AtJwtProfileRequired => "at_jwt_profile_required",
            TokenRejection::AtJwtProfileClaimMissing(_) => "at_jwt_claim_missing",
            TokenRejection::SubjectMissing => "subject_missing",
            TokenRejection::UnknownKeyId(_) => "unknown_key_id",
            TokenRejection::NoUsableKey => "no_usable_key",
            TokenRejection::AmbiguousKey(_) => "ambiguous_key",
            TokenRejection::UnusableJwk(_) => "unusable_jwk",
        }
    }
}

/// An authentication failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    /// The request carried no credential for any enabled mechanism.
    #[error("no credentials supplied")]
    MissingCredentials,
    /// An unknown user, or a password that does not match the stored Argon2
    /// hash: a credential presented and rejected — RFC 6750 §3.1's
    /// `invalid_token` case for bearer, an ordinary 401 for Basic.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// The `Authorization` header is not a well-formed credential at all: an
    /// unparsable header, an unknown scheme, or a bearer credential outside the
    /// RFC 6750 §2.1 `b64token` grammar.
    ///
    /// RFC 6750 §3.1 gives this its own `invalid_request` code and a `400`,
    /// because it is a request defect rather than a credential decision.
    #[error("malformed authorization header: {0}")]
    MalformedRequest(String),
    /// A bearer token that failed structural, signature, or claim validation.
    ///
    /// The [`TokenRejection`] names which check refused it and carries the
    /// underlying `jsonwebtoken` error as its source; the distinction is an
    /// operator signal, never a wire one.
    #[error("invalid bearer token: {0}")]
    InvalidToken(#[from] TokenRejection),
    /// The issuer's signing keys (JWKS) could not be fetched or parsed, so no
    /// bearer token can be validated.
    #[error("could not resolve signing keys: {0}")]
    KeyResolution(String),
    /// The caller authenticated but is not permitted the request — the 403
    /// branch of the ITS-REST 401-vs-403 split.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// The server could not run credential verification at all, for instance a
    /// panicked blocking Argon2 task. A server fault, never a statement about
    /// the credentials, so it maps to 500 rather than 401.
    #[error("credential verification unavailable: {0}")]
    VerificationUnavailable(String),
}

impl AuthError {
    fn to_api_error(&self) -> ApiError {
        match self {
            AuthError::Forbidden(m) => ApiError::Forbidden(m.clone()),
            AuthError::VerificationUnavailable(m) => {
                crate::overview::error::internal_fault("verify the supplied credentials", m)
            }
            AuthError::MalformedRequest(m) => ApiError::BadRequest(m.clone()),
            // With the issuer's keys unreachable no token can be validated, so
            // the server cannot decide: RFC 9110 §15.6.4 makes that a 503. A 401
            // would tell a caller with a valid token its credential was
            // rejected.
            AuthError::KeyResolution(m) => ApiError::ServiceUnavailable(m.clone()),
            // The body says nothing about why: rendering the rejection would
            // tell an unauthenticated caller whether a token was expired or
            // forged. The reason stays in the log.
            _ => ApiError::Unauthorized("authentication failed".to_owned()),
        }
    }

    /// Returns the RFC 6750 §3.1 `error` code this outcome carries on its
    /// `WWW-Authenticate` challenge, or `None` when it must carry none.
    ///
    /// §3.1: "If the request lacks any authentication information … the resource
    /// server SHOULD NOT include an error code".
    const fn bearer_error_code(&self) -> Option<&'static str> {
        match self {
            AuthError::InvalidCredentials | AuthError::InvalidToken(_) => Some("invalid_token"),
            AuthError::Forbidden(_) => Some("insufficient_scope"),
            AuthError::MalformedRequest(_) => Some("invalid_request"),
            // No credential presented, or a server-side fault: neither is a
            // statement the client can act on by changing its credential.
            AuthError::MissingCredentials
            | AuthError::KeyResolution(_)
            | AuthError::VerificationUnavailable(_) => None,
        }
    }

    /// A stable, low-cardinality label naming this outcome.
    ///
    /// Carries no credential material, so it is safe as a structured log field
    /// or a metric label. A bearer refusal delegates to
    /// [`TokenRejection::label`], so expired-versus-invalid is countable.
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            AuthError::MissingCredentials => "missing_credentials",
            AuthError::InvalidCredentials => "invalid_credentials",
            AuthError::MalformedRequest(_) => "malformed_request",
            AuthError::InvalidToken(rejection) => rejection.label(),
            AuthError::KeyResolution(_) => "key_resolution",
            AuthError::Forbidden(_) => "forbidden",
            AuthError::VerificationUnavailable(_) => "verification_unavailable",
        }
    }
}

/// The configured authenticator: the enabled mechanisms and the coarse admin
/// gate.
pub struct Authenticator {
    config: AuthConfig,
    jwt: Option<JwtValidator>,
    /// Verified Basic-credential cache, keyed by the SHA-256 of the presented
    /// `Authorization` header.
    ///
    /// An entry exists only after a successful Argon2 verification, and the TTL
    /// (`config::AuthConfig::verified_cache_ttl_seconds`) bounds revocation lag;
    /// `None` disables it. Argon2 costs tens of milliseconds of CPU per call by
    /// design, so without this a busy client's request rate is capped by the
    /// KDF's work factor.
    verified: Option<moka::future::Cache<[u8; 32], Principal>>,
    /// KDF verifications actually performed: a test seam and a cheap
    /// operational signal.
    kdf_verifications: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authenticator")
            .field("enabled", &self.config.enabled)
            .field("basic", &self.config.basic.is_some())
            .field("oidc", &self.jwt.is_some())
            .finish_non_exhaustive()
    }
}

impl Authenticator {
    /// Builds an authenticator from configuration, constructing the bearer
    /// validator if OIDC is configured.
    ///
    /// # Errors
    /// Returns a message if the OIDC key material/algorithms are invalid.
    pub fn new(config: AuthConfig) -> Result<Arc<Self>, String> {
        Self::with_role_claims(
            config,
            ferroehr::config::authz::RbacConfig::default().role_claims,
        )
    }

    /// Builds an authenticator with explicit RBAC role-claim paths, used when an
    /// [`crate::extensions::access::authz::AuthzHandle`] is wired;
    /// [`Authenticator::new`] defaults them.
    ///
    /// # Errors
    /// Returns a message if the OIDC key material/algorithms are invalid.
    pub fn with_role_claims(
        config: AuthConfig,
        role_claims: Vec<String>,
    ) -> Result<Arc<Self>, String> {
        let jwt = match &config.oidc {
            Some(oidc) => Some(JwtValidator::with_role_claims(oidc, role_claims)?),
            None => None,
        };
        let verified = (config.verified_cache_ttl_seconds > 0).then(|| {
            moka::future::Cache::builder()
                .max_capacity(1024)
                .time_to_live(std::time::Duration::from_secs(
                    config.verified_cache_ttl_seconds,
                ))
                .build()
        });
        Ok(Arc::new(Self {
            config,
            jwt,
            verified,
            kdf_verifications: std::sync::atomic::AtomicU64::new(0),
        }))
    }

    pub(crate) fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// The authentication configuration this authenticator was built from.
    pub(crate) fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// Builds the `WWW-Authenticate` challenge advertising the enabled
    /// mechanisms.
    pub(crate) fn challenge(&self, outcome: Option<&AuthError>) -> HeaderValue {
        let error = outcome.and_then(AuthError::bearer_error_code);
        let mut parts: Vec<String> = Vec::new();
        if self.config.basic.is_some() {
            // RFC 7617 §2.1: `charset="UTF-8"` names the credential encoding,
            // and is the RFC's only legal value.
            parts.push(r#"Basic realm="ferroehr", charset="UTF-8""#.to_owned());
        }
        if self.jwt.is_some() {
            // RFC 6750 §3: the challenge carries `realm` and, when the client
            // can act on the failure, an `error` code from §3.1.
            let bearer = match error {
                Some(code) => format!(r#"Bearer realm="ferroehr", error="{code}""#),
                None => r#"Bearer realm="ferroehr""#.to_owned(),
            };
            parts.push(bearer);
        }
        if parts.is_empty() {
            // Unreachable in a booted server (`auth.enabled` without a mechanism
            // is a boot error), but a challenge must never be empty.
            parts.push(r#"Basic realm="ferroehr", charset="UTF-8""#.to_owned());
        }
        HeaderValue::from_str(&parts.join(", "))
            .unwrap_or_else(|_| HeaderValue::from_static(r#"Basic realm="ferroehr""#))
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "every failure on the credential path collapses to one opaque \
                  outcome deliberately: a caller must not learn from the 401 which \
                  part of the credential was rejected"
    )]
    pub(crate) async fn authenticate(
        &self,
        headers: &http::HeaderMap,
    ) -> Result<Authenticated, AuthError> {
        let auth = headers
            .get(header::AUTHORIZATION)
            .ok_or(AuthError::MissingCredentials)?;
        let raw = auth.to_str().map_err(|_| {
            AuthError::MalformedRequest(
                "the Authorization header is not valid ISO-8859-1 text".to_owned(),
            )
        })?;
        let scheme = raw
            .split_whitespace()
            .next()
            .ok_or_else(|| {
                AuthError::MalformedRequest("the Authorization header is empty".to_owned())
            })?
            .to_ascii_lowercase();

        match scheme.as_str() {
            "basic" => {
                let cfg = self
                    .config
                    .basic
                    .as_ref()
                    .ok_or(AuthError::InvalidCredentials)?;
                // Keyed by the SHA-256 of the presented header, never the
                // plaintext. A miss runs Argon2 on the blocking pool, so the
                // async workers are never parked on CPU-bound hashing.
                let key: [u8; 32] = {
                    use sha2::Digest as _;
                    sha2::Sha256::digest(auth.as_bytes()).into()
                };
                if let Some(cache) = &self.verified
                    && let Some(principal) = cache.get(&key).await
                {
                    // A cache hit is a continuing session, not a new
                    // authentication event.
                    return Ok(Authenticated {
                        principal,
                        fresh: false,
                    });
                }
                self.kdf_verifications
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let principal = {
                    let auth = auth.clone();
                    let cfg = cfg.clone();
                    tokio::task::spawn_blocking(move || basic::verify(&auth, &cfg))
                        .await
                        .map_err(|join_error| {
                            AuthError::VerificationUnavailable(format!(
                                "password-verification task failed: {join_error}"
                            ))
                        })??
                };
                if let Some(cache) = &self.verified {
                    cache.insert(key, principal.clone()).await;
                }
                // The KDF actually ran: this request is a genuine authentication.
                Ok(Authenticated {
                    principal,
                    fresh: true,
                })
            }
            "bearer" => {
                let validator = self.jwt.as_ref().ok_or(AuthError::InvalidCredentials)?;
                let token = bearer_credential(raw)?;
                // Federated: the authentication event happened at the OIDC
                // provider, so a per-request login record is never minted here.
                validator
                    .validate(token)
                    .await
                    .map(|principal| Authenticated {
                        principal,
                        fresh: false,
                    })
            }
            other => Err(AuthError::MalformedRequest(format!(
                "unsupported authentication scheme `{other}`"
            ))),
        }
    }
}

/// Extracts the bearer credential from an `Authorization` header value,
/// enforcing the RFC 6750 §2.1 grammar.
///
/// The RFC is exact: `credentials = "Bearer" 1*SP b64token` with
/// `b64token = 1*( ALPHA / DIGIT / "-" / "." / "_" / "~" / "+" / "/" ) *"="`.
/// A value outside it is a malformed request rather than a rejected credential,
/// so it answers 400 (§3.1 `invalid_request`).
///
/// # Errors
/// [`AuthError::MalformedRequest`] when the scheme is not followed by at least
/// one space and a non-empty `b64token`, or when the token carries a character
/// the grammar does not admit.
fn bearer_credential(raw: &str) -> Result<&str, AuthError> {
    let rest = raw
        .strip_prefix("Bearer")
        .or_else(|| raw.strip_prefix("bearer"))
        .ok_or_else(|| AuthError::MalformedRequest("expected the Bearer scheme".to_owned()))?;
    // `1*SP`: at least one space, and the RFC names SP specifically — a tab or a
    // newline is not a legal separator.
    if !rest.starts_with(' ') {
        return Err(AuthError::MalformedRequest(
            "the Bearer scheme must be followed by a single space and the token".to_owned(),
        ));
    }
    let token = rest.trim_start_matches(' ');
    if token.is_empty() {
        return Err(AuthError::MalformedRequest(
            "the Bearer credential carries no token".to_owned(),
        ));
    }
    // `*"="` is a TRAILING run only, so padding may not appear mid-token.
    let body = token.trim_end_matches('=');
    if !body
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/'))
    {
        return Err(AuthError::MalformedRequest(
            "the Bearer token is outside the RFC 6750 b64token grammar".to_owned(),
        ));
    }
    Ok(token)
}

tokio::task_local! {
    /// The authenticated principal for the request currently being handled.
    static REQUEST_PRINCIPAL: Option<Principal>;
}

/// Returns the authenticated principal for the current request, if any.
///
/// Set by `middleware` for the duration of request handling, so downstream
/// layers read it without the principal being threaded through the generated
/// trait signatures. `None` when unauthenticated or called outside a request.
#[must_use]
pub fn current_principal() -> Option<Principal> {
    REQUEST_PRINCIPAL.try_with(Clone::clone).ok().flatten()
}

/// Counts one refusal, tagged by the mechanism that produced it and the status
/// the caller receives.
fn count_auth_failure(mechanism: &'static str, status: &'static str) {
    ferroehr::telemetry::metrics::metrics().auth_failures.add(
        1,
        &[
            opentelemetry::KeyValue::new("mechanism", mechanism),
            opentelemetry::KeyValue::new("status", status),
        ],
    );
}

/// The authentication and RBAC middleware, attached to the API router; the
/// public endpoints are mounted outside it.
pub(crate) async fn middleware(
    State(layer): State<AuthLayer>,
    mut req: Request,
    next: Next,
) -> Response {
    let auth = &layer.authenticator;
    if !auth.enabled() {
        return REQUEST_PRINCIPAL.scope(None, next.run(req)).await;
    }

    match auth.authenticate(req.headers()).await {
        Ok(Authenticated { principal, fresh }) => {
            // How the person authenticated is judged before what they may do:
            // a step-up refusal is an authentication outcome (RFC 9470 §3).
            let actor = match layer.patient_data.as_deref() {
                None => ActorAuthentication::default(),
                Some(gate) => {
                    let matched = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);
                    match gate.judge(&principal, matched) {
                        Ok(actor) => actor,
                        Err((refusal, actor)) => {
                            return patient_data_refusal(gate, &principal, refusal, actor);
                        }
                    }
                }
            };
            if let Some(mut refusal) = rbac_refusal(&layer, &req, &principal) {
                refusal.extensions_mut().insert(actor);
                return refusal;
            }
            req.extensions_mut().insert(principal.clone());
            let for_audit = principal.clone();
            let committer = committer_identity(&for_audit);
            // The roles ride beside the committer identity for the request
            // scope, so a record the service layer emits by itself carries
            // them too (#3239).
            let roles = for_audit.roles.clone();
            let mut resp = REQUEST_PRINCIPAL
                .scope(
                    Some(principal),
                    ferroehr::system_log::access_context::with_roles(
                        roles,
                        ferroehr::service::committer::with_committer(
                            Some(committer),
                            next.run(req),
                        ),
                    ),
                )
                .await;
            // The outer ATNA audit layer cannot observe request-extension
            // mutations, so the principal is republished onto the response.
            resp.extensions_mut().insert(for_audit);
            resp.extensions_mut().insert(actor);
            if fresh {
                resp.extensions_mut().insert(FreshAuthentication);
            }
            resp
        }
        Err(e) => refusal_response(auth, req.headers(), &e),
    }
}

/// Runs the RBAC gate over an authenticated caller: the matched operation's
/// class against the caller's roles, then the read-only restriction.
///
/// A principal carrying the configured read-only role is refused on every write
/// operation, overriding any grant. Both denials produce the same `403`,
/// attributed to the caller so the ATNA audit layer records it. No openEHR spec
/// governs role semantics — our own design/extension.
fn rbac_refusal(layer: &AuthLayer, req: &Request, principal: &Principal) -> Option<Response> {
    let rbac = layer.authz.as_deref().and_then(AuthzHandle::rbac)?;
    let matched = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned());
    let class = rbac.class_for(req.method(), matched.as_deref());
    let decision = match rbac.decide(class, &principal.roles) {
        RbacDecision::Deny(reason) => RbacDecision::Deny(reason),
        RbacDecision::Allow => {
            let is_write = rbac.is_write_for(req.method(), matched.as_deref());
            rbac.decide_readonly(is_write, &principal.roles)
        }
    };
    let RbacDecision::Deny(reason) = decision else {
        return None;
    };
    count_auth_failure(mechanism_label(principal.method), "403");
    let mut resp = RestError(ApiError::Forbidden(reason)).into_response();
    resp.extensions_mut().insert(principal.clone());
    Some(resp)
}

/// Builds the response for a patient-data request the gate refused, attributed
/// to the caller so the audit layer records who was refused and how they
/// authenticated.
///
/// An insufficient assurance level is a `401` carrying the RFC 9470 §3 step-up
/// challenge (`insufficient_user_authentication` with the accepted
/// `acr_values`); a credential naming no natural person is a `403`, the
/// authenticated-but-refused branch of the ITS-REST split, without a challenge
/// as the RBAC refusal has none.
fn patient_data_refusal(
    gate: &PatientDataGate,
    principal: &Principal,
    refusal: PatientDataRefusal,
    actor: ActorAuthentication,
) -> Response {
    let mechanism = mechanism_label(principal.method);
    let mut resp = match refusal {
        PatientDataRefusal::InsufficientAssurance => {
            count_auth_failure(mechanism, "401");
            tracing::warn!(
                mechanism,
                reason = "insufficient_user_authentication",
                "patient-data request refused"
            );
            let mut resp = RestError(ApiError::Unauthorized(
                "patient data requires a higher authentication assurance level".to_owned(),
            ))
            .into_response();
            resp.headers_mut()
                .insert(header::WWW_AUTHENTICATE, gate.step_up_challenge());
            resp
        }
        PatientDataRefusal::NoNaturalPerson => {
            count_auth_failure(mechanism, "403");
            tracing::warn!(
                mechanism,
                reason = "no_natural_person",
                "patient-data request refused"
            );
            RestError(ApiError::Forbidden(
                "patient data requires a credential that names a natural person, or a \
                 client credential acting for a named professional"
                    .to_owned(),
            ))
            .into_response()
        }
    };
    resp.extensions_mut().insert(principal.clone());
    resp.extensions_mut().insert(actor);
    resp
}

/// Returns the platform committer identity published for the service layer.
///
/// A write whose request carried no committal headers is attributed to the
/// authenticated principal rather than the system identity
/// (`AUDIT_DETAILS.committer` 1..1 — RM common master04 §Audit Details). A
/// Bearer subject was minted by the identity provider, so the audit records the
/// validated token issuer; Basic credentials are local and carry none.
fn committer_identity(principal: &Principal) -> ferroehr::service::committer::CommitterIdentity {
    ferroehr::service::committer::CommitterIdentity {
        subject: principal.subject.clone(),
        id_type: match principal.method {
            AuthMethod::Basic => "basic",
            AuthMethod::Bearer => "oauth2",
        },
        issuer: match principal.method {
            AuthMethod::Basic => None,
            AuthMethod::Bearer => principal
                .claims
                .get("iss")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        },
    }
}

/// Builds the response for a refused authentication, metered and logged.
///
/// A refusal that presented no credential is routine; a presented-and-rejected
/// one is the operator's signal. Neither record carries the token or a claim
/// value. ITS-REST §Authentication and authorization makes the
/// `WWW-Authenticate` challenge a MUST on a `401`, and RFC 6750 §3 carries one
/// on a bearer `403` too, where it names what the client lacks.
fn refusal_response(auth: &Authenticator, headers: &header::HeaderMap, e: &AuthError) -> Response {
    let api = e.to_api_error();
    let status = api.status();
    let mechanism = scheme_label(headers);
    count_auth_failure(
        mechanism,
        if status == StatusCode::FORBIDDEN {
            "403"
        } else {
            "401"
        },
    );
    let reason = e.label();
    if matches!(e, AuthError::MissingCredentials) {
        tracing::debug!(mechanism, reason, "authentication refused");
    } else {
        tracing::warn!(mechanism, reason, detail = %e, "authentication refused");
    }
    let needs_challenge = matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN);
    let challenge = auth.challenge(Some(e));
    let mut resp = RestError(api).into_response();
    if needs_challenge {
        resp.headers_mut()
            .insert(header::WWW_AUTHENTICATE, challenge);
    }
    resp
}

/// The RFC 6750 §2.1 grammar and the challenge shapes, asserted per clause.
#[cfg(test)]
mod bearer_grammar_tests {
    use super::{AuthError, TokenRejection, bearer_credential};

    #[test]
    fn bearer_credentials_follow_rfc6750_abnf() {
        // `credentials = "Bearer" 1*SP b64token`
        assert_eq!(
            bearer_credential("Bearer abc.def.ghi").expect("ok"),
            "abc.def.ghi"
        );
        assert_eq!(bearer_credential("bearer abc").expect("ok"), "abc");
        // `1*SP` — more than one space is legal.
        assert_eq!(bearer_credential("Bearer   abc").expect("ok"), "abc");
        // Trailing `*"="` padding is part of the grammar.
        assert_eq!(bearer_credential("Bearer abc==").expect("ok"), "abc==");

        for malformed in [
            "Bearer",         // no separator, no token
            "Bearer ",        // separator, no token
            "Bearer\tabc",    // a tab is not SP
            "Bearer abc def", // a space inside the token
            "Bearer abc?def", // `?` is outside b64token
            "Bearer a=bc",    // interior padding
            "Basic abc",      // wrong scheme for this parser
        ] {
            assert!(
                matches!(
                    bearer_credential(malformed),
                    Err(AuthError::MalformedRequest(_))
                ),
                "`{malformed}` must be a malformed REQUEST, not a rejected credential",
            );
        }
    }

    /// RFC 6750 §3.1 assigns each outcome its own code, and deliberately assigns
    /// NONE when no credential was presented: an unauthenticated first request
    /// has not made a mistake yet.
    #[test]
    fn challenge_error_codes_follow_rfc6750() {
        assert_eq!(AuthError::MissingCredentials.bearer_error_code(), None);
        assert_eq!(
            AuthError::InvalidCredentials.bearer_error_code(),
            Some("invalid_token")
        );
        assert_eq!(
            AuthError::InvalidToken(TokenRejection::NoUsableKey).bearer_error_code(),
            Some("invalid_token")
        );
        assert_eq!(
            AuthError::Forbidden("x".to_owned()).bearer_error_code(),
            Some("insufficient_scope")
        );
        assert_eq!(
            AuthError::MalformedRequest("x".to_owned()).bearer_error_code(),
            Some("invalid_request")
        );
        // A server-side fault is not a statement about the credential.
        assert_eq!(
            AuthError::KeyResolution("x".to_owned()).bearer_error_code(),
            None
        );
    }

    /// RFC 9110 §15.6.4: an unreachable issuer means the server cannot decide,
    /// which is a 503 — never a 401 telling a caller its valid token was
    /// rejected.
    #[test]
    fn unreachable_issuer_is_503_not_401() {
        let api = AuthError::KeyResolution("jwks unreachable".to_owned()).to_api_error();
        assert_eq!(api.status(), http::StatusCode::SERVICE_UNAVAILABLE);
    }

    /// A malformed header is a request defect (400), not a credential decision
    /// (401) — RFC 6750 §3.1 `invalid_request`.
    #[test]
    fn malformed_authorization_header_is_400_invalid_request() {
        let api = AuthError::MalformedRequest("bad".to_owned()).to_api_error();
        assert_eq!(api.status(), http::StatusCode::BAD_REQUEST);
    }
}

/// The `mechanism` metric label for a known authenticated principal.
fn mechanism_label(method: AuthMethod) -> &'static str {
    match method {
        AuthMethod::Basic => "basic",
        AuthMethod::Bearer => "bearer",
    }
}

/// The `mechanism` metric label derived from the request's `Authorization`
/// scheme (for failures where no principal was established).
fn scheme_label(headers: &http::HeaderMap) -> &'static str {
    match headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split_whitespace().next())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("basic") => "basic",
        Some("bearer") => "bearer",
        _ => "none",
    }
}

/// Extractor for handlers and the service layer to read the authenticated
/// caller, yielding a 401 when no principal is present.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser(pub Principal);

impl<S: Sync> FromRequestParts<S> for AuthenticatedUser {
    type Rejection = RestError;

    // The trait declares an `async fn`; an impl may satisfy it with a plain fn
    // returning a future, which is what this reads-one-extension body is
    // (https://doc.rust-lang.org/reference/items/traits.html).
    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> {
        std::future::ready(
            parts
                .extensions
                .get::<Principal>()
                .cloned()
                .map(AuthenticatedUser)
                .ok_or_else(|| {
                    RestError(ApiError::Unauthorized("authentication required".to_owned()))
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::PasswordHasher;
    use argon2::{Argon2, PasswordHash};
    use ferroehr::config::auth::{BasicConfig, BasicUser, OidcConfig};

    fn hash(pw: &str) -> String {
        Argon2::default()
            .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
            .unwrap()
            .to_string()
    }

    fn basic_only() -> Arc<Authenticator> {
        Authenticator::new(AuthConfig {
            enabled: true,
            basic: Some(BasicConfig {
                users: vec![BasicUser {
                    username: "alice".to_owned(),
                    password_hash: ferroehr::config::secret::Secret::new(hash("pw")),
                    password_hash_file: None,
                    roles: vec!["USER".to_owned()],
                }],
            }),
            oidc: None,
            ..AuthConfig::default()
        })
        .unwrap()
    }

    fn hmac_oidc() -> Arc<Authenticator> {
        Authenticator::new(AuthConfig {
            enabled: true,
            basic: None,
            oidc: Some(OidcConfig {
                issuer: "https://issuer.example".to_owned(),
                audiences: vec![],
                algorithms: vec!["HS256".to_owned()],
                hmac_secret: Some(ferroehr::config::secret::Secret::new("secret".to_owned())),
                jwks_json: None,
                ..OidcConfig::default()
            }),
            ..AuthConfig::default()
        })
        .unwrap()
    }

    fn headers(auth: &str) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(auth).unwrap());
        h
    }

    #[tokio::test]
    async fn verified_credential_cache_skips_the_kdf_on_a_hit() {
        let auth = basic_only();
        let h = headers("Basic YWxpY2U6cHc="); // alice:pw
        let first = auth.authenticate(&h).await.expect("first verifies");
        let second = auth.authenticate(&h).await.expect("second hits the cache");
        assert!(first.fresh, "the KDF ran → a genuine authentication event");
        assert!(
            !second.fresh,
            "a cache hit is a continuing session, not a new authentication"
        );
        assert_eq!(
            auth.kdf_verifications
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "one KDF verification for two requests with the same credential"
        );

        // A different credential is a different key — it must NOT hit the
        // cached entry, and the wrong password still fails.
        let bad = headers("Basic YWxpY2U6d3Jvbmc="); // alice:wrong
        auth.authenticate(&bad).await.expect_err("wrong password");
        assert_eq!(
            auth.kdf_verifications
                .load(std::sync::atomic::Ordering::Relaxed),
            2,
            "the wrong credential paid its own (failed) verification"
        );
    }

    #[tokio::test]
    async fn verified_cache_ttl_zero_disables_caching() {
        let mut cfg = AuthConfig {
            enabled: true,
            basic: Some(BasicConfig {
                users: vec![BasicUser {
                    username: "alice".to_owned(),
                    password_hash: ferroehr::config::secret::Secret::new(hash("pw")),
                    password_hash_file: None,
                    roles: vec!["USER".to_owned()],
                }],
            }),
            oidc: None,
            ..AuthConfig::default()
        };
        cfg.verified_cache_ttl_seconds = 0;
        let auth = Authenticator::new(cfg).unwrap();
        let h = headers("Basic YWxpY2U6cHc=");
        auth.authenticate(&h).await.expect("ok");
        auth.authenticate(&h).await.expect("ok");
        assert_eq!(
            auth.kdf_verifications
                .load(std::sync::atomic::Ordering::Relaxed),
            2,
            "TTL 0 verifies every request"
        );
    }

    #[tokio::test]
    async fn missing_credentials() {
        let err = basic_only()
            .authenticate(&http::HeaderMap::new())
            .await
            .expect_err("reject");
        assert!(matches!(err, AuthError::MissingCredentials));
        assert_eq!(err.to_api_error().status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_basic() {
        // "alice:pw" base64 = YWxpY2U6cHc=
        let p = basic_only()
            .authenticate(&headers("Basic YWxpY2U6cHc="))
            .await
            .expect("ok");
        assert_eq!(p.principal.subject, "alice");
        assert!(
            p.fresh,
            "a first Basic verification is a genuine authentication"
        );
    }

    #[tokio::test]
    async fn bearer_without_oidc_is_rejected() {
        let err = basic_only()
            .authenticate(&headers("Bearer sometoken"))
            .await
            .expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[tokio::test]
    async fn wrong_password_is_401_not_500() {
        let err = basic_only()
            .authenticate(&headers("Basic YWxpY2U6d3Jvbmc=")) // "alice:wrong"
            .await
            .expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
        assert_eq!(err.to_api_error().status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn verification_unavailable_is_500_not_401() {
        // A JoinError from the blocking Argon2 task is a SERVER fault: it must
        // surface as 500 Internal, never as 401 InvalidCredentials — a server
        // failure misreported as a client error is the silent-wrong-answer
        // class this crate's error mapping exists to prevent.
        let err = AuthError::VerificationUnavailable("task panicked".to_owned());
        assert_eq!(
            err.to_api_error().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn phc_hash_is_parseable() {
        // Guards the fixed-salt test helper against argon2 format drift.
        assert!(PasswordHash::new(&hash("pw")).is_ok());
    }

    #[tokio::test]
    async fn bearer_smoke_with_hmac() {
        // A malformed token is rejected as an invalid token (not missing creds).
        let err = hmac_oidc()
            .authenticate(&headers("Bearer not-a-jwt"))
            .await
            .expect_err("reject");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::Malformed(_))),
            "got {err:?}"
        );
    }

    /// One value of every [`TokenRejection`] variant, for the wire-identity
    /// assertion below.
    fn every_token_rejection() -> Vec<TokenRejection> {
        let cause = || jsonwebtoken::errors::Error::from(ErrorKind::ExpiredSignature);
        vec![
            TokenRejection::Expired(cause()),
            TokenRejection::NotYetValid(cause()),
            TokenRejection::Signature(cause()),
            TokenRejection::Issuer(cause()),
            TokenRejection::Audience(cause()),
            TokenRejection::Subject(cause()),
            TokenRejection::ClaimSet(cause()),
            TokenRejection::Malformed(cause()),
            TokenRejection::AlgorithmMismatch(cause()),
            TokenRejection::KeyMaterial(cause()),
            TokenRejection::Unclassified(cause()),
            TokenRejection::AlgorithmNotAccepted(Algorithm::HS256),
            TokenRejection::TokenTypeNotAccessToken("JWT+ID".to_owned()),
            TokenRejection::AtJwtProfileRequired,
            TokenRejection::AtJwtProfileClaimMissing("jti".to_owned()),
            TokenRejection::SubjectMissing,
            TokenRejection::UnknownKeyId("k1".to_owned()),
            TokenRejection::NoUsableKey,
            TokenRejection::AmbiguousKey(2),
            TokenRejection::UnusableJwk(cause()),
        ]
    }

    /// RFC 6750 §3.1 gives ONE code to every rejected token — `invalid_token`
    /// covers a token that "is expired, revoked, malformed, or invalid for
    /// other reasons" — so neither the status nor the challenge a client sees
    /// may vary with the reason the server classified internally.
    #[test]
    fn every_token_rejection_renders_one_status_and_one_challenge() {
        let auth = hmac_oidc();
        let mut challenges: Vec<HeaderValue> = Vec::new();
        for rejection in every_token_rejection() {
            let err = AuthError::InvalidToken(rejection);
            assert_eq!(
                err.to_api_error().status(),
                StatusCode::UNAUTHORIZED,
                "{err:?}"
            );
            assert_eq!(err.bearer_error_code(), Some("invalid_token"), "{err:?}");
            challenges.push(auth.challenge(Some(&err)));
        }
        let first = challenges.first().expect("rejection kinds exist").clone();
        assert!(
            challenges.iter().all(|c| *c == first),
            "the challenge must not vary with the refusal reason: {challenges:?}"
        );
        assert_eq!(
            first,
            HeaderValue::from_static(r#"Bearer realm="ferroehr", error="invalid_token""#)
        );
    }

    /// Collects every tracing EVENT's fields, so the refusal record can be read
    /// back field by field.
    #[derive(Clone, Default)]
    struct EventCapture(Arc<std::sync::Mutex<Vec<std::collections::BTreeMap<String, String>>>>);

    /// Renders one event's fields into `name -> value`.
    struct FieldSink(std::collections::BTreeMap<String, String>);

    impl tracing::field::Visit for FieldSink {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for EventCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut sink = FieldSink(std::collections::BTreeMap::new());
            event.record(&mut sink);
            self.0.lock().expect("lock").push(sink.0);
        }
    }

    /// The refusal reason must reach the LOG as its own field — countable
    /// rather than greppable — and neither the token nor a claim value may
    /// appear anywhere in the record.
    #[tokio::test]
    async fn a_refused_token_logs_its_reason_and_never_the_token() {
        use tracing_subscriber::layer::SubscriberExt as _;

        const TOKEN: &str = "not-a-jwt-at-all";

        let capture = EventCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let layer = AuthLayer {
            authenticator: hmac_oidc(),
            authz: None,
            patient_data: None,
        };
        let app = axum::Router::new()
            .route("/x", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(layer, middleware));

        let response = tower::ServiceExt::oneshot(
            app,
            Request::builder()
                .uri("/x")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let events = capture.0.lock().expect("lock").clone();
        let refusal = events
            .iter()
            .find(|fields| {
                fields
                    .get("message")
                    .is_some_and(|m| m == "authentication refused")
            })
            .expect("the refusal is logged");
        assert_eq!(
            refusal.get("reason").map(String::as_str),
            Some("malformed"),
            "{refusal:?}"
        );
        assert_eq!(
            refusal.get("mechanism").map(String::as_str),
            Some("bearer"),
            "{refusal:?}"
        );
        for fields in &events {
            for (name, value) in fields {
                assert!(
                    !value.contains(TOKEN),
                    "the credential leaked into the `{name}` field: {value}"
                );
            }
        }
    }

    /// Each classified reason gets its own stable label, so a burst of one kind
    /// is countable in the logs rather than greppable out of a message.
    #[test]
    fn token_rejection_labels_are_distinct() {
        let mut labels: Vec<&'static str> = every_token_rejection()
            .iter()
            .map(TokenRejection::label)
            .collect();
        let total = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), total, "two rejection kinds share a label");
    }
}
