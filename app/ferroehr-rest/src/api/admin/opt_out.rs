// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access opt-out** wire: the EHDS Art. 10 opt-out from access
//! through the access services, **our own extension**.
//!
//! Regulation (EU) 2025/327 Art. 10(1) lets a Member State's law give a person
//! the right to opt out from access to their data "through the electronic
//! health data access services referred to in Articles 4 and 12", and has the
//! Member State "ensure that the exercise of that right is reversible"
//! (`docs/law/eu/ehds/text.html`). These routes record the person's opt-out on
//! their instruction, reverse it, and read the register. No openEHR ITS-REST
//! operation governs any of it.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off) unchanged. Recording is refused while
//! `[authz.rbac] access_service_roles` is empty, since the mark would then
//! hold for nobody; reversing and reading are always open, so a recorded
//! opt-out can always be reversed.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::response::Response;
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, RegisterReadRefusal, optional_str, required_query, required_str};
use crate::api::RequestParts;
use crate::api::guarded_handler;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::parse_ehr_id;
use crate::state::AppState;

/// The access opt-out extension routes — **no ITS-REST contract** (module
/// docs).
pub(crate) fn opt_out_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_access_opt_out, admin_access_opt_out_register))
        .routes(routes!(admin_reverse_access_opt_out))
}

guarded_handler! {
    /// Record the person's opt-out from access through the access services
    /// (`POST /admin/access-opt-out`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/access-opt-out", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"ehr_id\": …, \"note\": …? }`, recorded on the \
                                    person's instruction (EHDS Art. 10(1)). The \
                                    opt-out covers the whole EHR.",
                     example = json!({
                         "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                         "note": "opt-out received through the national access service on 2026-11-01"
                     })),
        responses(
            (status = 204, description = "The opt-out is recorded. A caller holding \
                                          one of `authz.rbac.access_service_roles` \
                                          then reads the EHR as absent at every \
                                          route, query and export, unless it \
                                          declares an emergency purpose and acts \
                                          for no person (EHDS Art. 10(2)); every \
                                          other caller reads it as before."),
            (status = 400, description = "The body is not the object above, or the \
                                          deployment declares no access-service \
                                          caller (`authz.rbac.access_service_roles` \
                                          is empty).",
             body = serde_json::Value),
            (status = 404, description = "No such EHR; nothing is recorded.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_opt_out => |state, parts| {
            super::body::gated(state, parts, opt_out)
        }
}

guarded_handler! {
    /// Read one EHR's opt-out register (`GET /admin/access-opt-out`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/access-opt-out", tag = "admin-marks",
        params(("ehr_id" = String, Query, description = "The EHR whose register to read.")),
        responses(
            (status = 200, description = "The register rows, newest first; a \
                                          reversal is a row with `reversed_at` set.",
             body = serde_json::Value),
            RegisterReadRefusal,
            AdminRefusal
        )
    )]
    admin_access_opt_out_register => |state, parts| super::body::gated(state, parts, register)
}

guarded_handler! {
    /// Reverse the person's opt-out (`POST /admin/access-opt-out/reverse`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/access-opt-out/reverse", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"ehr_id\": … }`, on the person's instruction \
                                    (EHDS Art. 10(1): the exercise of the right is \
                                    reversible).",
                     example = json!({ "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398" })),
        responses(
            (status = 204, description = "Every opt-out that stood carries a \
                                          reversal instant, and the access services \
                                          read the EHR again. Reversing nothing \
                                          succeeds."),
            (status = 400, description = "The body does not name the EHR.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_reverse_access_opt_out => |state, parts| super::body::gated(state, parts, reverse)
}

/// The `admin_access_opt_out_register` operation.
async fn register(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let rows = state
        .backend()
        .access_opt_out_register(parse_ehr_id(&required_query(&parts, "ehr_id")?)?)
        .await?;
    let register = serde_json::json!({ "access_opt_outs": rows });
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &register,
    ))
}

/// The `admin_access_opt_out` operation.
async fn opt_out(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let ehr_id = required_str(&body, "ehr_id")?;
    let declared = &state.config().rbac.access_service_roles;
    if declared.iter().all(|role| role.trim().is_empty()) {
        return Err(RestError::from(ApiError::BadRequest(
            "this deployment declares no EHDS Art. 10 access-service caller \
             ([authz.rbac] access_service_roles is empty), so an opt-out would hold for \
             nobody; declare the roles of the access services first"
                .to_owned(),
        )));
    }
    let note = optional_str(&body, "note")?;
    state
        .backend()
        .record_access_opt_out(parse_ehr_id(ehr_id)?, note)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_reverse_access_opt_out` operation.
async fn reverse(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let ehr_id = required_str(&body, "ehr_id")?;
    state
        .backend()
        .reverse_access_opt_out(parse_ehr_id(ehr_id)?)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}
