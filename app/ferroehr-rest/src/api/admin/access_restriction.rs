// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access restriction** wire: the EHDS Art. 8 restriction of
//! access by health professionals, **our own extension**.
//!
//! These routes record the restriction of Regulation (EU) 2025/327 Art. 8
//! (`docs/law/eu/ehds/text.html`) on the person's instruction, lift it, and
//! read the register. No openEHR ITS-REST operation governs any of it.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::response::Response;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{optional_str, required_query, required_str};
use crate::api::RequestParts;
use crate::api::guarded_handler;
use crate::api::refusals::JsonBodyRequired;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::{parse_ehr_id, parse_vo_id};
use crate::state::AppState;

/// The access-restriction extension routes — **no ITS-REST contract**
/// (module docs).
pub(crate) fn access_restriction_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            admin_restrict_access,
            admin_access_restriction_register
        ))
        .routes(routes!(admin_lift_access_restriction))
}

guarded_handler! {
    /// Record an EHDS Art. 8 restriction of access by health professionals
    /// (`POST /admin/access-restriction`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/access-restriction", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"ehr_id\": …, \"vo_id\": …?, \"note\": …? }`, \
                                    recorded on the person's instruction (EHDS \
                                    Art. 8). Omitting `vo_id` restricts the whole \
                                    EHR, including data recorded after it.",
                     example = json!({
                         "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                         "note": "instruction received at the front desk on 2026-10-20"
                     })),
        responses(
            (status = 204, description = "The restriction is recorded. Every \
                                          professional route, query and export \
                                          then treats the data as absent, and no \
                                          status, header or count says that a \
                                          restriction exists. A caller holding \
                                          `authz.rbac.emergency_access_role` that \
                                          declares an emergency purpose reads it \
                                          (EHDS Art. 11(5))."),
            (status = 400, description = "The body is not the object above.",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
            (status = 404, description = "The EHR is unknown, or the named object is \
                                          not in it. Nothing is recorded.",
             body = serde_json::Value),
            (status = 405, description = "The admin API is disabled on this server.",
             body = serde_json::Value),
            JsonBodyRequired
        )
    )]
    admin_restrict_access => |state, parts| {
            super::body::gated(state, parts, restrict_access)
        }
}

guarded_handler! {
    /// Read one EHR's access-restriction register
    /// (`GET /admin/access-restriction`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/access-restriction", tag = "admin-marks",
        params(("ehr_id" = String, Query, description = "The EHR whose register to read.")),
        responses(
            (status = 200, description = "The register rows, newest request first; \
                                          a lift is a row with `lifted_at` set.",
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
    admin_access_restriction_register => |state, parts| super::body::gated(state, parts, register)
}

guarded_handler! {
    /// Lift an EHDS Art. 8 access restriction
    /// (`POST /admin/access-restriction/lift`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/access-restriction/lift", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"ehr_id\": …, \"vo_id\": …? }`, the grain to \
                                    lift. Omitting `vo_id` lifts the whole-EHR \
                                    restrictions and leaves any object-scoped one \
                                    standing.",
                     example = json!({ "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398" })),
        responses(
            (status = 204, description = "Every in-force access restriction at that \
                                          grain carries a lift instant. Idempotent: \
                                          lifting nothing succeeds."),
            (status = 400, description = "The body is not the object above.",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
            (status = 404, description = "The EHR is unknown, or the named object is \
                                          not in it.",
             body = serde_json::Value),
            (status = 405, description = "The admin API is disabled on this server.",
             body = serde_json::Value),
            JsonBodyRequired
        )
    )]
    admin_lift_access_restriction => |state, parts| super::body::gated(state, parts, lift)
}

/// The `admin_restrict_access` operation.
async fn restrict_access(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let body = negotiate::decode::json_value(h, &parts.body)?;
    let ehr_id = required_str(&body, "ehr_id")?;
    let vo_id = optional_str(&body, "vo_id")?;
    let note = optional_str(&body, "note")?;
    state
        .backend()
        .restrict_access(
            parse_ehr_id(ehr_id)?,
            vo_id.map(parse_vo_id).transpose()?,
            note,
        )
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_access_restriction_register` operation.
async fn register(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let ehr_id = required_query(&parts, "ehr_id")?;
    let rows = state
        .backend()
        .access_restriction_register(parse_ehr_id(&ehr_id)?)
        .await?;
    let body: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "ehr_id": row.ehr_id.to_string(),
                "vo_id": row.vo_id.map(|id| id.to_string()),
                "requested_at": row.requested_at.to_string(),
                "lifted_at": row.lifted_at.map(|at| at.to_string()),
                "note": row.note,
            })
        })
        .collect();
    Ok(negotiate::body::respond(
        h,
        StatusCode::OK,
        &serde_json::json!({ "access_restrictions": body }),
    ))
}

/// The `admin_lift_access_restriction` operation.
async fn lift(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let body = negotiate::decode::json_value(h, &parts.body)?;
    let ehr_id = required_str(&body, "ehr_id")?;
    let vo_id = optional_str(&body, "vo_id")?;
    state
        .backend()
        .lift_access_restriction(parse_ehr_id(ehr_id)?, vo_id.map(parse_vo_id).transpose()?)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}
