// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public health family at the process root: `/health`,
//! `/health/liveness`, `/health/readiness`.
//!
//! NOTE: no openEHR spec governs this — our own operational surface;
//! disposition recorded on issue #305. The vendored ITS-REST System API
//! defines exactly one operation (`OPTIONS /`, the conformance manifest,
//! `docs/specs/openehr/ITS-REST/specifications/docs/system/`) and no health
//! resource at all, so this whole family is our own extension.
//!
//! **Always mounted, never gated.** These three routes are mounted
//! unconditionally by [`crate::router::router`], outside the API subtree — so
//! outside authentication, outside the ATNA audit layer, and outside the
//! overload-shed layer. An orchestrator can therefore always probe the
//! server: no credentials, no configuration to remember, and a saturated
//! server cannot shed its own probes.
//!
//! The three contracts are deliberately different, one per client:
//!
//! | Route | Contract | Canonical client |
//! |---|---|---|
//! | `GET /health` | constant `OK`, no I/O | load balancers, container `HEALTHCHECK` |
//! | `GET /health/liveness` | the same constant `OK` (path alias) | container/orchestrator liveness probe |
//! | `GET /health/readiness` | the family readiness document, evaluated per call; `200` while ready, `503` while booting, draining or an indicator is `down`; the body names the phase and each indicator's state, nothing more | orchestrator readiness probe |
//!
//! **The probes say as little as possible.** An orchestrator acts on the
//! status code alone, and the body is reachable by anyone who can reach the
//! port, so readiness serves [`Readiness::without_detail`]: no detail string, count,
//! chain position, host or version. The same evaluation with every
//! indicator's detail is `GET /management/health`
//! ([`crate::extensions::management`]), behind the management access gate.
//! `GET /ferroehr/rest/status` ([`crate::overview::status`]) is a different
//! contract — the product status document.

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::extensions::management::readiness_status;
use crate::state::AppState;
use ferrohealth_health::document::Readiness;

/// The constant liveness body. Reaching a liveness handler at all is the whole
/// check — the process is running and the router is answering — so the body is
/// a fixed `OK` with no I/O behind it.
const LIVENESS_BODY: &str = "OK";

/// The always-on public health routes. Merged into the pre-auth surface by
/// [`crate::router::router`]; there is no configuration switch, by design.
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(root_health))
        .route("/health/liveness", get(liveness))
        .route("/health/readiness", get(readiness))
}

/// The health family's `OpenAPI` document. Unauthenticated — these operations
/// carry no security requirement in the served document.
pub(crate) fn openapi() -> utoipa::openapi::OpenApi {
    OpenApiRouter::<AppState>::new()
        .routes(routes!(root_health))
        .routes(routes!(liveness))
        .routes(routes!(readiness))
        .into_openapi()
}

/// Process liveness (`GET /health`).
///
/// OUR OWN SURFACE — no openEHR spec governs a health endpoint. Returns the
/// plain-text body `OK`. Always mounted, unauthenticated (outside the auth and
/// overload-shed layers).
#[utoipa::path(
    get, path = "/health", tag = "status",
    responses(
        (status = 200, description = "Server process alive; plain-text `OK`.",
         body = String)
    )
)]
async fn root_health() -> Response {
    (StatusCode::OK, LIVENESS_BODY).into_response()
}

/// Process liveness under the orchestrator-conventional path
/// (`GET /health/liveness`) — a byte-identical alias of [`root_health`].
///
/// OUR OWN SURFACE — no openEHR spec governs a health endpoint. Always
/// mounted, unauthenticated, and never touches a dependency: a `200` here says
/// the process is up, nothing more (that is what a liveness probe must mean —
/// a DB outage must not get the container killed).
#[utoipa::path(
    get, path = "/health/liveness", tag = "status",
    responses(
        (status = 200, description = "Server process alive; plain-text `OK`.",
         body = String)
    )
)]
async fn liveness() -> Response {
    (StatusCode::OK, LIVENESS_BODY).into_response()
}

/// Readiness (`GET /health/readiness`) — the family readiness document,
/// evaluated on every call and served without detail.
///
/// OUR OWN SURFACE — no openEHR spec governs a health endpoint. Always
/// mounted, unauthenticated. The body is the state, the process phase and
/// each indicator's name and state (DB ping, migrations applied, and whatever
/// else the binary registered), never an indicator's detail; the status is
/// `200` while ready and `503` while the process boots or drains or an
/// indicator is `down`, so an orchestrator takes the instance out of rotation
/// without killing it. The detail is on the authenticated
/// `GET /management/health`.
#[utoipa::path(
    get, path = "/health/readiness", tag = "status",
    responses(
        (status = 200, description = "Ready to serve (state `up` or `degraded`); \
                                      `{state, phase, indicators: [{name, state}]}`, \
                                      no detail.",
         body = serde_json::Value),
        (status = 503, description = "Not ready (booting, draining, or an indicator is \
                                      `down`); `{state, phase, indicators: [{name, state}]}`, \
                                      no detail.",
         body = serde_json::Value)
    )
)]
async fn readiness(State(state): State<AppState>) -> Response {
    let observability = state.observability();
    let document: Readiness = observability
        .health
        .readiness(&observability.lifecycle)
        .await;
    (readiness_status(&document), Json(document.without_detail())).into_response()
}
