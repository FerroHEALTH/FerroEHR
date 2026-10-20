// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authentication over the `ferrohealth-smart` crate, plus the coarse RBAC gate
//! and the SMART on openEHR route requirements.
//!
//! ITS-REST leaves authentication a `SHOULD` and mandates no scheme
//! (`overview/Requests_and_responses.md` §Authentication and authorization), but
//! when a framework IS present the service MUST use `WWW-Authenticate` and
//! return `401`/`403`/`407` as applicable — the bar this middleware meets:
//! missing or invalid credentials are a `401` with a challenge, and an
//! authenticated-but-refused caller a `403`. The CNF security suites
//! (`CNF/tests/platform/robot/SECURITY_TESTS/`) are the conformance reference.
//!
//! The crate's [`Gate`] checks every credential: a bearer token against the
//! configured issuers' key sets (RFC 9068 §4, asymmetric algorithms only) and a
//! Basic credential against the Argon2id hashes of `[[auth.basic.users]]`
//! (RFC 7617). It issues no token. Its [`Refusal`] gives each refusal its status
//! and challenge (RFC 6750 §3.1); the body is the ITS-REST error body.
//!
//! No openEHR spec governs role-based authorization — the SM places it out of
//! band, so the coarse RBAC gate is our own enterprise extension. It runs as one
//! axum middleware over the API router: a successful authentication puts a
//! [`Principal`] into the request extensions, and when an
//! [`crate::extensions::access::authz::AuthzHandle`] is wired the gate then
//! judges the matched operation's class, denying with a `403` that carries the
//! principal so the ATNA audit layer records it. The SMART on openEHR
//! requirement of the matched route ([`crate::smart::routes`]) is checked after
//! RBAC, so SMART only narrows (`master08 §Resource Scopes`).
//!
//! Between authentication and RBAC, the `assurance` gate judges a patient-data
//! request by the assurance level and the natural person its credential
//! carries, and publishes what it read for the access record.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 7): RFC 7519 leaves the claim set open; \
              decided-on claims lift into typed fields"
)]

pub(crate) mod assurance;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{FromRequestParts, MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ferrohealth_smart::guard::Gate;
use ferrohealth_smart::refusal::Refusal;
use ferrohealth_smart::resource::configuration::SmartOptions;
use ferrohealth_smart::resource::{
    Access, Caller, Honoured, IssuerSet, ResourceError, ResourceServer, Via,
};
use http::request::Parts;
use http::{HeaderValue, StatusCode, header};
use openehr_its::rest::runtime::ApiError;

use crate::config::AppConfig;
use crate::extensions::access::authz::AuthzHandle;
use crate::extensions::access::authz::roles::{RbacDecision, extract_roles};
use crate::overview::error::RestError;
use crate::smart::routes::{Need, SmartRoutes};
use assurance::{PatientDataGate, PatientDataRefusal};
use ferroehr::system_log::event::ActorAuthentication;

/// The state the [`middleware`] runs on: the authenticator, the optional
/// authorization handle (`None` is authentication-only), the optional
/// patient-data gate and the SMART on openEHR route table.
#[derive(Clone)]
pub(crate) struct AuthLayer {
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) authz: Option<Arc<AuthzHandle>>,
    /// The patient-data assurance and natural-person gate; `None` when no
    /// bearer mechanism is configured.
    pub(crate) patient_data: Option<Arc<PatientDataGate>>,
    /// The SMART on openEHR requirement of each route.
    pub(crate) smart: Arc<SmartRoutes>,
}

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Token `sub` / Basic username.
    pub subject: String,
    /// The `scope` claim of the token, or the scopes `[[auth.basic.users]]`
    /// grants a Basic user (RFC 6749 §3.3).
    pub scopes: Vec<String>,
    /// Roles granted to the caller, normalized to upper-case: for Bearer from
    /// the configured JWT claim paths (defaulting to the RFC 9068 §2.2.3.1
    /// carriers), for Basic from `[authz.rbac.basic_roles]`. An OAuth2 scope is
    /// not a role — see [`Self::scopes`].
    pub roles: Vec<String>,
    /// The retained, validated JWT claim set (Bearer only; empty for Basic), so
    /// the ABAC layer can resolve subject attributes without re-parsing the
    /// token.
    pub claims: serde_json::Map<String, serde_json::Value>,
    /// Which mechanism authenticated the caller.
    pub method: AuthMethod,
    /// What the `ferrohealth-smart` gate admitted, which the SMART on openEHR
    /// checks read.
    pub access: Access,
    /// What FerroEHR honours for the caller beyond a `user/` grant: whether it
    /// is a listed backend client and whether its issuer's `patient/` grants
    /// count (`[smart] backend_clients`, `[smart] patient_issuers`).
    pub honoured: Honoured,
}

impl Principal {
    /// Returns the principal of the caller the gate admitted with `access`,
    /// holding `roles` and honoured as `honoured`.
    ///
    /// An open surface admits no caller, so it yields an empty, unattributed
    /// principal; the middleware never builds one there.
    #[must_use]
    pub fn new(access: Access, roles: Vec<String>, honoured: Honoured) -> Self {
        let (subject, scopes, claims, method) = match access.caller() {
            Some(caller) => (
                caller.subject().to_owned(),
                caller.scope().to_vec(),
                caller.claims().clone(),
                match caller.via() {
                    Via::Basic => AuthMethod::Basic,
                    Via::Bearer => AuthMethod::Bearer,
                },
            ),
            None => (
                String::new(),
                Vec::new(),
                serde_json::Map::new(),
                AuthMethod::Bearer,
            ),
        };
        Self {
            subject,
            scopes,
            roles,
            claims,
            method,
            access,
            honoured,
        }
    }
}

/// The mechanism that authenticated a [`Principal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// HTTP Basic: a username/password pair verified against the Argon2 store.
    Basic,
    /// OAuth2/OIDC Bearer: a JWT validated against the issuer's key set.
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

/// An authentication failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    /// The request carried no credential for any admitted mechanism.
    #[error("no credentials supplied")]
    MissingCredentials,
    /// A Basic credential that does not verify: an unknown user or a wrong
    /// password, one outcome so a refusal says neither.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// The `Authorization` header is not a well-formed credential at all: two
    /// credentials, an unparsable header, or a bearer credential outside the
    /// RFC 6750 §2.1 `b64token` grammar.
    ///
    /// RFC 6750 §3.1 gives this its own `invalid_request` code and a `400`,
    /// because it is a request defect rather than a credential decision.
    #[error("malformed authorization header: {0}")]
    MalformedRequest(String),
    /// A bearer token that does not hold.
    ///
    /// RFC 9068 §4 gives every failed check the one `invalid_token` answer; the
    /// crate logs which check failed at debug level and nothing else.
    #[error("invalid bearer token")]
    InvalidToken,
    /// The issuer's signing keys could not be read, so no bearer token can be
    /// validated.
    #[error("the authorization server's signing keys could not be read")]
    KeyResolution,
    /// The caller authenticated but is not permitted the request — the 403
    /// branch of the ITS-REST 401-vs-403 split.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// The server could not run credential verification at all. A server
    /// fault, never a statement about the credentials, so it maps to 500
    /// rather than 401.
    #[error("credential verification unavailable: {0}")]
    VerificationUnavailable(String),
}

impl AuthError {
    /// The outcome of a [`Refusal`] the gate answered.
    fn from_refusal(refusal: &Refusal) -> Self {
        match refusal {
            Refusal::Missing(_) => AuthError::MissingCredentials,
            Refusal::InvalidRequest => AuthError::MalformedRequest(
                "the Authorization header carries no single well-formed credential".to_owned(),
            ),
            Refusal::InvalidToken => AuthError::InvalidToken,
            Refusal::Forbidden | Refusal::Unclassified => {
                AuthError::Forbidden("the credential does not permit this request".to_owned())
            }
            Refusal::Unavailable => AuthError::KeyResolution,
            // NOTE: `Refusal` is non-exhaustive; an outcome added later is a
            // credential that does not hold until it is read here.
            Refusal::InvalidBasic | _ => AuthError::InvalidCredentials,
        }
    }

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
            AuthError::KeyResolution => ApiError::ServiceUnavailable(self.to_string()),
            // The body says nothing about why: rendering the rejection would
            // tell an unauthenticated caller whether a token was expired or
            // forged. The reason stays in the debug log.
            AuthError::MissingCredentials
            | AuthError::InvalidCredentials
            | AuthError::InvalidToken => ApiError::Unauthorized("authentication failed".to_owned()),
        }
    }

    /// The refusal whose challenge this outcome carries, or `None` for an
    /// outcome that carries none.
    const fn refusal(&self, schemes: ferrohealth_smart::refusal::Schemes) -> Option<Refusal> {
        match self {
            AuthError::MissingCredentials => Some(Refusal::Missing(schemes)),
            AuthError::InvalidCredentials => Some(Refusal::InvalidBasic),
            AuthError::InvalidToken => Some(Refusal::InvalidToken),
            AuthError::MalformedRequest(_) => Some(Refusal::InvalidRequest),
            AuthError::Forbidden(_) => Some(Refusal::Forbidden),
            AuthError::KeyResolution | AuthError::VerificationUnavailable(_) => None,
        }
    }

    /// A stable, low-cardinality label naming this outcome.
    ///
    /// Carries no credential material, so it is safe as a structured log field
    /// or a metric label.
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            AuthError::MissingCredentials => "missing_credentials",
            AuthError::InvalidCredentials => "invalid_credentials",
            AuthError::MalformedRequest(_) => "malformed_request",
            AuthError::InvalidToken => "invalid_token",
            AuthError::KeyResolution => "key_resolution",
            AuthError::Forbidden(_) => "forbidden",
            AuthError::VerificationUnavailable(_) => "verification_unavailable",
        }
    }
}

/// The configured authenticator: the crate's gate, the role sources and the
/// verified Basic credential cache.
pub struct Authenticator {
    /// `[auth] enabled`.
    enabled: bool,
    /// The credentials admitted, checked by `ferrohealth-smart`.
    gate: Gate,
    /// The protection space every challenge names.
    realm: String,
    /// The issuer whose metadata the SMART discovery document is built from.
    primary: Option<Arc<ResourceServer>>,
    /// Dotted JWT claim paths mined for RBAC roles (`[authz.rbac] role_claims`).
    role_claims: Vec<String>,
    /// The RBAC rules a Basic user's roles are read from.
    rbac: ferroehr::config::authz::RbacConfig,
    /// Verified Basic-credential cache, keyed by the SHA-256 of the presented
    /// `Authorization` header.
    ///
    /// An entry exists only after a successful Argon2 verification, and the TTL
    /// (`[authz] basic_cache_ttl_seconds`) bounds revocation lag; `None`
    /// disables it. Argon2 costs tens of milliseconds of CPU per call by
    /// design, so without this a busy client's request rate is capped by the
    /// KDF's work factor.
    verified: Option<moka::future::Cache<[u8; 32], Principal>>,
    /// KDF verifications actually performed: a test seam and a cheap
    /// operational signal.
    kdf_verifications: AtomicU64,
}

impl std::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authenticator")
            .field("enabled", &self.enabled)
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}

impl Authenticator {
    /// Builds the authenticator of `config`: each `[auth]` issuer's metadata
    /// and key set are read here, so an authorization server that cannot be
    /// reached refuses the start rather than the first request.
    ///
    /// # Errors
    /// A message naming the refused setting or the issuer that could not be
    /// read.
    pub async fn new(config: &AppConfig) -> Result<Arc<Self>, String> {
        let resolved = ferroehr::config::auth::resolve(&config.auth)
            .map_err(|e| ferroehr::service::error::ErrorChain::new(&e).to_string())?;
        let mut gate = Gate::open();
        let mut primary = None;
        if resolved.enabled {
            if !resolved.issuers.is_empty() {
                let mut issuers = IssuerSet::new();
                for settings in resolved.issuers {
                    let issuer = settings.issuer.clone();
                    let server = ResourceServer::discover(settings)
                        .await
                        .map_err(|e| unreadable_issuer(&issuer, &e))?;
                    let server = Arc::new(server);
                    primary.get_or_insert_with(|| Arc::clone(&server));
                    issuers = issuers.with(server);
                }
                gate = gate.with_bearer(Arc::new(issuers));
            }
            if let Some(store) = resolved.basic {
                gate = gate.with_basic(Arc::new(store));
            }
            let backend = config.smart.backend_clients.clone();
            let patient = config.smart.patient_issuers.clone();
            gate = gate
                .with_realm(ferroehr::config::auth::realm(&config.auth))
                .with_honoured(move |caller| Honoured {
                    backend: caller
                        .client_id()
                        .is_some_and(|client| backend.iter().any(|listed| listed == client)),
                    patient: caller
                        .issuer()
                        .is_some_and(|issuer| patient.iter().any(|listed| listed == issuer)),
                });
        }
        let verified = (config.basic_cache_ttl_seconds > 0).then(|| {
            moka::future::Cache::builder()
                .max_capacity(1024)
                .time_to_live(std::time::Duration::from_secs(
                    config.basic_cache_ttl_seconds,
                ))
                .build()
        });
        Ok(Arc::new(Self {
            enabled: config.auth.enabled,
            gate,
            realm: ferroehr::config::auth::realm(&config.auth).to_owned(),
            primary,
            role_claims: config.role_claims.clone(),
            rbac: config.rbac.clone(),
            verified,
            kdf_verifications: AtomicU64::new(0),
        }))
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether a bearer token is admitted.
    pub(crate) fn bearer_admitted(&self) -> bool {
        self.gate.schemes().bearer
    }

    /// The SMART discovery document of the first issuer under `options`, or
    /// `None` when no issuer's metadata was read.
    pub(crate) fn smart_configuration(&self, options: &SmartOptions) -> Option<serde_json::Value> {
        self.primary
            .as_ref()
            .map(|server| server.smart_configuration(options))
    }

    /// KDF verifications performed so far.
    #[cfg(test)]
    pub(crate) fn kdf_verifications(&self) -> u64 {
        self.kdf_verifications.load(Ordering::Relaxed)
    }

    /// Builds the `WWW-Authenticate` challenge of `outcome`: the schemes the
    /// gate admits with no error code when no credential was presented
    /// (RFC 6750 §3.1), the refused scheme's otherwise.
    pub(crate) fn challenge(&self, outcome: Option<&AuthError>) -> HeaderValue {
        let refusal = outcome.map_or(Some(Refusal::Missing(self.gate.schemes())), |e| {
            e.refusal(self.gate.schemes())
        });
        let value = refusal
            .and_then(|refusal| refusal.challenge(Some(&self.realm)))
            .unwrap_or_else(|| format!(r#"Bearer realm="{}""#, self.realm));
        HeaderValue::from_str(&value)
            .unwrap_or_else(|_invalid| HeaderValue::from_static(r#"Bearer realm="ferroehr""#))
    }

    /// Authenticates the request carrying `headers`.
    ///
    /// A Basic credential verified within the cache TTL is answered from the
    /// cache, a continuing session; every other credential goes to the gate.
    pub(crate) async fn authenticate(
        &self,
        headers: &http::HeaderMap,
    ) -> Result<Authenticated, AuthError> {
        let mut basic_key = None;
        if let Some(raw) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            let scheme = raw.split_whitespace().next().unwrap_or_default();
            if scheme.eq_ignore_ascii_case("bearer") && self.gate.schemes().bearer {
                bearer_credential(raw)?;
            }
            if scheme.eq_ignore_ascii_case("basic") && self.gate.schemes().basic {
                // Keyed by the SHA-256 of the presented header, never the
                // plaintext.
                let key: [u8; 32] = {
                    use sha2::Digest as _;
                    sha2::Sha256::digest(raw.as_bytes()).into()
                };
                if let Some(cache) = &self.verified
                    && let Some(principal) = cache.get(&key).await
                {
                    return Ok(Authenticated {
                        principal,
                        fresh: false,
                    });
                }
                self.kdf_verifications.fetch_add(1, Ordering::Relaxed);
                basic_key = Some(key);
            }
        }
        let access = self
            .gate
            .authenticate(headers)
            .await
            .map_err(|refusal| AuthError::from_refusal(&refusal))?;
        let caller = access.caller().ok_or(AuthError::MissingCredentials)?;
        // An unattributable subject cannot be recorded in the ATNA trail, so a
        // blank `sub` is refused like any token that does not hold.
        if caller.subject().trim().is_empty() {
            return Err(AuthError::InvalidToken);
        }
        let roles = self.roles_of(caller);
        let honoured = self.gate.honoured(&access);
        let principal = Principal::new(access.clone(), roles, honoured);
        // The KDF ran for a Basic credential: a genuine authentication event.
        let fresh = basic_key.is_some() && caller.via() == Via::Basic;
        if let (true, Some(key), Some(cache)) = (fresh, basic_key, &self.verified) {
            cache.insert(key, principal.clone()).await;
        }
        Ok(Authenticated { principal, fresh })
    }

    /// The roles of `caller`: the configured claim paths of a token, the
    /// `[authz.rbac.basic_roles]` entry of a Basic user.
    fn roles_of(&self, caller: &Caller) -> Vec<String> {
        match caller.via() {
            Via::Basic => self.rbac.roles_of_basic_user(caller.subject()),
            Via::Bearer => extract_roles(caller.claims(), &self.role_claims),
        }
    }
}

/// The boot refusal of an issuer whose metadata or key set could not be read.
///
/// A key set that does not parse is named by its JSON failure class and
/// position, never by the parser's message, which quotes the offending value;
/// every other failure carries its whole cause chain.
fn unreadable_issuer(issuer: &str, error: &ResourceError) -> String {
    match error {
        ResourceError::KeySet { error: json, .. } => format!(
            "the authorization server {issuer} could not be read: {error}: {:?} error at \
             line {}, column {}",
            json.classify(),
            json.line(),
            json.column()
        ),
        _ => format!(
            "the authorization server {issuer} could not be read: {}",
            ferroehr::service::error::ErrorChain::new(error)
        ),
    }
}

/// Holds a bearer credential to the RFC 6750 §2.1 grammar.
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
            if let Some(mut refusal) = smart_refusal(&layer, &req, &principal) {
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

/// Checks the SMART on openEHR requirement of the matched route, after RBAC.
///
/// A requirement the caller's grants do not meet, or a `patient/` grant whose
/// target is not the EHR of the token's launch context, is a `403` with the
/// `insufficient_scope` challenge (RFC 6750 §3.1) that names no scope,
/// attributed to the caller. A composition operation is checked by the PEP once
/// its template is known.
fn smart_refusal(layer: &AuthLayer, req: &Request, principal: &Principal) -> Option<Response> {
    let matched = req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    let need = layer.smart.need(req.method(), matched, req.uri().path());
    let Need::Now(requirement) = need else {
        return None;
    };
    let reason = match crate::smart::routes::check(
        &requirement,
        principal,
        crate::smart::routes::target_ehr(&requirement, matched, req.uri(), req.headers())
            .as_deref(),
    ) {
        Ok(()) => return None,
        Err(reason) => reason,
    };
    count_auth_failure(mechanism_label(principal.method), "403");
    tracing::debug!(reason, "a SMART on openEHR requirement was not met");
    let mut resp = RestError(ApiError::Forbidden(
        "the credential does not permit this request".to_owned(),
    ))
    .into_response();
    resp.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        layer
            .authenticator
            .challenge(Some(&AuthError::Forbidden(String::new()))),
    );
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
                .access
                .caller()
                .and_then(Caller::issuer)
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
/// on a bearer `400` and `403` too, where it names what the client did wrong.
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
        tracing::warn!(
            mechanism,
            reason,
            detail = %ferroehr::service::error::ErrorChain::new(e),
            "authentication refused"
        );
    }
    let mut resp = RestError(api).into_response();
    if matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST
    ) {
        resp.headers_mut()
            .insert(header::WWW_AUTHENTICATE, auth.challenge(Some(e)));
    }
    resp
}

/// The RFC 6750 §2.1 grammar, asserted per clause.
#[cfg(test)]
mod bearer_grammar_tests {
    use super::{AuthError, bearer_credential};

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

    /// RFC 9110 §15.6.4: an unreachable issuer means the server cannot decide,
    /// which is a 503 — never a 401 telling a caller its valid token was
    /// rejected.
    #[test]
    fn unreachable_issuer_is_503_not_401() {
        let api = AuthError::KeyResolution.to_api_error();
        assert_eq!(api.status(), http::StatusCode::SERVICE_UNAVAILABLE);
    }

    /// A malformed header is a request defect (400), not a credential decision
    /// (401) — RFC 6750 §3.1 `invalid_request`.
    #[test]
    fn malformed_authorization_header_is_400_invalid_request() {
        let api = AuthError::MalformedRequest("bad".to_owned()).to_api_error();
        assert_eq!(api.status(), http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn verification_unavailable_is_500_not_401() {
        // A failed blocking verification is a SERVER fault: it must surface as
        // 500 Internal, never as 401 — a server failure misreported as a client
        // error is the silent-wrong-answer class this mapping exists to prevent.
        let err = AuthError::VerificationUnavailable("task panicked".to_owned());
        assert_eq!(
            err.to_api_error().status(),
            http::StatusCode::INTERNAL_SERVER_ERROR
        );
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
    #![expect(
        clippy::disallowed_types,
        reason = "a test builds a token's claim set directly; the claim set is RFC 7519-open"
    )]

    use std::time::Duration;

    use argon2::password_hash::PasswordHasher;
    use argon2::{Argon2, PasswordHash};
    use ferrohealth_smart::config::{AuthSection, BasicSection, BasicUser};
    use secrecy::SecretString;
    use serde_json::{Value, json};

    use super::*;
    use crate::test_support::{self, AUDIENCE, ISSUER};

    fn hash(pw: &str) -> String {
        Argon2::default()
            .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
            .unwrap()
            .to_string()
    }

    /// A Basic-only configuration admitting `alice` / `pw`.
    fn basic_config(cache_ttl: u64) -> AppConfig {
        AppConfig {
            auth: AuthSection {
                basic: Some(BasicSection {
                    users: vec![BasicUser {
                        user: "alice".to_owned(),
                        password_hash: Some(SecretString::from(hash("pw"))),
                        password_hash_file: None,
                        scopes: vec!["user/composition-*.cruds".to_owned()],
                    }],
                }),
                ..AuthSection::default()
            },
            basic_cache_ttl_seconds: cache_ttl,
            ..AppConfig::default()
        }
    }

    async fn basic_only() -> Arc<Authenticator> {
        Authenticator::new(&basic_config(60))
            .await
            .expect("a Basic-only authenticator")
    }

    async fn bearer_only() -> Arc<Authenticator> {
        Authenticator::new(&AppConfig {
            auth: test_support::bearer_auth(),
            ..AppConfig::default()
        })
        .await
        .expect("a bearer authenticator")
    }

    fn headers(auth: &str) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(auth).unwrap());
        h
    }

    fn bearer(claims: &Value) -> http::HeaderMap {
        headers(&format!("Bearer {}", test_support::token(claims)))
    }

    fn now() -> i64 {
        jiff::Timestamp::now().as_second()
    }

    #[tokio::test]
    async fn verified_credential_cache_skips_the_kdf_on_a_hit() {
        let auth = basic_only().await;
        let h = headers("Basic YWxpY2U6cHc="); // alice:pw
        let first = auth.authenticate(&h).await.expect("first verifies");
        let second = auth.authenticate(&h).await.expect("second hits the cache");
        assert!(first.fresh, "the KDF ran → a genuine authentication event");
        assert!(
            !second.fresh,
            "a cache hit is a continuing session, not a new authentication"
        );
        assert_eq!(
            auth.kdf_verifications(),
            1,
            "one KDF verification for two requests with the same credential"
        );

        // A different credential is a different key — it must NOT hit the
        // cached entry, and the wrong password still fails.
        let bad = headers("Basic YWxpY2U6d3Jvbmc="); // alice:wrong
        auth.authenticate(&bad).await.expect_err("wrong password");
        assert_eq!(
            auth.kdf_verifications(),
            2,
            "the wrong credential paid its own (failed) verification"
        );
    }

    #[tokio::test]
    async fn verified_cache_ttl_zero_disables_caching() {
        let auth = Authenticator::new(&basic_config(0)).await.unwrap();
        let h = headers("Basic YWxpY2U6cHc=");
        auth.authenticate(&h).await.expect("ok");
        auth.authenticate(&h).await.expect("ok");
        assert_eq!(auth.kdf_verifications(), 2, "TTL 0 verifies every request");
    }

    #[tokio::test]
    async fn missing_credentials() {
        let err = basic_only()
            .await
            .authenticate(&http::HeaderMap::new())
            .await
            .expect_err("reject");
        assert!(matches!(err, AuthError::MissingCredentials));
        assert_eq!(err.to_api_error().status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_basic() {
        let p = basic_only()
            .await
            .authenticate(&headers("Basic YWxpY2U6cHc="))
            .await
            .expect("ok");
        assert_eq!(p.principal.subject, "alice");
        assert_eq!(p.principal.method, AuthMethod::Basic);
        assert_eq!(
            p.principal.scopes,
            vec!["user/composition-*.cruds".to_owned()]
        );
        assert_eq!(
            p.principal.roles,
            vec!["USER".to_owned()],
            "a Basic user with no [authz.rbac.basic_roles] entry holds the user role"
        );
        assert!(
            p.fresh,
            "a first Basic verification is a genuine authentication"
        );
    }

    #[tokio::test]
    async fn configured_basic_roles_are_upper_cased() {
        let mut config = basic_config(60);
        config.rbac.basic_roles.insert(
            "alice".to_owned(),
            vec!["admin".to_owned(), "user".to_owned()],
        );
        let auth = Authenticator::new(&config).await.unwrap();
        let p = auth
            .authenticate(&headers("Basic YWxpY2U6cHc="))
            .await
            .expect("ok");
        assert_eq!(
            p.principal.roles,
            vec!["ADMIN".to_owned(), "USER".to_owned()]
        );
    }

    /// RFC 6750 §3.1: a request that "attempted using an unsupported
    /// authentication method" lacks authentication information, so a bearer
    /// token on a Basic-only surface is a missing credential, challenged with no
    /// error code.
    #[tokio::test]
    async fn bearer_without_an_issuer_is_a_missing_credential() {
        let auth = basic_only().await;
        let err = auth
            .authenticate(&headers("Bearer sometoken"))
            .await
            .expect_err("reject");
        assert!(matches!(err, AuthError::MissingCredentials), "got {err:?}");
        let challenge = auth.challenge(Some(&err));
        assert!(
            !challenge.to_str().unwrap().contains("error="),
            "{challenge:?}"
        );
    }

    #[tokio::test]
    async fn wrong_password_is_401_not_500() {
        let auth = basic_only().await;
        for credential in [
            "Basic YWxpY2U6d3Jvbmc=", // alice:wrong
            "Basic Ym9iOnB3",         // bob:pw, an unknown user
        ] {
            let err = auth
                .authenticate(&headers(credential))
                .await
                .expect_err("reject");
            assert!(matches!(err, AuthError::InvalidCredentials), "got {err:?}");
            assert_eq!(err.to_api_error().status(), StatusCode::UNAUTHORIZED);
        }
    }

    /// RFC 7617 §2: the credential is the base64 of `user-id:password`, so an
    /// unpadded, over-padded, interior-padded or non-alphabet value does not
    /// decode and is refused like a wrong password.
    #[tokio::test]
    async fn a_malformed_basic_credential_is_refused() {
        let auth = basic_only().await;
        for malformed in [
            "Basic YWxpY2U6cHc",    // unpadded
            "Basic YWxpY2U6cHc===", // excess padding
            "Basic YWxp=Y2U6cHc=",  // interior padding
            "Basic YWxpY2U6cHc*",   // outside the alphabet
        ] {
            let err = auth
                .authenticate(&headers(malformed))
                .await
                .expect_err("a malformed credential must be refused");
            assert!(
                matches!(err, AuthError::InvalidCredentials),
                "{malformed}: {err:?}"
            );
        }
    }

    #[test]
    fn phc_hash_is_parseable() {
        // Guards the fixed-salt test helper against argon2 format drift.
        assert!(PasswordHash::new(&hash("pw")).is_ok());
    }

    #[tokio::test]
    async fn valid_token_authenticates() {
        let p = bearer_only()
            .await
            .authenticate(&bearer(&test_support::claims(
                "alice",
                "openid user/composition-*.r",
            )))
            .await
            .expect("ok");
        assert_eq!(p.principal.subject, "alice");
        assert_eq!(p.principal.method, AuthMethod::Bearer);
        assert!(p.principal.scopes.contains(&"openid".to_owned()));
        // The validated claim set is retained on the Principal.
        assert_eq!(
            p.principal.claims.get("sub").and_then(Value::as_str),
            Some("alice")
        );
        assert!(
            !p.fresh,
            "a bearer request is federated, never a login event"
        );
    }

    /// One invalid token of every kind the old validator classified: each is
    /// refused, and none differs from another on the wire.
    fn invalid_tokens() -> Vec<(&'static str, String)> {
        let base = test_support::claims("alice", "user/composition-*.r");
        let without = |claim: &str| {
            let mut c = base.clone();
            c.as_object_mut().expect("object").remove(claim);
            test_support::token(&c)
        };
        let with = |claim: &str, value: Value| {
            let mut c = base.clone();
            c[claim] = value;
            test_support::token(&c)
        };
        let mut tampered = test_support::token(&base);
        let flip = tampered.len() - 10;
        let replacement = if tampered.as_bytes()[flip] == b'A' {
            "B"
        } else {
            "A"
        };
        tampered.replace_range(flip..=flip, replacement);
        let unsigned = |typ: Option<&str>, claims: &Value| {
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
            header.typ = typ.map(str::to_owned);
            // RFC 8725 §2.1: a token "signed" with the issuer's PUBLIC key as an
            // HMAC secret, the algorithm-confusion forgery.
            jsonwebtoken::encode(
                &header,
                claims,
                &jsonwebtoken::EncodingKey::from_secret(include_bytes!(
                    "../../../../../../docs/conformance/party/smart/jwks.json"
                )),
            )
            .expect("encode")
        };
        let typed = |typ: &str| {
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            header.typ = Some(typ.to_owned());
            header.kid = Some("cnf-smart-test".to_owned());
            jsonwebtoken::encode(
                &header,
                &base,
                &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                    "../../../../../../docs/conformance/party/smart/cnf-smart-test.key.pem"
                ))
                .expect("key"),
            )
            .expect("encode")
        };
        vec![
            ("not a JWT", "not-a-jwt".to_owned()),
            ("no sub", without("sub")),
            ("blank sub", with("sub", json!("   "))),
            ("expired", with("exp", json!(now() - 3600))),
            ("not yet valid", with("nbf", json!(now() + 3600))),
            ("wrong issuer", with("iss", json!("https://evil.example"))),
            ("no issuer", without("iss")),
            (
                "another audience",
                with("aud", json!("some-other-resource-server")),
            ),
            ("tampered signature", tampered),
            ("HMAC confusion", unsigned(Some("at+jwt"), &base)),
            ("an ID token", typed("JWT+ID")),
            ("untyped under RFC 9068", typed("JWT")),
            ("no jti", without("jti")),
            ("no client_id", without("client_id")),
            ("no iat", without("iat")),
            ("issued in the future", with("iat", json!(now() + 3600))),
        ]
    }

    /// RFC 9068 §4 rejects a token "in case of any failure in the validation
    /// checks" with one `invalid_token` answer (RFC 6750 §3.1): neither the
    /// status nor the challenge a client sees may vary with the reason.
    #[tokio::test]
    async fn every_invalid_token_renders_one_status_and_one_challenge() {
        let auth = bearer_only().await;
        let mut challenges: Vec<HeaderValue> = Vec::new();
        for (case, token) in invalid_tokens() {
            let err = auth
                .authenticate(&headers(&format!("Bearer {token}")))
                .await
                .expect_err(case);
            assert!(matches!(err, AuthError::InvalidToken), "{case}: {err:?}");
            assert_eq!(
                err.to_api_error().status(),
                StatusCode::UNAUTHORIZED,
                "{case}"
            );
            challenges.push(auth.challenge(Some(&err)));
        }
        let first = challenges.first().expect("cases exist").clone();
        assert!(
            challenges.iter().all(|c| *c == first),
            "the challenge must not vary with the refusal reason: {challenges:?}"
        );
        assert_eq!(
            first,
            HeaderValue::from_static(r#"Bearer realm="ferroehr", error="invalid_token""#)
        );
    }

    /// RFC 9068 §4: under the `jwt` profile a plainly typed or untyped token is
    /// accepted, and a token typed as another kind of JWT is still refused.
    #[tokio::test]
    async fn the_jwt_profile_admits_a_plain_jwt_and_refuses_another_kind() {
        let mut auth = test_support::bearer_auth();
        auth.token_profile = ferrohealth_smart::resource::TokenProfile::Jwt;
        let authenticator = Authenticator::new(&AppConfig {
            auth,
            ..AppConfig::default()
        })
        .await
        .expect("authenticator");
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
            "../../../../../../docs/conformance/party/smart/cnf-smart-test.key.pem"
        ))
        .expect("key");
        let mint = |typ: Option<&str>| {
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            header.typ = typ.map(str::to_owned);
            header.kid = Some("cnf-smart-test".to_owned());
            let mut claims = json!({
                "iss": ISSUER, "aud": AUDIENCE, "sub": "alice", "exp": now() + 3600,
            });
            claims["scope"] = json!("user/composition-*.r");
            jsonwebtoken::encode(&header, &claims, &key).expect("encode")
        };
        for typ in [Some("JWT"), None] {
            authenticator
                .authenticate(&headers(&format!("Bearer {}", mint(typ))))
                .await
                .expect("a plain JWT authenticates under the jwt profile");
        }
        let err = authenticator
            .authenticate(&headers(&format!("Bearer {}", mint(Some("JWT+ID")))))
            .await
            .expect_err("an ID token is refused under either profile");
        assert!(matches!(err, AuthError::InvalidToken), "got {err:?}");
    }

    #[tokio::test]
    async fn keycloak_realm_access_roles_extracted() {
        // Roles live under `realm_access.roles` and surface upper-cased. The
        // `scope` claim is NOT a role source: an OAuth2 scope grants a client
        // delegated authority (RFC 6749 §3.3) and asserts nothing about the
        // subject's roles.
        let mut c = test_support::claims("alice", "openid EHR_READ");
        c["realm_access"] = json!({ "roles": ["user", "ferroehr-admin"] });
        let p = bearer_only()
            .await
            .authenticate(&bearer(&c))
            .await
            .expect("ok")
            .principal;
        assert!(p.roles.contains(&"USER".to_owned()));
        assert!(p.roles.contains(&"FERROEHR-ADMIN".to_owned()));
        assert!(
            !p.roles.contains(&"OPENID".to_owned()),
            "`openid` is a scope, not a role",
        );
        assert!(
            !p.roles.contains(&"EHR_READ".to_owned()),
            "a scope must not become a role",
        );
        assert!(p.scopes.contains(&"openid".to_owned()));
    }

    #[test]
    fn extract_roles_upper_cases_and_dedups() {
        let claims = json!({
            "realm_access": { "roles": ["admin", "Admin", "user"] },
            "scope": "user OFFLINE_ACCESS",
        })
        .as_object()
        .cloned()
        .unwrap();
        let roles = extract_roles(
            &claims,
            &ferroehr::config::authz::RbacConfig::default().role_claims,
        );
        assert_eq!(roles, vec!["ADMIN".to_owned(), "USER".to_owned()]);
    }

    #[test]
    fn extract_roles_missing_paths_yields_empty() {
        let claims = json!({ "sub": "x" }).as_object().cloned().unwrap();
        assert!(
            extract_roles(
                &claims,
                &ferroehr::config::authz::RbacConfig::default().role_claims
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_roles_custom_nested_path() {
        let claims = json!({ "resource_access": { "client": { "roles": ["writer"] } } })
            .as_object()
            .cloned()
            .unwrap();
        let roles = extract_roles(&claims, &["resource_access.client.roles".to_owned()]);
        assert_eq!(roles, vec!["WRITER".to_owned()]);
    }

    /// Each outcome gets its own stable label, so a burst of one kind is
    /// countable in the logs rather than greppable out of a message.
    #[test]
    fn outcome_labels_are_distinct() {
        let mut labels: Vec<&'static str> = [
            AuthError::MissingCredentials,
            AuthError::InvalidCredentials,
            AuthError::MalformedRequest(String::new()),
            AuthError::InvalidToken,
            AuthError::KeyResolution,
            AuthError::Forbidden(String::new()),
            AuthError::VerificationUnavailable(String::new()),
        ]
        .iter()
        .map(AuthError::label)
        .collect();
        let total = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), total, "two outcomes share a label");
    }

    /// RFC 6750 §3.1 assigns each outcome its own code, and assigns NONE when
    /// no credential was presented.
    #[tokio::test]
    async fn challenge_error_codes_follow_rfc6750() {
        let auth = bearer_only().await;
        let code = |outcome: &AuthError| {
            auth.challenge(Some(outcome))
                .to_str()
                .unwrap()
                .split("error=")
                .nth(1)
                .map(|rest| rest.trim_matches('"').to_owned())
        };
        assert_eq!(code(&AuthError::MissingCredentials), None);
        assert_eq!(
            code(&AuthError::InvalidToken).as_deref(),
            Some("invalid_token")
        );
        assert_eq!(
            code(&AuthError::Forbidden("x".to_owned())).as_deref(),
            Some("insufficient_scope")
        );
        assert_eq!(
            code(&AuthError::MalformedRequest("x".to_owned())).as_deref(),
            Some("invalid_request")
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

    /// The refusal reaches the log as its own field — the crate's debug record
    /// names which check failed, FerroEHR's record the outcome — and neither
    /// the token nor a claim value appears anywhere in the record.
    #[tokio::test]
    async fn a_refused_token_logs_its_reason_and_never_the_token() {
        use tracing_subscriber::layer::SubscriberExt as _;

        const TOKEN: &str = "not-a-jwt-at-all";

        let capture = EventCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let config = AppConfig {
            auth: test_support::bearer_auth(),
            ..AppConfig::default()
        };
        let layer = AuthLayer {
            authenticator: Authenticator::new(&config).await.expect("authenticator"),
            authz: None,
            patient_data: None,
            smart: Arc::new(SmartRoutes::new("", "__ferroehr.platform", Vec::new())),
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
            Some("invalid_token"),
            "{refusal:?}"
        );
        assert_eq!(
            refusal.get("mechanism").map(String::as_str),
            Some("bearer"),
            "{refusal:?}"
        );
        let check = events
            .iter()
            .find(|fields| {
                fields
                    .get("message")
                    .is_some_and(|m| m == "an access token was refused")
            })
            .expect("the crate logs which check failed");
        assert_eq!(
            check.get("reason").map(String::as_str),
            Some("malformed"),
            "{check:?}"
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

    /// The issuer's key set, read over its `jwks_uri`: the boot read, the
    /// refetch on an unknown key, and the failures of each. No openEHR spec
    /// governs auth transport hardening — the crate's design, held here at
    /// FerroEHR's seam.
    mod remote_issuer {
        use std::sync::atomic::AtomicUsize;
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
                "../../../../../../docs/conformance/party/smart/jwks.json"
            ))
            .expect("the committed key set");
            set["keys"][0]["kid"] = json!(kid);
            set
        }

        /// An `[auth]` section trusting the issuer `server` runs.
        fn section(issuer: &str, refresh_seconds: u64, timeout_ms: u64) -> AuthSection {
            AuthSection {
                issuer: ferrohealth_smart::config::IssuerKey::Flat(issuer.to_owned()),
                audience: vec![AUDIENCE.to_owned()],
                jwks_refresh_interval_seconds: refresh_seconds,
                timeout_ms,
                ..AuthSection::default()
            }
        }

        async fn authenticator(section: AuthSection) -> Result<Arc<Authenticator>, String> {
            Authenticator::new(&AppConfig {
                auth: section,
                ..AppConfig::default()
            })
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
        fn token_under(issuer: &str, kid: &str) -> http::HeaderMap {
            let mut claims = test_support::claims("alice", "user/composition-*.r");
            claims["iss"] = json!(issuer);
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            header.typ = Some("at+jwt".to_owned());
            header.kid = Some(kid.to_owned());
            let token = jsonwebtoken::encode(
                &header,
                &claims,
                &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                    "../../../../../../docs/conformance/party/smart/cnf-smart-test.key.pem"
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
                    ResponseTemplate::new(200)
                        .set_body_json(json!({ "keys": "s3cr3t-key-material" })),
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
    }
}
