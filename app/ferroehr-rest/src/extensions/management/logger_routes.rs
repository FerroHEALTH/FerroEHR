// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Axum handlers for the logger-reload surface (model:
//! [`ferroehr::telemetry::log_reload::LogReload`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use ferroehr::telemetry::log_reload::{FilterReloadError, LogReload};

#[derive(Debug, Serialize)]
pub(super) struct LoggersView {
    /// The effective filter directives.
    filter: String,
    /// The boot-time filter directives (the reset target).
    boot_filter: String,
}

/// The `POST` request body.
#[derive(Debug, Deserialize)]
pub(super) struct SetFilter {
    /// The new filter directive set (`EnvFilter` syntax).
    filter: String,
}

fn view(reload: &LogReload) -> LoggersView {
    LoggersView {
        filter: reload.current(),
        boot_filter: reload.boot_filter().to_owned(),
    }
}

/// `GET /management/loggers`.
pub(super) fn get(reload: &LogReload) -> Json<LoggersView> {
    Json(view(reload))
}

/// `POST /management/loggers`.
pub(super) fn set(reload: &LogReload, body: &SetFilter) -> Response {
    match reload.set(&body.filter) {
        Ok(()) => (StatusCode::OK, Json(view(reload))).into_response(),
        Err(e) => (
            refusal_status(&e),
            Json(serde_json::json!({ "error": refusal_message("invalid filter", &e) })),
        )
            .into_response(),
    }
}

/// `DELETE /management/loggers`.
pub(super) fn reset(reload: &LogReload) -> Response {
    match reload.reset() {
        Ok(()) => (StatusCode::OK, Json(view(reload))).into_response(),
        Err(e) => (
            refusal_status(&e),
            Json(serde_json::json!({ "error": refusal_message("reset failed", &e) })),
        )
            .into_response(),
    }
}

/// Renders a refused filter swap for the response body: the refusal and the
/// failure that caused it.
///
/// One hop rather than the whole chain, because `tracing_subscriber`'s
/// directive `ParseError` already repeats its own source in its `Display`.
fn refusal_message(prefix: &str, e: &FilterReloadError) -> String {
    match std::error::Error::source(e) {
        Some(cause) => format!("{prefix}: {e}: {cause}"),
        None => format!("{prefix}: {e}"),
    }
}

/// The two arms of a refused filter swap are two different faults: directives
/// the caller wrote wrong are a `400`, a reload layer that is gone is a `503`.
fn refusal_status(e: &FilterReloadError) -> StatusCode {
    match e {
        FilterReloadError::Directives(_) => StatusCode::BAD_REQUEST,
        FilterReloadError::Handle(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `400` body names the parser's diagnosis, which the refusal carries
    /// as its source rather than in its own message.
    #[test]
    fn a_refused_filter_body_names_the_parse_diagnosis() {
        let parse = tracing_subscriber::EnvFilter::try_new("ferroehr=!!bad")
            .expect_err("the directive must not parse");
        let diagnosis = parse.to_string();
        let refused = FilterReloadError::from(parse);
        assert_eq!(
            refusal_message("invalid filter", &refused),
            format!("invalid filter: the filter directives are not valid: {diagnosis}")
        );
        assert_eq!(refusal_status(&refused), StatusCode::BAD_REQUEST);
    }
}
