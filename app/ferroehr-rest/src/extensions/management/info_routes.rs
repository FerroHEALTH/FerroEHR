// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET /management/info` — the axum handler over the platform
//! [`BuildInfo`] model
//! (the model lives in the platform, wire here).

use axum::response::Json;

use ferroehr::telemetry::build_info::BuildInfo;

/// Render the build/spec provenance as JSON.
pub(super) fn info(build: BuildInfo) -> Json<BuildInfo> {
    Json(build)
}
