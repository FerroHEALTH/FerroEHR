// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET /management/info` — the axum handler over the platform
//! [`BuildInfo`] model
//! (the model lives in the platform, wire here).

use axum::response::Json;

use ferroehr::party_binding::grants::live::LiveGrants;
use ferroehr::telemetry::build_info::BuildInfo;

/// Render the build/spec provenance as JSON, its binding listing the
/// federation agreements `grants` hold in force now.
///
/// A copy the party store cannot refresh lists the agreements it holds, and
/// logs the failure.
pub(super) async fn info(mut build: BuildInfo, grants: &LiveGrants) -> Json<BuildInfo> {
    if let Some(binding) = build.binding.as_mut() {
        if let Err(error) = grants.refresh_if_stale().await {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the policy grants could not be read for /management/info"
            );
        }
        binding.agreements = grants.agreements(jiff::Timestamp::now());
    }
    Json(build)
}
