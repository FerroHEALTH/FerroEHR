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
//! creation; these routes let a health professional hold an object, release it
//! early, and read the register. No openEHR ITS-REST operation and no SM
//! interface governs any of it.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::admin.enabled` config gate
//! (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{optional_str, required_str};
use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

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
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown, or the named object is \
                                      not in it. Nothing is recorded.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_delay_access(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_delay_access", parts, dispatch).await
}

/// Read one EHR's access-delay register (`GET /admin/access-delay`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    get, path = "/admin/access-delay", tag = "admin-marks",
    params(("ehr_id" = String, Query, description = "The EHR whose register to read.")),
    responses(
        (status = 200, description = "The register rows, newest hold first; a \
                                      release is a row with `released_at` set.",
         body = serde_json::Value),
        (status = 400, description = "`ehr_id` is missing or not a UUID.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_access_delay_register(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_access_delay_register", parts, dispatch).await
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
                                      composition. Idempotent: releasing \
                                      nothing succeeds."),
        (status = 400, description = "The body is not the object above.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown, or the named object is \
                                      not in it.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_release_access_delay(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_release_access_delay", parts, dispatch).await
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        run(state, op, parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    if let Some(refusal) = super::dispatch::admin_group_gate(&state) {
        return Ok(refusal);
    }
    let h = &parts.headers;
    match op {
        "admin_delay_access" => {
            let body = negotiate::json_value(h, &parts.body)?;
            state
                .backend()
                .delay_person_access(
                    required_str(&body, "ehr_id")?,
                    required_str(&body, "vo_id")?,
                    optional_str(&body, "until")?,
                    optional_str(&body, "note")?,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_access_delay_register" => {
            let ehr_id =
                params::query_param(parts.query.as_deref(), "ehr_id").ok_or_else(|| {
                    RestError::from(ApiError::BadRequest(
                        "the `ehr_id` query parameter is required".to_owned(),
                    ))
                })?;
            let rows = state.backend().access_delay_register(&ehr_id).await?;
            let body: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.id,
                        "ehr_id": row.ehr_id.to_string(),
                        "vo_id": row.vo_id.to_string(),
                        "basis": row.basis,
                        "withheld_at": row.withheld_at.to_string(),
                        "withheld_until": row.withheld_until.to_string(),
                        "released_at": row.released_at.map(|at| at.to_string()),
                        "note": row.note,
                    })
                })
                .collect();
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &serde_json::json!({ "access_delays": body }),
            ))
        }
        "admin_release_access_delay" => {
            let body = negotiate::json_value(h, &parts.body)?;
            state
                .backend()
                .release_person_access(
                    required_str(&body, "ehr_id")?,
                    required_str(&body, "vo_id")?,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError::from(ApiError::Internal(format!(
            "unrouted admin access delay operation: {other}"
        )))),
    }
}
