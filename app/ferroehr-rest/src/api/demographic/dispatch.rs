// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic operation-id → handler match (the group dispatcher).
//!
//! `openehr_its::rest::generated::demographic::ROUTES` (from the vendored
//! `demographic.openapi.yaml`, via the `mount` adapter) and the native
//! `utoipa-axum` `PARTY_RELATIONSHIP` extension router
//! ([`relationship_routes`](super::relationship::relationship_routes)) are both routed onto
//! this one dispatcher. It classifies the
//! operation id and forwards to the resource module: [`party`](super::party),
//! [`tags`](super::tags), [`versioned_party`](super::versioned_party),
//! [`contribution`](super::contribution), or [`relationship`](super::relationship).

use axum::response::{IntoResponse, Response};

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts};
use crate::overview::error::RestError;
use crate::state::AppState;

/// The group dispatcher: box the response future and forward to [`run`].
pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        Box::pin(run(state, op, parts))
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // Our-own-design PARTY_RELATIONSHIP extension routes (no ITS-REST contract):
    // matched before the per-kind party ops (which never share this prefix).
    if op.starts_with("party_relationship") || op.starts_with("versioned_party_relationship") {
        return Box::pin(super::relationship::run(state, op, parts)).await;
    }
    if let Some((kind, action)) = super::parse_party_op(op) {
        return match action {
            "create" | "get" | "update" | "delete" => {
                Box::pin(super::party::run(state, kind, action, parts)).await
            }
            "tags_get" | "tags_update" | "tags_delete" => {
                super::tags::run(state, kind, action, parts).await
            }
            other => Err(RestError(ApiError::Internal(format!(
                "unrouted demographic party operation: {}_{other}",
                kind.segment()
            )))),
        };
    }
    // Kind-agnostic operations.
    match op {
        op if op.starts_with("versioned_party") => {
            super::versioned_party::run(state, op, parts).await
        }
        "contribution_create" | "contribution_get" => {
            super::contribution::run(state, op, parts).await
        }
        "demographic_tags_get" => super::tags::run_collection(state, parts).await,
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted demographic operation: {other}"
        )))),
    }
}
