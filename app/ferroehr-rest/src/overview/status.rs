// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The status document in its public and full forms.
//!
//! The public `/rest/status` summary is mounted outside the authentication
//! layer; the full document is served by the management surface behind its
//! access gate (`/management/status`).
//!
//! NOTE: no openEHR spec governs an operational status or health endpoint —
//! our own operational surface. Health is a separate contract with its own
//! clients and lives entirely in the process-root family (`/health`,
//! `/health/liveness`, `/health/readiness` — [`crate::extensions::health`]);
//! this module serves only the status document.
//!
//! The public form says what a client needs before it holds a credential and
//! nothing more: that the server answers, its version (the System Options
//! manifest already publishes it as `solution_version`), the ITS-REST release
//! it implements, and the deployment profile, so a `sandbox` is never mistaken
//! for a `production` (#3226). The licence, the open and accepted deployment
//! gaps and the support period name an installation's weak points and its
//! owner, so they are served only by [`FullStatus`] behind authentication.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use ferroehr::config::deployment::{DeploymentPosture, DeploymentProfile};
use ferroehr::licence::state::LicenceStatus;
use ferroehr::support::{SupportPeriod, SupportReport, today_utc};
use serde::Serialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;
use ferroehr::telemetry::provenance;

/// The public `/rest/status` body: the server answers, at which version, and
/// under which deployment profile.
#[derive(Debug, Serialize)]
struct PublicStatus {
    status: &'static str,
    server_version: &'static str,
    openehr_rest_api_version: &'static str,
    timestamp: String,
    deployment: PublicDeployment,
}

/// The one deployment fact the public document carries: the declared profile.
#[derive(Debug, Serialize)]
struct PublicDeployment {
    profile: DeploymentProfile,
}

/// The full status document `GET /management/status` serves.
///
/// The public fields plus the licence in force, the complete deployment
/// posture and the support period. `openehr_rest_api_version` is the single
/// shared provenance identity ([`provenance::ITS_REST`]), the released
/// ITS-REST contract version that management `/info` and the System Options
/// manifest also report. `licence` is the summary of the `[licence]` boot
/// outcome ([`LicenceStatus`]): its state, use class, licensee and last valid
/// day, never a refusal reason.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FullStatus {
    status: &'static str,
    server_version: &'static str,
    openehr_rest_api_version: &'static str,
    timestamp: String,
    licence: LicenceStatus,
    /// The declared deployment posture (#3226): the profile, the separations
    /// it has not made and the ones it accepted by name.
    deployment: DeploymentPosture,
    /// The support period of this release and where it stands today
    /// (`docs/law/eu/cra/text.html Art. 13(19)`): `status` is `ended` once the
    /// release is out of support.
    support: SupportReport,
}

impl FullStatus {
    /// The full document as of now, over the licence and posture fixed at
    /// boot.
    pub(crate) fn now(licence: LicenceStatus, deployment: DeploymentPosture) -> Self {
        Self {
            status: "UP",
            server_version: env!("CARGO_PKG_VERSION"),
            openehr_rest_api_version: provenance::ITS_REST,
            timestamp: jiff::Timestamp::now().to_string(),
            licence,
            deployment,
            support: SupportPeriod::current().report_on(today_utc()),
        }
    }
}

/// Server status (`GET /ferroehr/rest/status`).
///
/// OUR OWN SURFACE — no openEHR spec governs an operational status endpoint.
/// Reports that the server answers, its version, the tested ITS-REST contract
/// identity and the declared deployment profile. Unauthenticated (mounted
/// outside the auth layer); the licence, the deployment gaps and the support
/// period are served only by the authenticated `GET /management/status`. The
/// path above is the DEFAULT deployment spelling: a non-default
/// `server.base_path` moves the live mount ([`router`]) and the served
/// document follows it ([`openapi`]).
#[utoipa::path(
    get, path = "/ferroehr/rest/status", tag = "status",
    responses(
        (status = 200, description = "Server up; a JSON `{status, server_version, \
                                      openehr_rest_api_version, timestamp, \
                                      deployment: {profile}}` object. The licence, \
                                      the deployment gaps and the support period \
                                      are on the authenticated `/management/status`.",
         body = serde_json::Value)
    )
)]
async fn status(State(state): State<AppState>) -> Json<PublicStatus> {
    Json(PublicStatus {
        status: "UP",
        server_version: env!("CARGO_PKG_VERSION"),
        openehr_rest_api_version: provenance::ITS_REST,
        timestamp: jiff::Timestamp::now().to_string(),
        deployment: PublicDeployment {
            profile: state.backend().deployment().profile,
        },
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
