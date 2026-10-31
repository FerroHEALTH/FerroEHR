// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The members the admin register routes read from a JSON request body.
//!
//! No openEHR spec governs these extension bodies: our own design.

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
