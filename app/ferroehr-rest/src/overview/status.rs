// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public status surface hanging off the REST root: the `/rest/status`
//! product status document, mounted outside the authentication layer.
//!
//! NOTE: no openEHR spec governs an operational status or health endpoint —
//! our own operational surface. Health is a separate contract with its own
//! clients and lives entirely in the process-root family (`/health`,
//! `/health/liveness`, `/health/readiness` — [`crate::extensions::health`]);
//! this module serves only the status document.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use ferroehr::licence::state::LicenceStatus;
use serde::Serialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;
use ferroehr::telemetry::provenance;

/// `/rest/status` body — server and conformance-target versions, plus the
/// licence in force. The `openehr_rest_api_version` is the single shared
/// provenance identity ([`provenance::ITS_REST`]) — the released ITS-REST
/// contract version that management `/info` and the System Options manifest
/// also report. `licence` is the public summary of the `[licence]` boot
/// outcome ([`LicenceStatus`]): its state, use class, licensee and last valid
/// day, never a refusal reason.
#[derive(Debug, Serialize)]
struct ServerStatus {
    status: &'static str,
    server_version: &'static str,
    openehr_rest_api_version: &'static str,
    timestamp: String,
    licence: LicenceStatus,
    /// The declared deployment posture (#3226): the profile, the separations
    /// it has not made and the ones it accepted by name, so a `sandbox` cannot
    /// be mistaken for a `production` from its own API.
    deployment: ferroehr::config::deployment::DeploymentPosture,
}

/// Server status (`GET /ferroehr/rest/status`).
///
/// OUR OWN SURFACE — no openEHR spec governs an operational status endpoint.
/// Reports the server version, the tested ITS-REST contract identity and the
/// licence in force. Unauthenticated (mounted outside the auth layer). The
/// path above is the DEFAULT deployment spelling: a non-default
/// `server.base_path` moves the live mount ([`router`]) and the served
/// document follows it ([`openapi`]).
#[utoipa::path(
    get, path = "/ferroehr/rest/status", tag = "status",
    responses(
        (status = 200, description = "Server up; a JSON `{status, server_version, \
                                      openehr_rest_api_version, timestamp, \
                                      licence: {state, use?, licensee?, not_after?, \
                                      configured_token}, deployment: {profile, gaps, \
                                      accepted}}` \
                                      object.",
         body = serde_json::Value)
    )
)]
async fn status(State(state): State<AppState>) -> Json<ServerStatus> {
    Json(ServerStatus {
        status: "UP",
        server_version: env!("CARGO_PKG_VERSION"),
        openehr_rest_api_version: provenance::ITS_REST,
        timestamp: jiff::Timestamp::now().to_string(),
        licence: state.backend().licence().status(),
        deployment: state.backend().deployment().clone(),
    })
}

pub(crate) fn router(rest_root: &str) -> Router<AppState> {
    Router::new().route(&format!("{rest_root}/status"), get(status))
}

/// The public status surface's `OpenAPI` document, its path derived from the
/// SAME `rest_root` the live [`router`] mounts under — a non-default
/// `server.base_path` moves the served path, and the published document must
/// follow (the `#[utoipa::path]` literal is only the default-root spelling).
/// The operation is unauthenticated (mounted outside the auth layer). No
/// openEHR spec governs an operational status endpoint — our own surface.
pub(crate) fn openapi(rest_root: &str) -> utoipa::openapi::OpenApi {
    let mut doc = OpenApiRouter::<AppState>::new()
        .routes(routes!(status))
        .into_openapi();
    crate::extensions::openapi::rehome_path(
        &mut doc,
        "/ferroehr/rest/status",
        &format!("{rest_root}/status"),
    );
    doc
}
