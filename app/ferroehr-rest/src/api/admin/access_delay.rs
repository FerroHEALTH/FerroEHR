// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access delay** wire: the EHDS Art. 3(3) delay of the person's
//! own access, **our own extension**.
//!
//! Regulation (EU) 2025/327 Art. 3(3) lets a Member State delay a person's
//! access to their data "for a limited period of time until a health
//! professional is able to properly communicate and explain" it
//! (`docs/law/eu/ehds/text.html`, recital 10). A deployment that enables
//! `[access_delay]` holds new compositions of the configured categories at
//! creation; these routes let a health professional hold a composition,
//! release it early, and read the register. No openEHR ITS-REST operation
//! governs any of it.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::admin.enabled` config gate
//! (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, optional_str, required_query, required_str};
use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The access-delay extension routes — **no ITS-REST contract** (module docs).
pub(crate) fn access_delay_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_delay_access, admin_access_delay_register))
        .routes(routes!(admin_release_access_delay))
}

/// Hold one composition from the person's own access
/// (`POST /admin/access-delay`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post, path = "/admin/access-delay", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_id\": …, \"vo_id\": …, \"until\": …?, \
                                \"note\": …? }`. `until` is an RFC 3339 instant \
                                no later than `access_delay.hours` from now, the \
                                limit by default (EHDS Art. 3(3)).",
                 example = json!({
                     "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                     "vo_id": "8849182c-82ad-4088-a07f-48ead4180515",
                     "note": "result to be discussed at the visit"
                 })),
    responses(
        (status = 204, description = "The hold is recorded. A patient-context \
                                      read, query or export treats the \
                                      composition as absent until a release or \
                                      `until`; a professional reads it as \
                                      before."),
        (status = 400, description = "The body is not the object above, \
                                      `access_delay` is not enabled, `until` is \
                                      not in the future or past the limit, or \
                                      the object is not a COMPOSITION.",
         body = serde_json::Value),
        (status = 404, description = "No such EHR, or no such object in it; \
                                      nothing is held.",
         body = serde_json::Value),
        (status = 415, description = "The body is not `application/json`.",
         body = serde_json::Value),
        AdminRefusal
    )
)]
pub(crate) async fn admin_delay_access(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    handle(state, request, "admin_delay_access").await
}

/// Read one EHR's access-delay register (`GET /admin/access-delay`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    get, path = "/admin/access-delay", tag = "admin-marks",
    params(("ehr_id" = String, Query, description = "The EHR whose holds to read.")),
    responses(
        (status = 200, description = "The register rows, newest hold first, each \
                                      with its `basis` (`category` or \
                                      `professional`), `withheld_until` and \
                                      `released_at`.",
         body = serde_json::Value),
        (status = 400, description = "`ehr_id` is missing or not a UUID.",
         body = serde_json::Value),
        (status = 404, description = "No such EHR.", body = serde_json::Value),
        AdminRefusal
    )
)]
pub(crate) async fn admin_access_delay_register(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    handle(state, request, "admin_access_delay_register").await
}

/// Release a held composition to the person
/// (`POST /admin/access-delay/release`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post, path = "/admin/access-delay/release", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_id\": …, \"vo_id\": … }`, the composition \
                                a health professional has explained to the \
                                person.",
                 example = json!({
                     "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                     "vo_id": "8849182c-82ad-4088-a07f-48ead4180515"
                 })),
    responses(
        (status = 204, description = "Every hold still in force carries a \
                                      release instant, and the person reads the \
                                      composition. Releasing nothing succeeds."),
        (status = 400, description = "The body does not name both ids.",
         body = serde_json::Value),
        (status = 404, description = "No such EHR, or no such object in it.",
         body = serde_json::Value),
        (status = 415, description = "The body is not `application/json`.",
         body = serde_json::Value),
        AdminRefusal
    )
)]
pub(crate) async fn admin_release_access_delay(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    handle(state, request, "admin_release_access_delay").await
}

/// Run `op` through the uniform per-request stack.
async fn handle(state: AppState, request: Request, op: &'static str) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, op, parts, dispatch).await
}

fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        match run(&state, op, &parts).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        }
    })
}

async fn run(state: &AppState, op: &str, parts: &RequestParts) -> Result<Response, RestError> {
    if let Some(refusal) = super::dispatch::admin_group_gate(state) {
        return Ok(refusal);
    }
    let backend = state.backend();
    if op == "admin_access_delay_register" {
        let ehr_id = required_query(parts, "ehr_id")?;
        let rows = backend.access_delay_register(&ehr_id).await?;
        let body = serde_json::json!({ "access_delays": rows });
        return Ok(negotiate::respond(&parts.headers, StatusCode::OK, &body));
    }
    let body = negotiate::json_value(&parts.headers, &parts.body)?;
    let ehr_id = required_str(&body, "ehr_id")?;
    let vo_id = required_str(&body, "vo_id")?;
    if op == "admin_delay_access" {
        let until = optional_str(&body, "until")?;
        let note = optional_str(&body, "note")?;
        backend
            .delay_person_access(ehr_id, vo_id, until, note)
            .await?;
    } else {
        backend.release_person_access(ehr_id, vo_id).await?;
    }
    Ok(negotiate::empty(StatusCode::NO_CONTENT))
}
