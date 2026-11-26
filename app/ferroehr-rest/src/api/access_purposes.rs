// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The purposes of use a caller may declare (#6105), **our own extension**.
//!
//! `GET {extensions}/access/purposes` answers the `[audit] purpose_header`, the
//! `[audit] reason_header`, the `[audit] purpose_system` and each
//! `[audit] purpose_codes` entry with the
//! display text the terminology server gives it, so a client such as the
//! viewer offers the codes an access policy that requires a purpose accepts.
//! The list names no personal data, so any authenticated caller reads it, a
//! bound person and a roleless principal included
//! ([`crate::extensions::access::authz`]). No openEHR ITS-REST operation
//! governs it.

use axum::response::Response;
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The purposes route, mounted on the extension surface.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(access_purposes))
}

guarded_handler! {
    /// The purposes of use a caller may declare (`GET /access/purposes`).
    ///
    /// **Our own extension, no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get,
        path = "/access/purposes",
        tag = "audit",
        responses(
            (status = 200, description = "The header a purpose is declared in, the header \
                                          the reason of a justified access is stated in, \
                                          the code system of the codes, and the accepted codes \
                                          in configured order, each with its display \
                                          text when the terminology server gives one. \
                                          An empty `codes` accepts any code.",
             body = serde_json::Value),
            (status = 401, description = "No valid credential.", body = serde_json::Value),
            (status = 404, description = "Access logging is off, so no purpose is \
                                          recorded.",
             body = serde_json::Value)
        )
    )]
    access_purposes => |state, parts| async move { run(&state, &parts).await }
}

/// The answer: the header, the code system and the accepted codes.
#[derive(Debug, serde::Serialize)]
struct Listed<'a> {
    /// The request header a purpose is declared in.
    header: &'a str,
    /// The request header the reason of a justified access is stated in.
    reason_header: &'a str,
    /// The canonical URL of the code system.
    system: &'a str,
    /// The accepted codes in configured order.
    codes: Vec<Code<'a>>,
}

/// One accepted code, with its display text when the terminology server
/// gives one.
#[derive(Debug, serde::Serialize)]
struct Code<'a> {
    /// The code a caller declares.
    code: &'a str,
    /// Its display text.
    #[serde(skip_serializing_if = "Option::is_none")]
    display: Option<&'a str>,
}

async fn run(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    let Some(purposes) = state.backend().audit_purposes().await else {
        return Err(RestError::from(ApiError::NotFound(
            "access logging is off: no purpose of use is recorded".to_owned(),
        )));
    };
    let listed = Listed {
        header: &purposes.header,
        reason_header: &purposes.reason_header,
        system: &purposes.system,
        codes: purposes
            .codes
            .iter()
            .map(|purpose| Code {
                code: &purpose.code,
                display: purpose.display.as_deref(),
            })
            .collect(),
    };
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &listed,
    ))
}
