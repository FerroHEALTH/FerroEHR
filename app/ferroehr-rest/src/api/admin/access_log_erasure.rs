// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access-log erasure** wire: an administrator replaces an
//! access-log entry by its tombstone because a law orders its erasure before
//! its retention ends, **our own extension** (#5128).
//!
//! The erasure takes the path the end of a retention period takes
//! (`docs/plans/access-log.md` §Retention): the tombstone keeps the entry's
//! uid, `system_id`, `time_recorded` and chain link and names the grounds
//! the body gives. No openEHR ITS-REST operation governs this route, and no
//! openEHR specification defines an access log.
//!
//! Gating: mounted under `/admin/`, so the route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off). The erasure writes its access record
//! (`ferroehr::service::admin::access_log_erasure`) with the tombstone and emits
//! it to the forwarding sinks, so the response of a done erasure is marked
//! written and the audit middleware adds no record of its own.

use axum::response::Response;
use ferroehr::system_log::event::EntryScope;
use http::StatusCode;
use openehr_base::containers::NonEmptyVec;
use openehr_its::rest::runtime::ApiError;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, required_str};
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::system_log::middleware::AuditObject;

/// The access-log erasure route: **no ITS-REST contract** (module docs).
pub(crate) fn access_log_erasure_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(admin_access_log_erase))
}

guarded_handler! {
    /// Replace an access-log entry by its tombstone on the grounds of the law
    /// that orders its erasure (`POST /admin/access-log/erase`).
    ///
    /// **Our own extension: no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/access-log/erase", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"uid\": …, \"grounds\": [ … ] }`: the uid of the \
                                    access-log entry and one or more `DV_CODED_TEXT` \
                                    naming the provisions that order its erasure."),
        responses(
            (status = 200, description = "The `ACCESS_LOG_TOMBSTONE` that replaced the \
                                          entry, with its uid, `system_id`, \
                                          `time_recorded`, chain link, `time_removed` \
                                          and grounds.",
             body = serde_json::Value),
            (status = 400, description = "The body names no uid, gives no grounds, or a \
                                          ground is no `DV_CODED_TEXT` the model admits.",
             body = serde_json::Value),
            (status = 404, description = "No access-log record has the uid.",
             body = serde_json::Value),
            (status = 409, description = "The record is a tombstone already, or the local \
                                          audit store, which keeps the access log, is off.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_log_erase => |state, parts| {
            super::body::gated(state, parts, erase)
        }
}

/// The `admin_access_log_erase` operation.
async fn erase(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let uid = required_str(&body, "uid")?;
    let Some(grounds) = body.get("grounds") else {
        return Err(RestError::from(ApiError::BadRequest(
            "request body must carry `grounds`: one or more DV_CODED_TEXT".to_owned(),
        )));
    };
    let grounds =
        serde_json::from_value::<NonEmptyVec<DvCodedText>>(grounds.clone()).map_err(|error| {
            RestError::from(ApiError::BadRequest(format!(
                "`grounds` must be one or more DV_CODED_TEXT: {error}"
            )))
        })?;
    let tombstone = state.backend().erase_access_log_entry(uid, grounds).await?;
    let mut resp = negotiate::body::respond(&parts.headers, StatusCode::OK, &tombstone);
    // NOTE: our own design (#5602), the erasure wrote and emitted its record, the one record of
    // the act on every sink, so the middleware writes none.
    resp.extensions_mut().insert(AuditObject {
        uid: Some(uid.to_owned()),
        entry: EntryScope::Written,
        ..AuditObject::default()
    });
    Ok(resp)
}
