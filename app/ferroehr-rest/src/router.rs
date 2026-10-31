// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Router assembly: FerroEHR's routes inside the family stack.
//!
//! Every ITS-REST resource path is relative to the configured API base path
//! (overview `Resources.md` §Resource identification), so the generated API
//! surface ([`crate::api`]) nests under `rest.base_path`. The `/rest/status`
//! document hangs off the `/ferroehr/rest` root and the System Options manifest
//! answers at the base-path root itself.
//!
//! [`routes`] is FerroEHR's own: over the nested API, innermost first, the
//! caller rate (`[server.caller_rate]`, keyed on the authenticated subject),
//! authentication, the ATNA audit (wrapping auth so it observes auth
//! rejections) and the clinical body tier; then CORS over the whole surface,
//! with the System Options `OPTIONS` endpoint mounted beside it, since the CORS
//! layer would otherwise treat it as a preflight. [`router`] wraps those routes
//! in `ferrohealth_server::stack::app`, which adds the request id, the panic
//! catch, the request timeout, the body limit, the overload bound, compression,
//! the request log and the health routes, and writes its panics and refusals as
//! RFC 9457 problem details ([`stack_policy`]). The trailing-slash
//! normalization runs before routing and the security headers outermost.
//!
//! The overload bound covers every FerroEHR route and never the health routes,
//! so an orchestrator can always probe an overloaded server. No openEHR spec
//! governs server overload — our own design (RFC 9110 §15.6.4).

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction); the carriers here are \
              cfg(test)-only, so #[expect] would be unfulfilled in the non-test build"
)]

use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use ferrohealth_server::caller_rate::{self, CallerKey, CallerLimit};
use ferrohealth_server::health::Health;
use ferrohealth_server::panic::PanicBody;
use ferrohealth_server::refusal::RefusalBody;
use ferrohealth_server::request_id::RequestIds;
use ferrohealth_server::stack::{self, StackPolicy};
use http::header::AUTHORIZATION;
use http::{HeaderName, HeaderValue, StatusCode, header};
use tower::{Layer as _, ServiceBuilder};
use tower_http::cors::CorsLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::api::system;
use crate::extensions::access::authn::{self, Authenticator, Principal};
use crate::extensions::management::{self, ManagementState};
use crate::extensions::openapi;
use crate::overview::problem::{self, FhirBase};
use crate::overview::{error, status};
use crate::smart;
use crate::state::AppState;
use ferroehr::config::management::AccessLevel;
use ferrohealth_telemetry::request_log::RequestLog;

/// Assembles the full application: [`routes`] inside the family stack, the
/// management surface merged when it shares the main listener, the
/// trailing-slash normalization and the security headers.
pub fn router(state: AppState, authenticator: Arc<Authenticator>) -> Router {
    let cfg = state.config().clone();
    let observability = state.observability().clone();
    let product = routes(state.clone(), &authenticator);
    // Only when enabled and not bound to a separate port (which the binary
    // serves on its own listener).
    let product = if observability.management.enabled && observability.management.port.is_none() {
        product.merge(audited_management(state, authenticator))
    } else {
        product
    };
    let health = Health::new(
        observability.health.clone(),
        observability.lifecycle.clone(),
    );
    let app = stack::app(
        product,
        health,
        &cfg.server,
        stack_policy(
            observability.request_log.clone(),
            FhirBase::of(&cfg.rest.rest_root()),
        ),
    );
    with_security_headers(normalized(app))
}

/// Runs the request inside [`ferroehr::db::acquire::request_scope`], so it
/// checks a connection out of each pool once and keeps it until it answers.
///
/// No openEHR spec governs connection handling: our own design.
async fn one_connection_per_request(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    ferroehr::db::acquire::request_scope(next.run(request)).await
}

/// The management surface inside the audit trail (#3244): every management
/// request is recorded as a system-domain access, `/management/loggers` writes
/// included.
fn audited_management(state: AppState, authenticator: Arc<Authenticator>) -> Router {
    management::router(ManagementState::from_app_state(
        &state,
        authenticator,
        management_rbac(&state),
    ))
    .layer(from_fn_with_state(
        state,
        crate::system_log::middleware::middleware,
    ))
}

/// FerroEHR's routes without the family stack.
///
/// The ITS-REST API behind authentication, nested under the base path, the
/// status and SMART-discovery endpoints, the optional Swagger UI and the System
/// Options manifest.
pub fn routes(state: AppState, authenticator: &Arc<Authenticator>) -> Router {
    let cfg = state.config().clone();
    let rest_root = cfg.rest.rest_root();
    let namespaces = crate::api::Namespaces::of(&cfg.rest);

    // A known path called with a method it does not serve renders `405` as
    // problem details (overview §HTTP Methods); axum supplies the mandatory `Allow`
    // (RFC 9110 §15.5.6) from the matched route's method set. An unrecognized
    // method is `405` too: a catch-all fallback would report every unknown path
    // `501` instead of `404`.
    // NOTE: the overview's SHOULD-`501` (`Requests_and_responses.md` §HTTP
    // Methods) is honoured for a recognised but unimplemented operation via
    // `ApiError::NotImplemented`.
    let api = crate::api::api_router(&namespaces)
        .method_not_allowed_fallback(error::method_not_allowed_handler);
    // Added before the authentication layer, which in axum's outermost-last
    // ordering puts it inside: the subject it keys on exists only after auth.
    let api = match cfg.server.caller_rate {
        Some(rate) => api
            .layer(from_fn_with_state(
                CallerLimit::new(rate),
                caller_rate::limit,
            ))
            .layer(axum::middleware::from_fn(caller_key)),
        None => api,
    };
    let api = api.layer(from_fn_with_state(
        authn::AuthLayer {
            authenticator: Arc::clone(authenticator),
            authz: state.authz(),
            patient_data: authn::assurance::PatientDataGate::new(
                authenticator.bearer_admitted(),
                &cfg.assurance,
                cfg.professional.as_ref(),
                namespaces.clone(),
                ferroehr::config::auth::realm(&cfg.auth),
            )
            .map(Arc::new),
            smart: Arc::new(smart::routes::SmartRoutes::new(
                &namespaces,
                ferroehr::config::auth::product_scope(&cfg.auth),
                cfg.smart.demographic_clients.clone(),
            )),
            served: Arc::new(crate::api::served::ServedMethods::new(&namespaces)),
        },
        authn::middleware,
    ));
    // Always installed: the layer early-returns when the `SystemLog` reports
    // auditing off, so the no-audit case costs one check per request.
    let api = api.layer(from_fn_with_state(
        state.clone(),
        crate::system_log::middleware::middleware,
    ));
    // Above the audit layer so an over-large body is refused before it is read,
    // buffered or audited; the stack's body limit is the ceiling of every route.
    let api = api.layer(from_fn_with_state(state.clone(), crate::limits::middleware));
    // Outermost, so every statement of the request, its authorization
    // lookups included, runs on the one connection each pool lent it.
    let api = api.layer(axum::middleware::from_fn(one_connection_per_request));

    // A path no route serves answers a `404` problem, never an empty body.
    let inner = mount_public_surface(&cfg, &state, authenticator, api, &rest_root)
        .fallback(not_found)
        .layer(from_fn_with_state(
            FhirBase::of(&rest_root),
            problem::finish,
        ));

    let cors = if cfg.rest.cors_permissive {
        CorsLayer::very_permissive()
    } else {
        CorsLayer::new()
    };
    let inner: Router = inner
        .layer(
            ServiceBuilder::new()
                .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
                    AUTHORIZATION,
                )))
                .layer(cors),
        )
        .with_state(state);

    // The manifest advertises the live mounted-group set, so it never names a
    // group that answers 404; extension families declare themselves through the
    // served OpenAPI document instead.
    // NOTE: no released text says what `endpoints` must contain — the System API
    // chapter is a stub and `schemas/others/Options.yaml` gives only
    // `array of string` — so the content above is our own adjudication.
    let mut endpoints = vec![
        "/ehr".to_owned(),
        "/definition".to_owned(),
        "/query".to_owned(),
        "/demographic".to_owned(),
    ];
    if cfg.admin.enabled {
        endpoints.push("/admin".to_owned());
    }
    let manifest = Arc::new(system::options::SystemManifest::new(
        cfg.rest.identity.clone(),
        endpoints,
    ));

    // The API base-path root is the one location the System API defines
    // (`system.openapi.yaml`: servers `{baseUrl}/v1`, path `/`). The route is
    // merged beside the CORS-wrapped surface so `OPTIONS` is not eaten as a
    // preflight.
    inner.merge(Router::new().route(&cfg.rest.base_path, system::options::route(manifest)))
}

/// The `404` of a path no route serves, as an `about:blank` problem.
async fn not_found() -> Response {
    error::status_error_response(StatusCode::NOT_FOUND, "no resource is served at this path")
}

/// Puts the authenticated subject on the request as the caller the
/// `[server.caller_rate]` limit keys on.
///
/// Runs inside the authentication layer, so [`Principal`] is in the request
/// extensions. A request without one means authentication is disabled
/// altogether; those share a single key, which is the honest behaviour: with no
/// identity there is nothing fairer to key on. No openEHR spec governs request
/// rates — our own design.
async fn caller_key(mut request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let subject = request.extensions().get::<Principal>().map_or_else(
        || Arc::from("-"),
        |principal| Arc::from(principal.subject.as_str()),
    );
    request.extensions_mut().insert(CallerKey(subject));
    next.run(request).await
}

/// What FerroEHR decides inside the family stack.
///
/// A panic and each refusal the stack writes (`408`, `413`, `429`, `503`)
/// answer as problem details, or as an `OperationOutcome` under the FHIR base
/// ([`problem::stack_answer`]), and the request log is the one the
/// observability bundle carries.
#[must_use]
pub fn stack_policy(request_log: RequestLog, fhir: FhirBase) -> StackPolicy {
    StackPolicy {
        panic_body: panic_body(fhir.clone()),
        refusal_body: refusal_body(fhir),
        request_log,
    }
}

/// Renders a panicked handler as a `500` problem with the curated opaque
/// detail; the panic payload never reaches the body or the log.
///
/// The `instance` names the server's request id, as every refusal a FerroEHR
/// handler writes does ([`problem::finish`]), so one id finds the exchange in
/// the request log whichever layer answered.
fn panic_body(fhir: FhirBase) -> PanicBody {
    Arc::new(move |uri, ids: RequestIds<'_>| {
        problem::stack_answer(
            &fhir,
            uri,
            ids.server,
            StatusCode::INTERNAL_SERVER_ERROR,
            error::INTERNAL_MESSAGE,
        )
    })
}

/// Renders a refusal the stack writes itself as a problem, keeping its
/// status.
///
/// `408` is a predefined status (`Requests_and_responses.md` §HTTP status
/// codes) and `413`, `429` and `503` additional codes the same section permits.
/// The `instance` names the server's request id, as [`panic_body`]'s does.
fn refusal_body(fhir: FhirBase) -> RefusalBody {
    Arc::new(move |uri, ids: RequestIds<'_>, status, reason| {
        problem::stack_answer(&fhir, uri, ids.server, status, reason)
    })
}

/// Trims a trailing slash before routing, so `/ehr/` and `/ehr` reach one route.
fn normalized(app: Router) -> Router {
    Router::new().fallback_service(NormalizePathLayer::trim_trailing_slash().layer(app))
}

/// Applies the response headers every surface carries, per the OWASP HTTP
/// Headers Cheat Sheet.
///
/// Outermost, so they reach transport-layer responses (the body-limit `413`, the
/// timeout `408`, the panic-handler `500`) as well as handler ones. The set is
/// chosen for a clinical JSON API: `Cache-Control: no-store` because responses
/// carry patient data, `X-Content-Type-Options: nosniff`, a
/// `Referrer-Policy` that keeps `ehr_id`-bearing paths out of cross-origin
/// `Referer` headers, `Cross-Origin-Resource-Policy: same-site`, and
/// `X-Frame-Options: DENY` plus a minimal CSP for the HTML this origin can serve.
///
/// `Strict-Transport-Security` is deliberately absent: RFC 6797 §7.2 requires a
/// browser to ignore it over plain HTTP, and this server is commonly reached over
/// HTTP behind a proxy that owns the header.
fn with_security_headers(app: Router) -> Router {
    app.layer(
        ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::REFERRER_POLICY,
                HeaderValue::from_static("strict-origin-when-cross-origin"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                HeaderName::from_static("cross-origin-resource-policy"),
                HeaderValue::from_static("same-site"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::X_FRAME_OPTIONS,
                HeaderValue::from_static("DENY"),
            ))
            // `if_not_present`, not `overriding`: a rendered surface needs its
            // own policy, and this outermost layer would otherwise erase it.
            .layer(SetResponseHeaderLayer::if_not_present(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
            )),
    )
}

/// Mounts the public, pre-auth surface around the API tree: the REST-root
/// status surface, the config-gated SMART `/.well-known/smart-configuration`
/// document (SMART master04 §Service Discovery; an empty router when SMART is
/// disabled), and the Swagger UI plus `OpenAPI` documents at the access level
/// `rest.swagger_ui` names — `public` is open, `private`/`admin_only` sit
/// behind the same access guard the management endpoints use, `off` is not
/// mounted.
///
/// No openEHR spec governs this — our own operational surface. The health
/// routes are the family stack's, outside these routes and their guards.
fn mount_public_surface(
    cfg: &crate::config::AppConfig,
    state: &AppState,
    authenticator: &Arc<Authenticator>,
    api: Router<AppState>,
    rest_root: &str,
) -> Router<AppState> {
    // A pure function of static configuration, built once inside the router.
    let discovery = smart::discovery::router(cfg, rest_root, authenticator);

    let mut inner = Router::new()
        .merge(api)
        .merge(status::router(rest_root))
        .merge(discovery);

    match cfg.rest.swagger_ui {
        AccessLevel::Off => {}
        AccessLevel::Public => inner = inner.merge(openapi::swagger_router(cfg)),
        level @ (AccessLevel::Private | AccessLevel::AdminOnly) => {
            // `route_layer`, never `layer`: a layer also wraps the router's
            // fallback, so after the merge every unmatched path outside the
            // API (a SMART discovery document that is switched off, a typo)
            // answered the guard's `401` instead of `404`.
            inner = inner.merge(openapi::swagger_router(cfg).route_layer(from_fn_with_state(
                management::AccessGuard {
                    authenticator: Arc::clone(authenticator),
                    rbac: management_rbac(state),
                    level,
                },
                management::access_middleware,
            )));
        }
    }
    inner
}

/// Returns the RBAC rules the management `AdminOnly` gate applies: the live
/// handle's rule set, or a disabled one when no RBAC gate is wired, where being
/// authenticated is enough.
fn management_rbac(state: &AppState) -> ferroehr::config::authz::RbacConfig {
    state
        .authz()
        .as_deref()
        .and_then(|h| h.rbac_rules().cloned())
        .unwrap_or_else(|| ferroehr::config::authz::RbacConfig {
            enabled: false,
            ..ferroehr::config::authz::RbacConfig::default()
        })
}

/// Builds the standalone management application for separate-port mode.
///
/// The binary serves it on the management listener when `management.port` is
/// set: the management routes inside the family stack, with the same health
/// routes.
pub fn management_router(state: &AppState, authenticator: Arc<Authenticator>) -> Router {
    let observability = state.observability();
    let health = Health::new(
        observability.health.clone(),
        observability.lifecycle.clone(),
    );
    let routes = management::router(ManagementState::from_app_state(
        state,
        authenticator,
        management_rbac(state),
    ))
    .layer(from_fn_with_state(FhirBase::default(), problem::finish));
    normalized(stack::app(
        routes,
        health,
        &state.config().server,
        stack_policy(observability.request_log.clone(), FhirBase::default()),
    ))
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use http::StatusCode;

    use ferrohealth_server::request_id::RequestIds;

    use super::{panic_body, refusal_body};
    use crate::overview::problem::{BASE, FhirBase};

    const ID: &str = "0190a6f3-7c1e-7d7a-9f43-1c5b8a2e4d10";
    const CLIENT_ID: &str = "7d2c41e8-93b0-4f6a-8e15-c0a9b3d7e254";

    /// The ids of an exchange whose client sent `X-Request-Id: CLIENT_ID`.
    fn ids() -> RequestIds<'static> {
        RequestIds::new(ID, Some(CLIENT_ID))
    }

    /// A panicked handler renders the problem details every other error path
    /// emits (never a bare status), and nothing of the request reaches it:
    /// the `instance` is the server's id, never the one the client sent.
    #[tokio::test]
    async fn the_panic_body_is_a_problem() {
        let uri: http::Uri = "/ferroehr/rest/openehr/v1/ehr/Qz7sentinel".parse().unwrap();
        let resp = panic_body(FhirBase::of("/ferroehr/rest"))(&uri, ids());
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["type"], "about:blank");
        assert_eq!(body["title"], "Internal Server Error");
        assert_eq!(body["status"], 500);
        assert_eq!(body["instance"], format!("urn:uuid:{ID}"));
        assert!(!body.to_string().contains("Qz7sentinel"), "{body}");
    }

    /// Each refusal the stack writes keeps its status and carries the reason
    /// as the problem's detail; the availability refusals are FerroEHR types,
    /// and the `instance` is the server's id.
    #[tokio::test]
    async fn a_refusal_is_a_problem_with_its_status() {
        let uri: http::Uri = "/ferroehr/rest/openehr/v1/ehr".parse().unwrap();
        for (status, kind) in [
            (StatusCode::REQUEST_TIMEOUT, "about:blank".to_owned()),
            (StatusCode::PAYLOAD_TOO_LARGE, "about:blank".to_owned()),
            (StatusCode::TOO_MANY_REQUESTS, format!("{BASE}rate-limited")),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("{BASE}unavailable"),
            ),
        ] {
            let resp = refusal_body(FhirBase::default())(&uri, ids(), status, "the reason");
            assert_eq!(resp.status(), status);
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["type"], kind.as_str(), "{status}");
            assert_eq!(body["status"], status.as_u16(), "{status}");
            assert_eq!(body["detail"], "the reason");
            assert_eq!(body["instance"], format!("urn:uuid:{ID}"), "{status}");
        }
    }

    /// Under the FHIR base a stack refusal is an `OperationOutcome` (FHIR R4
    /// `http.html`).
    #[cfg(feature = "fhir")]
    #[tokio::test]
    async fn a_refusal_under_the_fhir_base_is_an_operation_outcome() {
        let uri: http::Uri = "/ferroehr/rest/fhir/r4/AuditEvent".parse().unwrap();
        let resp = refusal_body(FhirBase::of("/ferroehr/rest"))(
            &uri,
            ids(),
            StatusCode::REQUEST_TIMEOUT,
            "the reason",
        );
        assert_eq!(resp.status(), StatusCode::REQUEST_TIMEOUT);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["resourceType"], "OperationOutcome");
    }
}
