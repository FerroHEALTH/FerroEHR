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
//! No openEHR spec governs role-based authorization, so the coarse RBAC gate is our own enterprise extension. It runs as one
//! axum middleware over the API router: a successful authentication puts a
//! [`Principal`] into the request extensions, and when an
//! [`crate::extensions::access::authz::AuthzHandle`] is wired the gate then
//! judges the matched operation's class, denying with a `403` that carries the
//! principal so the ATNA audit layer records it. The SMART on openEHR
//! requirement of the matched route (`smart::routes`) is checked after
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
mod build;
pub(crate) mod declared;
mod gates;
mod issuer;
mod party;
pub(crate) mod purpose;
pub(crate) mod record;
pub(crate) mod refusal;
mod scope;
pub(crate) mod trc;
mod wabvpz;

use std::sync::Arc;

use axum::extract::{FromRequestParts, MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ferrohealth_smart::guard::Gate;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::refusal::Refusal;
use ferrohealth_smart::resource::configuration::SmartOptions;
use ferrohealth_smart::resource::introspection::Introspector;
use ferrohealth_smart::resource::{Access, Honoured, ResourceServer, Via};
use http::request::Parts;
use http::{HeaderValue, StatusCode};
use openehr_its::rest::runtime::ApiError;

use crate::api::served::ServedMethods;
use crate::extensions::access::authz::AuthzHandle;
use crate::overview::error::RestError;
use crate::smart::routes::SmartRoutes;
use assurance::PatientDataGate;
use ferroehr::system_log::event::ActorAuthentication;
use gates::patient_data_refusal;

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
    /// The methods each route serves, so an unserved one is answered `405`
    /// before any check judges the caller.
    pub(crate) served: Arc<ServedMethods>,
    /// The service a representative's subject is resolved to its EHR through.
    pub(crate) service: Arc<ferroehr::service::FerroEhrService>,
}

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Token `sub` / Basic username.
    pub subject: String,
    /// The `scope` claim of the token, or the scopes `[[auth.basic.users]]`
    /// grants a Basic user (RFC 6749 §3.3).
    pub scopes: Vec<String>,
    /// Roles granted to the caller, normalized to upper-case: the roles
    /// `ferrohealth-smart` read from the token's `[auth] role_claims` or a
    /// Basic user's `[[auth.basic.users]] roles`. An OAuth2 scope is not a
    /// role — see [`Self::scopes`].
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
    /// count (`[auth.smart] backend_clients`, `[auth.smart] patient_issuers`).
    pub honoured: Honoured,
    /// The EHR a bound representative is confined to, `None` for every other
    /// caller (#5086).
    pub represented: Option<RepresentedEhr>,
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
            represented: None,
        }
    }
}

/// The EHR a bound representative is confined to: the one whose
/// `EHR_STATUS.subject.external_ref` names the subject the token asserts and the
/// party source records a live representation of.
///
/// No openEHR spec governs resolving a login to a party: our own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentedEhr {
    /// That EHR's id, or `None` when no EHR names the subject, so the
    /// representative reaches no EHR.
    pub ehr_id: Option<String>,
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
/// was an authentication event.
///
/// `fresh` is `true` for a Basic credential the gate verified by the key
/// derivation on this request, and `false` for one its verified-credential
/// cache admitted (`Caller::cached`), which was authenticated on an earlier
/// request. A Bearer request is federated, so it is `false`. It decides
/// whether to emit an IHE ATNA login record, which marks authentication
/// events.
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
    /// A `DPoP` token that does not hold (RFC 9449 §7.1).
    #[error("invalid DPoP token")]
    InvalidDpopToken,
    /// A `DPoP` proof that does not hold (RFC 9449 §4.3).
    #[error("invalid DPoP proof")]
    InvalidDpopProof,
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
    /// The login binding refused the caller, under the binding `code`.
    #[error("{code}: {}", party::detail(code, named.as_ref()))]
    PartyRefused {
        /// The binding code, such as `refused-ambiguous`.
        code: &'static str,
        /// The caller whose login was refused, so the failed-authentication
        /// record names who was refused.
        principal: Option<Box<Principal>>,
        /// The party the refusal names: the one refused for its state or its
        /// marker.
        party: Option<Box<ferroehr::system_log::binding::PartyTriple>>,
        /// What the problem text names beside the code: the key claim the
        /// token lacked, or the identifier issuer of an unbound login.
        named: Option<party::Named>,
    },
    /// A caller of a kind the request does not admit, under the binding
    /// `code`: a system caller holding a personal scope, or an external caller
    /// outside its agreement.
    #[error("forbidden: {message}")]
    KindRefused {
        /// The binding code, such as `refused-no-agreement`.
        code: &'static str,
        /// What the refusal tells the caller.
        message: String,
        /// The caller who was refused, so the failed-authentication record
        /// names them.
        principal: Option<Box<Principal>>,
    },
    /// The party source or the identity provider could not answer, so the
    /// server cannot decide (RFC 9110 §15.6.4).
    #[error("the login binding could not be read")]
    PartyUnavailable {
        /// The `Retry-After` the identity provider gave, when it gave one.
        retry_after: Option<std::time::Duration>,
    },
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
            Refusal::InvalidDpopToken => AuthError::InvalidDpopToken,
            Refusal::InvalidDpopProof => AuthError::InvalidDpopProof,
            Refusal::Forbidden | Refusal::Unclassified => {
                AuthError::Forbidden("the credential does not permit this request".to_owned())
            }
            Refusal::Unavailable => AuthError::KeyResolution,
            // NOTE: `Refusal` is non-exhaustive; an outcome added later is a
            // credential that does not hold until it is read here.
            Refusal::InvalidBasic | _ => AuthError::InvalidCredentials,
        }
    }

    /// The HTTP status this outcome answers with.
    pub(crate) fn status(&self) -> StatusCode {
        self.to_api_error().status()
    }

    fn to_api_error(&self) -> ApiError {
        match self {
            AuthError::Forbidden(m) | AuthError::KindRefused { message: m, .. } => {
                ApiError::Forbidden(m.clone())
            }
            AuthError::VerificationUnavailable(m) => {
                crate::overview::error::internal_fault("verify the supplied credentials", m)
            }
            AuthError::MalformedRequest(m) => ApiError::BadRequest(m.clone()),
            // With the issuer's keys unreachable no token can be validated, so
            // the server cannot decide: RFC 9110 §15.6.4 makes that a 503. A 401
            // would tell a caller with a valid token its credential was
            // rejected.
            AuthError::KeyResolution | AuthError::PartyUnavailable { .. } => {
                ApiError::ServiceUnavailable(self.to_string())
            }
            AuthError::PartyRefused { .. } => ApiError::Forbidden(self.to_string()),
            // The body says nothing about why: rendering the rejection would
            // tell an unauthenticated caller whether a token was expired or
            // forged. The reason stays in the debug log.
            AuthError::MissingCredentials
            | AuthError::InvalidCredentials
            | AuthError::InvalidToken
            | AuthError::InvalidDpopToken
            | AuthError::InvalidDpopProof => {
                ApiError::Unauthorized("authentication failed".to_owned())
            }
        }
    }

    /// The refusal whose challenge this outcome carries, or `None` for an
    /// outcome that carries none.
    const fn refusal(&self, schemes: ferrohealth_smart::refusal::Schemes) -> Option<Refusal> {
        match self {
            AuthError::MissingCredentials => Some(Refusal::Missing(schemes)),
            AuthError::InvalidCredentials => Some(Refusal::InvalidBasic),
            AuthError::InvalidToken => Some(Refusal::InvalidToken),
            AuthError::InvalidDpopToken => Some(Refusal::InvalidDpopToken),
            AuthError::InvalidDpopProof => Some(Refusal::InvalidDpopProof),
            AuthError::MalformedRequest(_) => Some(Refusal::InvalidRequest),
            AuthError::Forbidden(_) | AuthError::KindRefused { .. } => Some(Refusal::Forbidden),
            AuthError::KeyResolution
            | AuthError::VerificationUnavailable(_)
            | AuthError::PartyRefused { .. }
            | AuthError::PartyUnavailable { .. } => None,
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
            AuthError::InvalidDpopToken => "invalid_dpop_token",
            AuthError::InvalidDpopProof => "invalid_dpop_proof",
            AuthError::KeyResolution => "key_resolution",
            AuthError::Forbidden(_) => "forbidden",
            AuthError::VerificationUnavailable(_) => "verification_unavailable",
            AuthError::PartyRefused { code, .. } | AuthError::KindRefused { code, .. } => code,
            AuthError::PartyUnavailable { .. } => "party_unavailable",
        }
    }
}

/// The check of the described issuer, whose metadata the SMART discovery
/// document is built from.
enum Primary {
    /// A JWT access token checked against the issuer's keys.
    Jwt(Arc<ResourceServer>),
    /// An opaque access token checked by the issuer's introspection endpoint
    /// (RFC 7662).
    Introspection(Arc<Introspector>),
}

/// The configured authenticator: the crate's gate and the role sources.
pub struct Authenticator {
    /// `[auth] enabled`.
    enabled: bool,
    /// The credentials admitted, checked by `ferrohealth-smart`.
    gate: Gate,
    /// The protection space every challenge names.
    realm: String,
    /// The issuer whose metadata the SMART discovery document is built from.
    primary: Option<Primary>,
    /// The issuers whose tokens may grant a privileged role.
    role_issuers: build::RoleIssuers,
    /// The party binding every admitted caller is looked up in, when an
    /// `[[auth.party]]` table binds an issuer.
    party: Option<Arc<party::Binding>>,
    /// The system clients and the federation agreements.
    declared: declared::Declared,
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
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether a bearer token is admitted.
    pub(crate) fn bearer_admitted(&self) -> bool {
        self.gate.schemes().bearer
    }

    /// The SMART discovery document of the described issuer under `options`, or
    /// `None` when no issuer's metadata was read.
    pub(crate) fn smart_configuration(&self, options: &SmartOptions) -> Option<serde_json::Value> {
        self.primary.as_ref().map(|primary| match primary {
            Primary::Jwt(server) => server.smart_configuration(options),
            Primary::Introspection(introspector) => introspector.smart_configuration(options),
        })
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

    /// Authenticates the request of `parts` through the gate, whose `DPoP`
    /// proof check reads the request line.
    ///
    /// The gate holds a bearer token to the RFC 6750 §2.1 `b64token` syntax and
    /// refuses a blank `sub` (RFC 9068 §2.2), and its Basic store keeps a
    /// verified credential for `[auth.basic] cache_ttl`; it logs which
    /// check refused a credential.
    pub(crate) async fn authenticate(&self, parts: &Parts) -> Result<Authenticated, AuthError> {
        let access = self
            .gate
            .authenticate_request(parts)
            .await
            .map_err(|denied| AuthError::from_refusal(denied.refusal()))?;
        self.admitted(access.clone())
            .await
            .map_err(|refusal| self.naming_caller(refusal, &access))
    }

    /// Binds and admits the caller of the verified `access`.
    async fn admitted(&self, access: Access) -> Result<Authenticated, AuthError> {
        let access = match &self.party {
            Some(binding) => party::bind(binding, access, &self.role_issuers).await?,
            None => access,
        };
        // NOTE: no openEHR spec governs resolving a login to a party, our own design: with
        // no binding no representation is recorded, so a representative is refused.
        if self.party.is_none()
            && access
                .caller()
                .is_some_and(|caller| matches!(caller.kind(), CallerKind::Representative { .. }))
        {
            return Err(party::unrecorded());
        }
        if let Some(refusal) = access.caller().and_then(party::kind_refusal) {
            return Err(refusal);
        }
        let access = self.declared.admit(access)?;
        let caller = access.caller().ok_or(AuthError::MissingCredentials)?;
        let roles = self.role_issuers.roles(caller);
        let honoured = self.gate.honoured(&access);
        // NOTE: no specification governs this: our own design; a credential the
        // verified-credential cache admitted was authenticated on an earlier request.
        let fresh = caller.via() == Via::Basic && !caller.cached();
        let principal = Principal::new(access.clone(), roles, honoured);
        Ok(Authenticated { principal, fresh })
    }

    /// Returns `refusal` naming the caller of `access` when the login binding
    /// or the caller's kind refused it, so the failed-authentication record
    /// carries who was refused.
    fn naming_caller(&self, refusal: AuthError, access: &Access) -> AuthError {
        let principal = || {
            access.caller().map(|caller| {
                let roles = self.role_issuers.roles(caller);
                let honoured = self.gate.honoured(access);
                Box::new(Principal::new(access.clone(), roles, honoured))
            })
        };
        match refusal {
            AuthError::PartyRefused {
                code, party, named, ..
            } => AuthError::PartyRefused {
                code,
                principal: principal(),
                party,
                named,
            },
            AuthError::KindRefused { code, message, .. } => AuthError::KindRefused {
                code,
                message,
                principal: principal(),
            },
            other => other,
        }
    }
}

/// Returns `roles` upper-cased, each once, in the order first met.
///
/// The RBAC gate matches roles without regard to case, and a policy reads the
/// role set it is handed, so every role reaches both in one spelling (no
/// specification governs role semantics: our own design).
fn upper_cased(roles: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(roles.len());
    for role in roles {
        let role = role.to_ascii_uppercase();
        if !out.contains(&role) {
            out.push(role);
        }
    }
    out
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

/// The authentication and RBAC middleware, attached to the API router; the
/// public endpoints are mounted outside it.
pub(crate) async fn middleware(
    State(layer): State<AuthLayer>,
    req: Request,
    next: Next,
) -> Response {
    let auth = &layer.authenticator;
    if !auth.enabled() {
        return REQUEST_PRINCIPAL.scope(None, next.run(req)).await;
    }
    let (parts, body) = req.into_parts();
    let outcome = auth.authenticate(&parts).await;
    let mut req = Request::from_parts(parts, body);
    match outcome {
        Ok(Authenticated {
            mut principal,
            fresh,
        }) => {
            if let Some(refusal) = purpose::mismatch(&layer, &principal, &req) {
                return refusal;
            }
            if let Some(refusal) = trc::patient_context(&layer, &mut principal, &req).await {
                return refusal;
            }
            // NOTE: RFC 9110 §15.5.6; whether a method is served is the
            // resource's property, so no grant changes the `405`.
            if let Some(mut refusal) = layer.served.refusal(
                req.method(),
                req.extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str),
            ) {
                refusal.extensions_mut().insert(principal);
                return refusal;
            }
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
            if let Some(mut refusal) = gates::rbac_refusal(&layer, &req, &principal) {
                refusal.extensions_mut().insert(actor);
                return refusal;
            }
            if let Some(mut refusal) = gates::smart_refusal(&layer, &req, &principal) {
                refusal.extensions_mut().insert(actor);
                return refusal;
            }
            req.extensions_mut().insert(principal.clone());
            let for_audit = principal.clone();
            let mut committer =
                party::committer_identity(&for_audit, &auth.declared.system_clients);
            if let Some(binding) = auth.party.as_deref() {
                match binding.details(&for_audit).await {
                    Ok(details) => committer.bound = details,
                    Err(e) => return refusal::response(auth, req.headers(), &e),
                }
            }
            // The roles ride beside the committer identity for the request
            // scope, so a record the service layer emits by itself carries
            // them too (#3239).
            let roles = for_audit.roles.clone();
            let binding = record::of_request(&for_audit, &committer);
            let mut resp = REQUEST_PRINCIPAL
                .scope(
                    Some(principal),
                    ferroehr::system_log::access_context::with_roles(
                        roles,
                        record::recorded(
                            binding,
                            ferroehr::versioning::committer::with_committer(
                                Some(committer),
                                next.run(req),
                            ),
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
        Err(e) => refusal::refusal_of(auth, req.headers(), &e),
    }
}

/// The client a request came through, which the `[[authz.origin]]` register maps to.
///
/// The Basic user, or the client the token's issuer profile reads from
/// `client_claims`, never another client claim the token carries (#5194).
#[must_use]
pub fn client_of(principal: &Principal) -> Option<String> {
    match principal.method {
        AuthMethod::Basic => Some(principal.subject.clone()),
        AuthMethod::Bearer => principal.access.caller()?.client_id().map(str::to_owned),
    }
}

/// The status each refusal outcome answers with.
#[cfg(test)]
mod outcome_status_tests {
    use super::AuthError;

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
                    RestError::from(ApiError::Unauthorized("authentication required".to_owned()))
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

    /// A replay store of this process alone, which these tests share no proof through.
    fn memory() -> Arc<dyn ferrohealth_smart::sender::replay::ReplayStore> {
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new())
    }

    use crate::config::AppConfig;
    use argon2::password_hash::PasswordHasher;
    use argon2::{Argon2, PasswordHash};
    use ferrohealth_smart::config::{AuthSection, BasicSection, BasicUser};
    use http::header;
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

    /// A Basic-only configuration admitting `alice` / `pw`, its verified
    /// credentials kept for `cache_ttl` seconds.
    fn basic_config(cache_ttl: u64) -> AppConfig {
        AppConfig {
            auth: ferroehr::config::auth_section::AuthConfig {
                shared: Box::new(AuthSection {
                    basic: Some(BasicSection {
                        users: vec![BasicUser {
                            user: "alice".to_owned(),
                            password_hash: Some(SecretString::from(hash("pw"))),
                            password_hash_file: None,
                            scopes: vec!["user/composition-*.cruds".to_owned()],
                            roles: Vec::new(),
                        }],
                        cache_ttl: ferrohealth_config::units::Duration::from_secs(cache_ttl),
                    }),
                    ..AuthSection::default()
                }),
                ..ferroehr::config::auth_section::AuthConfig::default()
            },
            ..AppConfig::default()
        }
    }

    /// An issuer at a `wiremock` server whose RFC 8414 metadata names its
    /// introspection endpoint, and an authenticator introspecting its tokens
    /// under the client authentication of RFC 7662 §2.1.
    async fn introspecting() -> (wiremock::MockServer, Arc<Authenticator>) {
        use wiremock::matchers::{method, path};
        let server = wiremock::MockServer::start().await;
        let uri = server.uri();
        wiremock::Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "issuer": uri,
                "jwks_uri": format!("{uri}/jwks"),
                "authorization_endpoint": format!("{uri}/authorize"),
                "token_endpoint": format!("{uri}/token"),
                "introspection_endpoint": format!("{uri}/introspect"),
                "grant_types_supported": ["client_credentials"],
                "code_challenge_methods_supported": ["S256"]
            })))
            .mount(&server)
            .await;
        let auth = test_support::trusting(ferrohealth_smart::config::IssuerEntry {
            mode: ferrohealth_smart::config::AuthMode::Introspection,
            introspection: Some(ferrohealth_smart::config::IntrospectionSection {
                client: Some(ferrohealth_smart::config::credentials::OauthSection {
                    client_id: "ferroehr".to_owned(),
                    client_secret: Some(SecretString::from("synthetic-secret")),
                    ..ferrohealth_smart::config::credentials::OauthSection::default()
                }),
                ..ferrohealth_smart::config::IntrospectionSection::default()
            }),
            ..test_support::entry(&uri)
        });
        let authenticator = Authenticator::new(
            &AppConfig {
                auth,
                ..AppConfig::default()
            },
            None,
            None,
            memory(),
        )
        .await
        .expect("an introspecting authenticator");
        (server, authenticator)
    }

    /// Mounts `body` with `status` as the introspection answer.
    async fn introspection_answers(server: &wiremock::MockServer, status: u16, body: Value) {
        use wiremock::matchers::{method, path};
        wiremock::Mock::given(method("POST"))
            .and(path("/introspect"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    /// The configuration of an issuer at `issuer` checked by introspection,
    /// authenticating with a client grant or, with `bearer`, a bearer grant,
    /// and trusting `ca_file` beside the platform roots when given.
    fn introspecting_through(
        issuer: &str,
        bearer: bool,
        ca_file: Option<std::path::PathBuf>,
    ) -> AppConfig {
        let grant = ferrohealth_smart::config::credentials::OauthSection {
            token_endpoint: format!("{issuer}/token"),
            client_id: "ferroehr".to_owned(),
            client_secret: Some(SecretString::from("synthetic-secret")),
            tls: ferrohealth_config::tls::TlsClientSection {
                trust_roots_file: ca_file.clone(),
                ..ferrohealth_config::tls::TlsClientSection::default()
            },
            ..ferrohealth_smart::config::credentials::OauthSection::default()
        };
        let (client, bearer) = if bearer {
            (None, Some(grant))
        } else {
            (Some(grant), None)
        };
        let mut auth = test_support::trusting(ferrohealth_smart::config::IssuerEntry {
            mode: ferrohealth_smart::config::AuthMode::Introspection,
            introspection: Some(ferrohealth_smart::config::IntrospectionSection {
                endpoint: format!("{issuer}/introspect"),
                client,
                bearer,
                ..ferrohealth_smart::config::IntrospectionSection::default()
            }),
            ..test_support::entry(issuer)
        });
        auth.shared.tls.trust_roots_file = ca_file;
        AppConfig {
            auth,
            ..AppConfig::default()
        }
    }

    /// `[auth.tls] trust_roots_file`: an issuer whose TLS certificate a private
    /// certificate authority signed is read at boot trusting that authority
    /// beside the platform roots, under either introspection credential, and
    /// refused without it; a file that holds no certificate refuses the start.
    /// No openEHR spec governs trust configuration: our own design.
    #[tokio::test]
    async fn an_issuer_behind_a_private_authority_is_read_with_auth_tls() {
        let issuer = testkit::https::HttpsServer::answering_with("application/json", |base| {
            json!({ "issuer": base }).to_string()
        })
        .await
        .expect("the private issuer starts");
        let empty = issuer.ca_file.with_extension("empty.pem");
        std::fs::write(&empty, b"").expect("the empty file is written");
        for bearer in [false, true] {
            let config =
                introspecting_through(&issuer.base_url, bearer, Some(issuer.ca_file.clone()));
            Authenticator::new(&config, None, None, memory())
                .await
                .unwrap_or_else(|e| panic!("bearer={bearer}: read trusting the roots file: {e}"));
            let untrusted = introspecting_through(&issuer.base_url, bearer, None);
            let refused = Authenticator::new(&untrusted, None, None, memory())
                .await
                .expect_err("the platform roots alone do not trust the private authority");
            assert!(
                refused.contains("could not be read"),
                "bearer={bearer}: {refused}"
            );
            let no_certificate =
                introspecting_through(&issuer.base_url, bearer, Some(empty.clone()));
            let refused = Authenticator::new(&no_certificate, None, None, memory())
                .await
                .expect_err("a roots file without a certificate refuses the start");
            assert!(refused.contains("tls"), "bearer={bearer}: {refused}");
        }
    }

    /// RFC 7662 §2.2: an active token admits its caller with its scopes and
    /// roles, an inactive one is a `401`, and an endpoint that cannot answer
    /// is a `503`, never a refusal of a token that may be valid.
    #[tokio::test]
    async fn an_opaque_token_is_checked_by_introspection() {
        let (server, auth) = introspecting().await;
        introspection_answers(
            &server,
            200,
            json!({
                "active": true,
                "scope": "user/composition-*.r",
                "sub": "alice",
                "client_id": "unit-test-client",
                "aud": AUDIENCE,
                "iss": server.uri(),
                "exp": jiff::Timestamp::now().as_second() + 300,
                "roles": ["clinician"]
            }),
        )
        .await;
        let p = auth
            .authenticate(&headers("Bearer synthetic-opaque-token"))
            .await
            .expect("an active token")
            .principal;
        assert_eq!(p.subject, "alice");
        assert_eq!(p.method, AuthMethod::Bearer);
        assert_eq!(p.scopes, vec!["user/composition-*.r".to_owned()]);
        assert_eq!(p.roles, vec!["CLINICIAN".to_owned()]);

        let (server, auth) = introspecting().await;
        introspection_answers(&server, 200, json!({ "active": false })).await;
        let err = auth
            .authenticate(&headers("Bearer synthetic-opaque-token"))
            .await
            .expect_err("an inactive token");
        assert_eq!(err.to_api_error().status(), StatusCode::UNAUTHORIZED);

        let (server, auth) = introspecting().await;
        introspection_answers(&server, 500, json!({ "error": "server_error" })).await;
        let err = auth
            .authenticate(&headers("Bearer synthetic-opaque-token"))
            .await
            .expect_err("no answer");
        assert_eq!(err.to_api_error().status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    async fn basic_only() -> Arc<Authenticator> {
        Authenticator::new(&basic_config(60), None, None, memory())
            .await
            .expect("a Basic-only authenticator")
    }

    async fn bearer_only() -> Arc<Authenticator> {
        Authenticator::new(
            &AppConfig {
                auth: test_support::bearer_auth(),
                ..AppConfig::default()
            },
            None,
            None,
            memory(),
        )
        .await
        .expect("a bearer authenticator")
    }

    fn headers(auth: &str) -> Parts {
        let request = http::Request::get("/ferroehr/rest/openehr/v1/ehr")
            .header(header::AUTHORIZATION, HeaderValue::from_str(auth).unwrap())
            .body(())
            .unwrap();
        request.into_parts().0
    }

    fn bearer(claims: &Value) -> Parts {
        headers(&format!("Bearer {}", test_support::token(claims)))
    }

    fn now() -> i64 {
        jiff::Timestamp::now().as_second()
    }

    /// A credential the cache holds never admits another password for the
    /// same user, with the cache on or off.
    #[tokio::test]
    async fn a_cached_credential_never_admits_a_wrong_password() {
        for ttl in [60, 0] {
            let auth = Authenticator::new(&basic_config(ttl), None, None, memory())
                .await
                .unwrap();
            let h = headers("Basic YWxpY2U6cHc="); // alice:pw
            let first = auth.authenticate(&h).await.expect("first verifies");
            let second = auth.authenticate(&h).await.expect("second verifies");
            assert!(first.fresh, "the first Basic request is an authentication");
            assert_eq!(
                ttl == 0,
                second.fresh,
                "a repeat the cache admits is no new authentication; without a cache it is"
            );
            for bad in [
                "Basic YWxpY2U6d3Jvbmc=", // alice:wrong
                "Basic YWxpY2U6cHcg",     // alice:"pw "
                "Basic Ym9iOnB3",         // bob:pw, an unknown user
            ] {
                let err = auth
                    .authenticate(&headers(bad))
                    .await
                    .expect_err("a credential the store does not hold is refused");
                assert!(
                    matches!(err, AuthError::InvalidCredentials),
                    "{bad}: {err:?}"
                );
            }
        }
    }

    /// RFC 6750 §2.1 holds a bearer credential to `"Bearer" 1*SP b64token`:
    /// a header that is no scheme and credential is a malformed request
    /// (`400 invalid_request`, §3.1), and a token outside the `b64token`
    /// alphabet is a malformed token (`401 invalid_token`, §3.1).
    #[tokio::test]
    async fn a_bearer_credential_outside_rfc6750_is_refused() {
        let auth = bearer_only().await;
        for (malformed, request_defect) in [
            ("Bearer", true),          // no separator, no token
            ("Bearer ", true),         // separator, no token
            ("Bearer\tabc", true),     // a tab is not SP
            ("Bearer abc def", false), // a space inside the token
            ("Bearer abc?def", false), // `?` is outside b64token
            ("Bearer a=bc", false),    // interior padding
            ("Bearer =abc", false),    // padding alone at the front
        ] {
            let err = auth
                .authenticate(&headers(malformed))
                .await
                .expect_err("a malformed bearer credential is refused");
            if request_defect {
                assert!(
                    matches!(err, AuthError::MalformedRequest(_)),
                    "`{malformed}`: {err:?}"
                );
                assert_eq!(err.to_api_error().status(), StatusCode::BAD_REQUEST);
            } else {
                assert!(
                    matches!(err, AuthError::InvalidToken),
                    "`{malformed}`: {err:?}"
                );
                assert_eq!(err.to_api_error().status(), StatusCode::UNAUTHORIZED);
            }
        }
        let err = auth
            .authenticate(&headers("Basic abc"))
            .await
            .expect_err("a scheme the surface does not admit");
        assert!(matches!(err, AuthError::MissingCredentials), "{err:?}");
    }

    #[tokio::test]
    async fn missing_credentials() {
        let err = basic_only()
            .await
            .authenticate(&http::Request::new(()).into_parts().0)
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
            Vec::<String>::new(),
            "a Basic user granted no role holds none"
        );
        assert!(
            p.fresh,
            "a first Basic verification is a genuine authentication"
        );
    }

    #[tokio::test]
    async fn configured_basic_roles_are_upper_cased() {
        let mut config = basic_config(60);
        if let Some(basic) = config.auth.shared.basic.as_mut()
            && let Some(alice) = basic.users.first_mut()
        {
            alice.roles = vec!["admin".to_owned(), "user".to_owned()];
        }
        let auth = Authenticator::new(&config, None, None, memory())
            .await
            .unwrap();
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

    /// RFC 9068 §4: an issuer whose `token_types` names `JWT` admits a plainly
    /// typed token, and refuses an untyped one and another kind of JWT.
    #[tokio::test]
    async fn token_types_admit_a_plain_jwt_and_refuse_another_kind() {
        let auth = test_support::trusting(ferrohealth_smart::config::IssuerEntry {
            jwks_file: test_support::jwks_file().display().to_string(),
            token_types: Some(vec!["JWT".to_owned()]),
            required_claims: Some(["iss", "exp", "aud", "sub"].map(str::to_owned).to_vec()),
            ..test_support::entry(ISSUER)
        });
        let authenticator = Authenticator::new(
            &AppConfig {
                auth,
                ..AppConfig::default()
            },
            None,
            None,
            memory(),
        )
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
                "client_id": "unit-test-client",
            });
            claims["scope"] = json!("user/composition-*.r");
            jsonwebtoken::encode(&header, &claims, &key).expect("encode")
        };
        authenticator
            .authenticate(&headers(&format!("Bearer {}", mint(Some("JWT")))))
            .await
            .expect("a plain JWT authenticates");
        for typ in [None, Some("JWT+ID")] {
            let err = authenticator
                .authenticate(&headers(&format!("Bearer {}", mint(typ))))
                .await
                .expect_err("an untyped token and an ID token are refused");
            assert!(matches!(err, AuthError::InvalidToken), "got {err:?}");
        }
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

    /// The roles `ferrohealth-smart` reads at a Keycloak deployment's claims,
    /// as the authenticator hands them on.
    fn default_roles(claims: &Value) -> Vec<String> {
        let claims = claims.as_object().cloned().unwrap();
        upper_cased(&ferrohealth_smart::roles::from_claims(
            &claims,
            &test_support::keycloak_role_claims(),
        ))
    }

    #[test]
    fn extract_roles_upper_cases_and_dedups() {
        let roles = default_roles(&json!({
            "realm_access": { "roles": ["admin", "Admin", "user"] },
            "scope": "user OFFLINE_ACCESS",
        }));
        assert_eq!(roles, vec!["ADMIN".to_owned(), "USER".to_owned()]);
    }

    #[test]
    fn extract_roles_missing_paths_yields_empty() {
        assert_eq!(default_roles(&json!({ "sub": "x" })), Vec::<String>::new());
    }

    #[test]
    fn extract_roles_custom_nested_path() {
        let claims = json!({ "resource_access": { "client": { "roles": ["writer"] } } })
            .as_object()
            .cloned()
            .unwrap();
        let roles = upper_cased(&ferrohealth_smart::roles::from_claims(
            &claims,
            &["resource_access.client.roles"],
        ));
        assert_eq!(roles, vec!["WRITER".to_owned()]);
    }

    /// An OAuth2 scope grants a CLIENT delegated authority (RFC 6749 §3.3); it
    /// asserts nothing about the subject's roles. Mining it made the
    /// at-least-one-role gate vacuous for every OIDC token, since `openid`
    /// alone satisfied it.
    #[test]
    fn scope_claim_does_not_grant_roles() {
        let roles = default_roles(&json!({
            "realm_access": { "roles": ["user", "ferroehr-admin"] },
            "scope": "openid EHR_READ",
        }));
        assert!(roles.contains(&"USER".to_owned()));
        assert!(roles.contains(&"FERROEHR-ADMIN".to_owned()));
        assert!(
            !roles.contains(&"OPENID".to_owned()),
            "`openid` is a scope, not a role",
        );
        assert!(
            !roles.contains(&"EHR_READ".to_owned()),
            "a scope must not become a role",
        );
    }

    /// The RFC 9068 §2.2.3.1 carriers are read in order, so an issuer using the
    /// flat `roles`/`groups`/`entitlements` shape needs no configuration.
    #[test]
    fn rfc9068_role_claims_extracted() {
        for carrier in ["roles", "groups", "entitlements"] {
            assert_eq!(
                default_roles(&json!({ carrier: ["clinician"] })),
                vec!["CLINICIAN".to_owned()],
                "the `{carrier}` claim must carry roles",
            );
        }
    }

    #[test]
    fn dedups_preserving_order() {
        // The same role from two configured carriers collapses to one entry.
        assert_eq!(
            default_roles(&json!({
                "roles": ["admin", "Admin"],
                "realm_access": { "roles": ["ADMIN"] },
            })),
            vec!["ADMIN".to_owned()]
        );
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
            authenticator: Authenticator::new(&config, None, None, memory())
                .await
                .expect("authenticator"),
            authz: None,
            patient_data: None,
            smart: Arc::new(SmartRoutes::new(
                &crate::api::Namespaces::of(&ferroehr::config::rest::ApiConfig::default()),
                "__ferroehr.platform",
                Vec::new(),
            )),
            served: Arc::new(ServedMethods::default()),
            service: Arc::new(ferroehr::service::FerroEhrService::new(
                &ferroehr::db::domain::DomainPools::from_shared(
                    &sqlx::postgres::PgPoolOptions::new()
                        .connect_lazy_with(sqlx::postgres::PgConnectOptions::new()),
                ),
            )),
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

    /// A metadata file written for one test, removed when dropped.
    struct MetadataFile(std::path::PathBuf);

    impl MetadataFile {
        fn new(name: &str, body: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ferroehr-rest-issuer-metadata-{name}-{}.json",
                std::process::id()
            ));
            std::fs::write(&path, body).expect("write the metadata file");
            Self(path)
        }
    }

    impl Drop for MetadataFile {
        fn drop(&mut self) {
            drop(std::fs::remove_file(&self.0));
        }
    }

    /// A `jwks_file` issuer authenticator reading `[auth.smart] issuer_metadata_file`.
    async fn with_metadata_file(file: Option<&MetadataFile>) -> Result<Arc<Authenticator>, String> {
        let mut config = AppConfig {
            auth: test_support::bearer_auth(),
            ..AppConfig::default()
        };
        config.auth.smart.issuer_metadata_file = file.map(|file| file.0.display().to_string());
        Authenticator::new(&config, None, None, memory()).await
    }

    /// SMART App Launch 2.2.0, Conformance, §Metadata requires a
    /// `token_endpoint`: a `jwks_file` issuer names one only through the
    /// operator-supplied metadata, which must name that issuer (RFC 8414 §3.3)
    /// and parse, never quoting its input when it does not.
    #[tokio::test]
    async fn a_jwks_file_issuer_takes_its_metadata_from_the_configured_file() {
        let options = SmartOptions::default();
        let without = with_metadata_file(None).await.expect("authenticator");
        let document = without.smart_configuration(&options).expect("a document");
        assert!(document.get("token_endpoint").is_none(), "{document}");

        let good = MetadataFile::new(
            "good",
            &json!({ "issuer": ISSUER, "token_endpoint": format!("{ISSUER}/token") }).to_string(),
        );
        let with = with_metadata_file(Some(&good))
            .await
            .expect("authenticator");
        let document = with.smart_configuration(&options).expect("a document");
        assert_eq!(
            document.get("token_endpoint").and_then(Value::as_str),
            Some("https://issuer.example/token"),
            "{document}"
        );

        let other = MetadataFile::new(
            "other",
            &json!({ "issuer": "https://other.example", "token_endpoint": "https://other.example/token" })
                .to_string(),
        );
        let Err(message) = with_metadata_file(Some(&other)).await else {
            panic!("metadata of another issuer is refused");
        };
        assert!(message.contains("issuer_metadata_file"), "{message}");

        let insecure = MetadataFile::new(
            "insecure",
            &json!({ "issuer": ISSUER, "token_endpoint": "http://issuer.example/token" })
                .to_string(),
        );
        assert!(with_metadata_file(Some(&insecure)).await.is_err());

        let broken = MetadataFile::new("broken", r#"{"issuer": "s3cr3t-in-the-input""#);
        let Err(message) = with_metadata_file(Some(&broken)).await else {
            panic!("a document that does not parse is refused");
        };
        assert!(!message.contains("s3cr3t"), "{message}");
    }

    /// The metadata file goes through the shared parse: RFC 8414 §3.3 refuses
    /// metadata naming another issuer, and RFC 8259 §4 leaves a repeated member
    /// name to each parser, so the shared parse refuses it.
    #[tokio::test]
    async fn a_metadata_file_of_another_issuer_or_with_a_repeated_member_is_refused() {
        let foreign = MetadataFile::new(
            "foreign",
            &json!({ "issuer": "https://foreign.example", "token_endpoint": "https://foreign.example/token" })
                .to_string(),
        );
        let Err(message) = with_metadata_file(Some(&foreign)).await else {
            panic!("metadata of a foreign issuer is refused");
        };
        assert!(message.contains("issuer_metadata_file"), "{message}");
        assert!(message.contains("https://foreign.example"), "{message}");

        let repeated = MetadataFile::new(
            "repeated",
            &format!(
                r#"{{"issuer": "{ISSUER}", "token_endpoint": "{ISSUER}/token", "x_note": "a", "x_note": "b"}}"#
            ),
        );
        let Err(message) = with_metadata_file(Some(&repeated)).await else {
            panic!("metadata with a repeated member is refused");
        };
        assert!(message.contains("issuer_metadata_file"), "{message}");
        assert!(message.contains("repeats a member name"), "{message}");
    }

    mod remote_issuer;
}
