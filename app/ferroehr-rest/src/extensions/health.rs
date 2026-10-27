// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The served documentation of the family health routes: `/health`,
//! `/health/liveness`, `/health/readiness`.
//!
//! NOTE: no openEHR spec governs this — our own operational surface;
//! disposition recorded on issue #305. The vendored ITS-REST System API
//! defines exactly one operation (`OPTIONS /`, the conformance manifest,
//! `docs/specs/openehr/ITS-REST/specifications/docs/system/`) and no health
//! resource at all, so this whole family is our own extension.
//!
//! The routes themselves are the family stack's
//! (`ferrohealth_server::health::routes`, mounted by `stack::app` under
//! `server.base_path`, outside authentication, the ATNA audit layer and the
//! overload bound), so an orchestrator can always probe the server. The stubs
//! below are their documentation twins, the same pattern as the System
//! `OPTIONS` manifest's: registered only to build the document, never served.
//! The detail behind readiness is `GET /management/health`
//! ([`crate::extensions::management`]), behind the management access gate.

use ferrohealth_server::section::BasePath;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// The health family's `OpenAPI` document under `base_path`. Unauthenticated —
/// these operations carry no security requirement in the served document.
pub(crate) fn openapi(base_path: &BasePath) -> utoipa::openapi::OpenApi {
    let mut doc = OpenApiRouter::<AppState>::new()
        .routes(routes!(root_health))
        .routes(routes!(liveness))
        .routes(routes!(readiness))
        .into_openapi();
    for route in [
        ferrohealth_server::health::SUMMARY,
        ferrohealth_server::health::LIVENESS,
        ferrohealth_server::health::READINESS,
    ] {
        crate::extensions::openapi::rehome_path(&mut doc, route, &base_path.join(route));
    }
    doc
}

/// The readiness summary (`GET /health`).
///
/// OUR OWN SURFACE — no openEHR spec governs a health endpoint. Answers the
/// status readiness answers with the state alone (`{"state": …}`), for a
/// caller that may not see the indicators. Always mounted, unauthenticated
/// (outside the auth layer and the overload bound).
#[utoipa::path(
    get, path = "/health", tag = "status",
    responses(
        (status = 200, description = "Ready to serve; `{state}`.",
         body = serde_json::Value),
        (status = 503, description = "Not ready (booting, draining, or an indicator is \
                                      `down`); `{state}`.",
         body = serde_json::Value)
    )
)]
async fn root_health() {}

/// Process liveness (`GET /health/liveness`).
///
/// OUR OWN SURFACE — no openEHR spec governs a health endpoint. Always
/// mounted, unauthenticated, and never touches a dependency: a `200` here says
/// the process is up, nothing more (that is what a liveness probe must mean —
/// a DB outage must not get the container killed).
#[utoipa::path(
    get, path = "/health/liveness", tag = "status",
    responses(
        (status = 200, description = "Server process alive; `{\"state\": \"up\"}`.",
         body = serde_json::Value)
    )
)]
async fn liveness() {}

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
async fn readiness() {}
