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

use openehr_its::rest::runtime::ApiError;

use crate::overview::error::RestError;

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
pub(super) fn required_query(
    parts: &crate::api::RequestParts,
    name: &str,
) -> Result<String, RestError> {
    crate::params::query_param(parts.query.as_deref(), name).ok_or_else(|| {
        RestError::from(ApiError::BadRequest(format!(
            "the `{name}` query parameter is required"
        )))
    })
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
