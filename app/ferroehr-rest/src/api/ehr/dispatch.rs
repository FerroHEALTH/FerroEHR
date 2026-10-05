// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the `ehr` API group — the operation-id → resource-module
//! router.
//!
//! The 33 EHR-group operations are implemented one module per spec resource
//! boundary (`docs/specs/openehr/ITS-REST/specifications/docs/ehr/`,
//! `specifications/operations/*.yaml`); this module only maps each generated
//! operation id to its owning resource module's `run`. The shared write / read
//! / committal / item-tag helpers live in the group root
//! ([`super`](crate::api::ehr)); the arm bodies live in the resource modules.

use axum::response::{IntoResponse, Response};

use crate::api::{BoxResponse, RequestParts};
use crate::overview::error::RestError;
use crate::state::AppState;

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        Box::pin(run(state, op, parts))
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

/// Route a generated operation id to its owning resource module (the spec's own
/// resource boundaries: EHR / `EHR_STATUS` / `VERSIONED_EHR_STATUS` / COMPOSITION /
/// `VERSIONED_COMPOSITION` / DIRECTORY / CONTRIBUTION).
async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    match op {
        // ── EHR (+ EHR-level item tags) ──────────────────────────────────────
        "ehr_get_by_subject" | "ehr_create" | "ehr_create_with_id" | "ehr_get_by_id"
        | "ehr_tags_get" => Box::pin(super::ehr_resource::run(state, op, parts)).await,
        // ── EHR_STATUS (+ its item tags) ─────────────────────────────────────
        "ehr_status_get_by_version_id"
        | "ehr_status_get_at_time"
        | "ehr_status_update"
        | "ehr_status_tags_get"
        | "ehr_status_tags_update"
        | "ehr_status_tags_delete" => Box::pin(super::ehr_status::run(state, op, parts)).await,
        // ── VERSIONED_EHR_STATUS ─────────────────────────────────────────────
        "versioned_ehr_status_get"
        | "versioned_ehr_status_revision_history"
        | "versioned_ehr_status_version_get_at_time"
        | "versioned_ehr_status_version_get_by_id" => {
            super::versioned_ehr_status::run(state, op, parts).await
        }
        // ── COMPOSITION (+ its item tags) ────────────────────────────────────
        "composition_create"
        | "composition_get"
        | "composition_update"
        | "composition_delete"
        | "composition_tags_get"
        | "composition_tags_update"
        | "composition_tags_delete" => Box::pin(super::composition::run(state, op, parts)).await,
        // ── VERSIONED_COMPOSITION ────────────────────────────────────────────
        "versioned_composition_get"
        | "versioned_composition_revision_history"
        | "versioned_composition_version_get_at_time"
        | "versioned_composition_version_get_by_id" => {
            super::versioned_composition::run(state, op, parts).await
        }
        // ── DIRECTORY (FOLDER) ───────────────────────────────────────────────
        "directory_get_at_time"
        | "directory_update"
        | "directory_create"
        | "directory_delete"
        | "directory_get_by_version_id" => Box::pin(super::directory::run(state, op, parts)).await,
        // ── CONTRIBUTION (+ the paged-list extension) ────────────────────────
        "contribution_create" | "contribution_get" | "contribution_list" => {
            super::contribution::run(state, op, parts).await
        }
        other => Err(RestError(openehr_its::rest::runtime::ApiError::Internal(
            format!("unrouted ehr operation: {other}"),
        ))),
    }
}
