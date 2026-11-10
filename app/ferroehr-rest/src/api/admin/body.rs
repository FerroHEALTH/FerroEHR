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

use axum::response::Response;
use openehr_its::rest::runtime::ApiError;

use crate::api::RequestParts;
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

/// One admin register operation behind the admin group gate.
///
/// # Errors
/// The refusals of `run`.
pub(super) async fn gated<F>(
    state: AppState,
    parts: RequestParts,
    run: fn(AppState, RequestParts) -> F,
) -> Result<Response, RestError>
where
    F: Future<Output = Result<Response, RestError>>,
{
    if let Some(refusal) = super::operations::admin_group_gate(&state) {
        return Ok(refusal);
    }
    run(state, parts).await
}

/// The answers of a register read beside [`AdminRefusal`], as one response
/// map for the OpenAPI document; the register routes build the answers.
pub(super) struct RegisterReadRefusal;

impl utoipa::IntoResponses for RegisterReadRefusal {
    fn responses() -> ResponseMap {
        json_responses(&[
            (400, "`ehr_id` is missing or not a UUID."),
            (404, "No such EHR."),
        ])
    }
}

/// The refusals every admin register route answers before it reads its
/// request, as one response map for the OpenAPI document; the authentication
/// layer and the admin gate build the refusals.
pub(super) struct AdminRefusal;

impl utoipa::IntoResponses for AdminRefusal {
    fn responses() -> ResponseMap {
        json_responses(&[
            (401, "Unauthenticated."),
            (403, "Not in the Admin class."),
            (405, "The admin API is disabled on this server."),
        ])
    }
}

/// The responses of an [`utoipa::IntoResponses`] map, by status.
type ResponseMap =
    std::collections::BTreeMap<String, utoipa::openapi::RefOr<utoipa::openapi::response::Response>>;

/// One `application/json` response per `(status, description)`, whose body is
/// any JSON value.
fn json_responses(answers: &[(u16, &str)]) -> ResponseMap {
    answers
        .iter()
        .map(|&(status, description)| {
            let content = utoipa::openapi::ContentBuilder::new()
                .schema(Some(<serde_json::Value as utoipa::PartialSchema>::schema()))
                .build();
            let response = utoipa::openapi::ResponseBuilder::new()
                .description(description)
                .content("application/json", content)
                .build();
            (status.to_string(), response.into())
        })
        .collect()
}

/// An admin route's unauthenticated refusal, with the released operations' `security: []` named.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 401,
    description = "Unauthenticated (auth enabled, no valid \
                  principal). Our own authorization design \
                  — the released admin operations carry \
                  `security: []` and declare no such \
                  branch."
)]
pub(super) struct AdminUnauthenticated;

/// An admin route's refusal of a caller outside the Admin class.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 403,
    description = "Authenticated but not in the Admin class \
                  (`OperationClass::Admin`, keyed off the \
                  `/admin/` path). Our own authorization \
                  design."
)]
pub(super) struct NotAdminClass;

/// An admin route's refusal while the admin API is disabled.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 405,
    description = "The admin API is disabled on this server \
                  (`AppConfig::api.admin`, default \
                  false), answered with an empty `Allow` \
                  per RFC 9110 §10.2.1."
)]
pub(super) struct AdminDisabled;
