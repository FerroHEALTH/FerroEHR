// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access-settings repair** wire: an administrator replaces the
//! `EHR_ACCESS` settings of an EHR held closed because its own do not parse,
//! **our own extension**.
//!
//! A held EHR refuses every route that names it in its path, the
//! CONTRIBUTION that would replace its settings included, so the EHR is named
//! in the body here. No openEHR ITS-REST operation governs this route, and no
//! openEHR specification governs FerroEHR's access-control scheme.
//!
//! Gating: mounted under `/admin/`, so the route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off). The repair commits a new
//! `EHR_ACCESS` version with its commit audit and writes an access record
//! (`ferroehr::service::admin::access_repair`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use axum::response::Response;
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, required_str};
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::parse_ehr_id;
use crate::state::AppState;

/// The access-settings repair route — **no ITS-REST contract** (module docs).
pub(crate) fn access_repair_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(admin_ehr_access_repair))
}

guarded_handler! {
    /// Replace the settings of an EHR held closed by `EHR_ACCESS` settings
    /// that do not parse (`POST /admin/ehr-access/repair`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/ehr-access/repair", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"ehr_id\": …, \"settings\": … }`: the EHR and \
                                    the whole new `EHR_ACCESS.settings` object, of the \
                                    `ferroehr.access_control.v1` scheme, whose \
                                    `_type` is `FERROEHR_ACCESS_CONTROL_V1`."),
        responses(
            (status = 200, description = "`{ \"ehr_id\": …, \"version_uid\": … }`: the \
                                          `EHR_ACCESS` version the repair committed. The \
                                          EHR is served again under the new settings.",
             body = serde_json::Value),
            (status = 400, description = "The body does not name the EHR, or the \
                                          settings are not settings of the \
                                          `ferroehr.access_control.v1` scheme that parse.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            (status = 409, description = "The EHR's settings parse, so it is not held: a \
                                          CONTRIBUTION changes them.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_ehr_access_repair => |state, parts| {
            super::body::gated(state, parts, repair)
        }
}

/// The `admin_ehr_access_repair` operation.
async fn repair(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let ehr_id = parse_ehr_id(required_str(&body, "ehr_id")?)?;
    let Some(settings) = body.get("settings").filter(|value| value.is_object()) else {
        return Err(RestError::from(ApiError::BadRequest(
            "request body must carry a `settings` object".to_owned(),
        )));
    };
    let meta = state
        .backend()
        .repair_ehr_access_settings(ehr_id, settings.clone())
        .await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "ehr_id": meta.ehr_id, "version_uid": meta.uid }),
    ))
}
