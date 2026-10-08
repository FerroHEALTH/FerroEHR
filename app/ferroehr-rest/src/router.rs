// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Router assembly and the `tower-http` middleware stack.
//!
//! Every ITS-REST resource path is relative to the configured API base path
//! (overview `Resources.md` §Resource identification), so the generated API
//! surface ([`crate::api`]) nests under `cfg.base_path`. The `/rest/status`
//! document hangs off the `/ferroehr/rest` root, the always-on health family
//! answers at the process root, and the System Options manifest answers at the
//! base-path root itself.
//!
//! Layer order over the nested API, innermost first: authentication, ATNA audit
//! (wrapping auth so it observes auth rejections), HTTP metrics, root span, then
//! overload shedding — outermost on that subtree, so a shed request is rejected
//! before auth, audit, or reading the body. The whole tree is then wrapped in the
//! shared `tower-http` stack (request-id, tracing, CORS, body limit, timeout,
//! compression), with `align_transport_error_body` between the CORS layer and the
//! body-limit/timeout pair to re-render their `408`/`413` in the openEHR
//! `{ error, message }` shape. The System Options `OPTIONS` endpoint mounts above
//! the CORS layer, which would otherwise treat it as a preflight.
//!
//! Shedding is scoped to the API subtree, so the health family, `/rest/status`,
//! SMART discovery and the management surface are never shed and an operator can
//! always reach an overloaded server. No openEHR spec governs server overload —
//! our own design (RFC 9110 §15.6.4).
//!
//! `NormalizePathLayer` is applied at serve time (it must wrap the router to run
//! before routing); see [`crate::serve_with`].

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction); the carriers here are \
              cfg(test)-only, so #[expect] would be unfulfilled in the non-test build"
)]

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderName, HeaderValue, header};
use openehr_its::rest::runtime::ApiError;
use tower::ServiceBuilder;
use tower_http::CompressionLevel;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::api::system;
use crate::extensions::access::authn::{self, Authenticator};
use crate::extensions::health;
use crate::extensions::management::{self, ManagementState};
use crate::extensions::openapi;
use crate::overview::{error, status};
use crate::smart;
use crate::state::AppState;
use ferroehr::config::management::AccessLevel;

/// Per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Assembles the full application router.
///
/// Mounts the ITS-REST API (behind the auth layer) nested under the base path,
/// the always-on public health family, the status + SMART-discovery endpoints,
/// the System Options manifest, the optional Swagger UI, and the shared
/// `tower-http` middleware stack.
pub fn router(state: AppState, authenticator: Arc<Authenticator>) -> Router {
    let cfg = state.config().clone();
    let mgmt_rbac = management_rbac(&state);
    let observability = state.observability().clone();
    let rest_root = cfg.server.rest_root();

    // A known path called with a method it does not serve renders `405` with the
    // openEHR body (overview §HTTP Methods); axum supplies the mandatory `Allow`
    // (RFC 9110 §15.5.6) from the matched route's method set. An unrecognized
    // method is `405` too: a catch-all fallback would report every unknown path
    // `501` instead of `404`.
    // NOTE: the overview's SHOULD-`501` (`Requests_and_responses.md` §HTTP
    // Methods) is honoured for a recognised but unimplemented operation via
    // `ApiError::NotImplemented`.
    let api =
        crate::api::api_router().method_not_allowed_fallback(error::method_not_allowed_handler);
    let api = api.layer(from_fn_with_state(
        authn::AuthLayer {
            authenticator: Arc::clone(&authenticator),
            authz: state.authz(),
            patient_data: authn::assurance::PatientDataGate::new(
                authenticator.config(),
                &cfg.server.base_path,
            )
            .map(Arc::new),
        },
        authn::middleware,
    ));
    // Always installed: the layer early-returns when the SM `SystemLog` reports
    // auditing off, so the no-audit case costs one check per request.
    let api = api.layer(from_fn_with_state(
        state.clone(),
        crate::system_log::middleware::middleware,
    ));
    // Added before the authentication layer, which in axum's outermost-last
    // ordering puts it inside: the subject it keys on exists only after auth.
    let api = match crate::rate_limit::principal_layer(&cfg.server.rate_limit) {
        Some(tier) => api.layer(tier),
        None => api,
    };
    // Above the audit layer so an over-large body is refused before it is read,
    // buffered or audited; the outer `RequestBodyLimitLayer` is the tree ceiling.
    let api = api.layer(from_fn_with_state(state.clone(), crate::limits::middleware));
    let api = api
        .layer(axum::middleware::from_fn(
            management::http_metrics::http_metrics,
        ))
        .layer(axum::middleware::from_fn(
            management::http_metrics::root_span,
        ));

    // Beyond `cfg.max_in_flight` concurrent API requests the excess is shed as
    // `503` + `Retry-After` rather than queued until memory runs out; `0`
    // installs no layer.
    let api = crate::overload::shed_layer(api, cfg.server.max_in_flight);

    let inner = mount_public_surface(&cfg, &state, &authenticator, api, &rest_root);

    let cors = if cfg.server.cors_permissive {
        CorsLayer::very_permissive()
    } else {
        CorsLayer::new()
    };

    // The address-keyed tier covers the whole tree, health family included: a
    // flood at any path is refused here, before authentication.
    let inner = match crate::rate_limit::address_layer(&cfg.server.rate_limit) {
        Some(tier) => inner.layer(tier),
        None => inner,
    };

    // The management surface sits outside the API tree but inside the audit
    // trail (#3244): every management request is recorded as a system-domain
    // access, `/management/loggers` writes included.
    let audit_state = state.clone();
    let inner: Router = with_shared_stack(inner, cfg.server.limits.ceiling(), cors, state);

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
        cfg.server.identity.clone(),
        endpoints,
    ));

    // The API base-path root is the one location the System API defines
    // (`system.openapi.yaml`: servers `{baseUrl}/v1`, path `/`). The mount sits
    // above CORS so `OPTIONS` is not eaten as a preflight; every other request
    // falls through to the CORS-wrapped application.
    let app: Router = Router::new()
        .route(&cfg.server.base_path, system::options::route(manifest))
        .fallback_service(inner);

    // Only when enabled and not bound to a separate port (which the binary
    // serves on its own listener).
    let app = if observability.management.enabled && observability.management.port.is_none() {
        let mgmt = management::router(ManagementState::from_app_state(
            &audit_state,
            authenticator,
            mgmt_rbac,
        ))
        .layer(from_fn_with_state(
            audit_state,
            crate::system_log::middleware::middleware,
        ));
        app.merge(mgmt)
    } else {
        app
    };
    with_security_headers(app)
}

/// Applies the shared request stack and binds the state, yielding a
/// protocol-complete, state-less service.
///
/// The stack is applied in two halves with [`align_transport_error_body`]
/// between them: `TimeoutLayer` and `RequestBodyLimitLayer` render their own
/// responses without ever reaching [`crate::overview::error`], so the mapping
/// must sit OUTSIDE both. Order (outermost → innermost): request-id, tracing,
/// catch-panic, CORS, body-limit ceiling, timeout, compression.
///
/// `body_ceiling` is the widest tier any route accepts
/// ([`BodyLimits::ceiling`](ferroehr::config::server::BodyLimits::ceiling)); the
/// narrower per-family tier is applied inside, by [`crate::limits`].
fn with_shared_stack(
    inner: Router<AppState>,
    body_ceiling: usize,
    cors: CorsLayer,
    state: AppState,
) -> Router {
    inner
        .layer(
            ServiceBuilder::new()
                .layer(RequestBodyLimitLayer::new(body_ceiling))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    REQUEST_TIMEOUT,
                ))
                // NOTE (docs.rs tower-http `CompressionLayer::quality`):
                // Fastest — the default brotli/gzip levels spend visible
                // per-response CPU for a marginal ratio gain on ~KB bodies.
                .layer(CompressionLayer::new().quality(CompressionLevel::Fastest)),
        )
        .layer(axum::middleware::map_response(align_transport_error_body))
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
                    AUTHORIZATION,
                )))
                .layer(TraceLayer::new_for_http().make_span_with(path_only_span))
                .layer(CatchPanicLayer::custom(handle_panic))
                .layer(cors),
        )
        .with_state(state)
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

/// Builds the `tower-http` request span, recording the path without its query
/// string.
///
/// `DefaultMakeSpan` would record the whole target, and `subject_id` — an
/// external patient identifier — is a query parameter (ITS-REST EHR API,
/// `GET /ehr?subject_id=…`), which identified data must never reach the ordinary
/// application log. The path keeps only system-assigned ids the audit log records
/// anyway. The route template is recorded separately by
/// [`management::http_metrics::root_span`], the span telemetry consumes.
fn path_only_span(request: &http::Request<axum::body::Body>) -> tracing::Span {
    tracing::debug_span!(
        "request",
        method = %request.method(),
        path = %request.uri().path(),
        version = ?request.version(),
    )
}

/// Renders a panicked handler as a `500` in the openEHR `{ error, message }`
/// body every other error path emits ([`crate::overview::error`]), never
/// tower-http's default `text/plain`.
///
/// The panic payload is logged but never echoed into the response. No openEHR
/// spec governs the error-body shape (a MAY, `Requests_and_responses.md` §HTTP
/// status codes) — our own design.
#[expect(
    clippy::needless_pass_by_value,
    reason = "tower-http's `CatchPanicLayer` handler contract hands the panic \
              payload over by value"
)]
fn handle_panic(err: Box<dyn Any + Send + 'static>) -> Response {
    let detail = if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_owned()
    };
    tracing::error!(panic = %detail, "request handler panicked");
    error::RestError(ApiError::Internal(error::INTERNAL_MESSAGE.to_owned())).into_response()
}

/// Aligns the two transport-layer error responses the `tower-http` stack renders
/// on its own onto the openEHR `{ error, message }` body
/// ([`crate::overview::error`]).
///
/// `TimeoutLayer` builds its `408` as a bare status and `RequestBodyLimitLayer`
/// its `413` as `text/plain`; neither passes through
/// [`crate::overview::error::RestError`], so without this mapping they would be
/// the only error responses in a foreign shape. `408` is a predefined status
/// (`Requests_and_responses.md` §HTTP status codes) and `413` one of the
/// additional codes the same section permits; the error body itself is a MAY
/// there, so this is consistency across our own surface.
///
/// A response that already carries a JSON body came from a handler and passes
/// through untouched.
async fn align_transport_error_body(response: Response) -> Response {
    let status = response.status();
    let message = match status {
        StatusCode::REQUEST_TIMEOUT => {
            "the request exceeded the maximum execution time and was aborted"
        }
        StatusCode::PAYLOAD_TOO_LARGE => {
            "the request body exceeds the maximum size accepted by this server"
        }
        _ => return response,
    };
    let already_rendered = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if already_rendered {
        return response;
    }
    error::status_error_response(status, message)
}

/// Mounts the public, pre-auth surface around the API tree: the always-on health
/// family, the REST-root status surface, the config-gated SMART
/// `/.well-known/smart-configuration` document (SMART master04 §Service
/// Discovery; an empty router when SMART is disabled), and the Swagger UI plus
/// `OpenAPI` documents at the access level `server.swagger_ui` names — `public`
/// sits beside the health family, `private`/`admin_only` sit behind the same
/// access guard the management endpoints use, `off` is not mounted.
///
/// No openEHR spec governs this — our own operational surface. The health family
/// ([`crate::extensions::health`]) is mounted unconditionally and ungated,
/// outside the API subtree, so an orchestrator can probe the server without
/// credentials, without any configuration enabled, and without the overload-shed
/// layer being able to shed the probe.
fn mount_public_surface(
    cfg: &crate::config::AppConfig,
    state: &AppState,
    authenticator: &Arc<Authenticator>,
    api: Router<AppState>,
    rest_root: &str,
) -> Router<AppState> {
    // A pure function of static configuration, built once inside the router.
    let discovery = smart::discovery::router(cfg, rest_root);

    let mut inner = Router::new()
        .nest(&cfg.server.base_path, api)
        .merge(health::router())
        .merge(status::router(rest_root))
        .merge(discovery);

    match cfg.server.swagger_ui {
        AccessLevel::Off => {}
        AccessLevel::Public => inner = inner.merge(openapi::swagger_router(cfg)),
        level @ (AccessLevel::Private | AccessLevel::AdminOnly) => {
            inner = inner.merge(openapi::swagger_router(cfg).layer(from_fn_with_state(
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

/// Builds the standalone management router for separate-port mode, which the
/// binary serves on the management listener when `management.port` is set.
pub fn management_router(state: &AppState, authenticator: Arc<Authenticator>) -> Router {
    management::router(ManagementState::from_app_state(
        state,
        authenticator,
        management_rbac(state),
    ))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::response::Response;
    use http::StatusCode;

    use super::{align_transport_error_body, handle_panic};

    /// a panicked handler renders the standard openEHR `{ error, message }`
    /// JSON 500 body (never tower-http's default text/plain), and the panic
    /// payload is never echoed into the response.
    #[tokio::test]
    async fn catch_panic_renders_openehr_json_body() {
        let resp = handle_panic(Box::new("secret panic detail"));
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "Internal Server Error");
        let message = body["message"].as_str().unwrap();
        assert!(
            !message.contains("secret panic detail"),
            "panic payload must not leak into the response body"
        );
    }

    async fn json_body(resp: Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        assert_eq!(
            resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// The `tower-http` `TimeoutLayer` renders `408` as a bare status with an
    /// empty body; the alignment re-renders it in the openEHR
    /// `{ error, message }` shape. `408` is a predefined status in the spec's
    /// own table (ITS-REST `Requests_and_responses.md` §HTTP status codes —
    /// "Request maximum execution time is reached, therefore the server aborted
    /// the request"), so only the body shape is ours to choose.
    #[tokio::test]
    async fn layer_produced_408_is_rendered_as_the_openehr_error_body() {
        let mut bare = Response::new(Body::empty());
        *bare.status_mut() = StatusCode::REQUEST_TIMEOUT;
        let (status, body) = json_body(align_transport_error_body(bare).await).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(body["error"], "Request Timeout");
        assert!(
            body.get("message")
                .and_then(serde_json::Value::as_str)
                .is_some()
        );
    }

    /// The `RequestBodyLimitLayer` renders `413` as `text/plain`; the alignment
    /// re-renders it in the openEHR `{ error, message }` shape. `413` is not in
    /// the spec's status table but is permitted as an additional,
    /// non-conflicting code (`Requests_and_responses.md` §HTTP status codes).
    #[tokio::test]
    async fn layer_produced_413_is_rendered_as_the_openehr_error_body() {
        let mut bare = Response::new(Body::from("length limit exceeded"));
        *bare.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
        bare.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        let (status, body) = json_body(align_transport_error_body(bare).await).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"], "Payload Too Large");
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| !m.contains("length limit exceeded")),
            "the tower-http text body must be replaced, not embedded"
        );
    }

    /// A handler-produced `408` (the AQL query-execution timeout, rendered
    /// through `RestError`) already carries the openEHR body and must pass
    /// through the alignment untouched — its message names the real cause.
    #[tokio::test]
    async fn handler_produced_408_passes_through_unchanged() {
        let original = crate::overview::error::status_error_response(
            StatusCode::REQUEST_TIMEOUT,
            "query aborted after 5000ms",
        );
        let (status, body) = json_body(align_transport_error_body(original).await).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(body["message"], "query aborted after 5000ms");
    }

    /// Every other status is returned byte-for-byte: the alignment is scoped to
    /// the two layer-produced transport errors and must never touch a success
    /// or another error path.
    #[tokio::test]
    async fn other_statuses_are_untouched() {
        let mut ok = Response::new(Body::from("hello"));
        ok.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain"),
        );
        let resp = align_transport_error_body(ok).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/plain"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), b"hello");
    }
}
