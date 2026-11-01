// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The members the admin register routes read from a request, and the
//! refusals every one of them answers.
//!
//! No openEHR spec governs these extension routes: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;

/// A required string member of the request body.
///
/// A missing or non-string member is a `400`: guessing a shape here would
/// record a legal mark against something the caller did not name.
pub(super) fn required_str<'a>(
    body: &'a serde_json::Value,
    field: &str,
) -> Result<&'a str, RestError> {
    body.get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RestError::from(ApiError::BadRequest(format!(
                "request body must carry a `{field}` string"
            )))
        })
}

/// An optional string member of the request body; a present non-string is a
/// `400` rather than a silent absence.
pub(super) fn optional_str<'a>(
    body: &'a serde_json::Value,
    field: &str,
) -> Result<Option<&'a str>, RestError> {
    match body.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.as_str())),
        Some(other) => Err(RestError::from(ApiError::BadRequest(format!(
            "`{field}` must be a string when present, got {other}"
        )))),
    }
}

/// A required query parameter of the request; a missing one is a `400`.
pub(super) fn required_query(parts: &RequestParts, name: &str) -> Result<String, RestError> {
    crate::params::query_param(parts.query.as_deref(), name).ok_or_else(|| {
        RestError::from(ApiError::BadRequest(format!(
            "the `{name}` query parameter is required"
        )))
    })
}

/// Run `op` of an admin register route through the uniform per-request stack.
pub(super) async fn handle(
    state: AppState,
    request: Request,
    op: &'static str,
    dispatch: fn(AppState, &'static str, RequestParts) -> BoxResponse,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, op, parts, dispatch).await
}

/// The operations of one group of admin register routes.
pub(super) trait RegisterRoutes {
    /// Answer `op`, behind the admin group gate.
    fn run(
        state: &AppState,
        op: &str,
        parts: &RequestParts,
    ) -> impl Future<Output = Result<Response, RestError>> + Send;
}

/// The dispatcher of the register routes `R`: the admin group gate first,
/// then `R::run`, whose error becomes its response.
pub(super) fn dispatch<R: RegisterRoutes>(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> BoxResponse {
    Box::pin(async move {
        if let Some(refusal) = super::dispatch::admin_group_gate(&state) {
            return refusal;
        }
        R::run(&state, op, &parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

/// The answers of a register read beside [`AdminRefusal`], as one response
/// map for the OpenAPI document.
#[derive(utoipa::IntoResponses)]
#[expect(
    dead_code,
    reason = "a response map the OpenAPI document reads; the answers themselves are built \
              by the register routes"
)]
pub(super) enum RegisterReadRefusal {
    #[response(status = 400, description = "`ehr_id` is missing or not a UUID.")]
    BadEhrId(#[to_schema] serde_json::Value),
    #[response(status = 404, description = "No such EHR.")]
    UnknownEhr(#[to_schema] serde_json::Value),
}

/// The refusals every admin register route answers before it reads its
/// request, as one response map for the OpenAPI document.
#[derive(utoipa::IntoResponses)]
#[expect(
    dead_code,
    reason = "a response map the OpenAPI document reads; the refusals themselves are built \
              by the authentication layer and the admin gate"
)]
pub(super) enum AdminRefusal {
    #[response(status = 401, description = "Unauthenticated.")]
    Unauthenticated(#[to_schema] serde_json::Value),
    #[response(status = 403, description = "Not in the Admin class.")]
    NotAdmin(#[to_schema] serde_json::Value),
    #[response(
        status = 405,
        description = "The admin API is disabled on this server."
    )]
    Disabled(#[to_schema] serde_json::Value),
}
