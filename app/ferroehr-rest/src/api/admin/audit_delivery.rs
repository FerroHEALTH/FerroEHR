// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **audit delivery** wire: the delivery of the audit records to
//! the ITI-20 sinks that deliver from the local store, **our own extension**
//! (#6242).
//!
//! The view lists each sink's cursor of each log, configured or not, how many
//! records it has still to deliver, and the records it parked. A parked
//! record is retried (offered before the cursor's next record) or dismissed
//! (the sink will not deliver it); a sink taken out of the configuration is
//! forgotten, which removes its cursors and parked records. Each act is
//! recorded in the system log as an application activity naming the
//! operator, as every admin request is. No openEHR ITS-REST operation or IHE
//! profile governs any of it.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the delivery view and the act bodies are \
              operational JSON, not RM content"
)]

use axum::response::Response;
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, required_str};
use crate::api::RequestParts;
use crate::api::guarded_handler;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The audit-delivery extension routes: **no ITS-REST contract** (module
/// docs).
pub(crate) fn audit_delivery_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_audit_delivery))
        .routes(routes!(admin_retry_parked_audit_record))
        .routes(routes!(admin_dismiss_parked_audit_record))
        .routes(routes!(admin_forget_audit_sink))
}

guarded_handler! {
    /// Read the delivery of the audit records to each sink
    /// (`GET /admin/audit-delivery`). **Our own extension.**
    #[utoipa::path(
        get, path = "/admin/audit-delivery", tag = "admin-audit-delivery",
        responses(
            (status = 200, description = "`cursors`: each sink's cursor of each log \
                                          (`system`, `access_log`) with its `through_id`, \
                                          how many records it is `behind`, and whether \
                                          the configuration names the sink \
                                          (`configured`); `parked`: the records a sink \
                                          parked, oldest first, at most 1000, each with \
                                          its `reason` (`unrenderable`, `refused`, \
                                          `reaped`, `tombstoned`), `parked_at` and \
                                          whether a `retry` is asked.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store is off.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_audit_delivery => |state, parts| super::body::gated(state, parts, view)
}

guarded_handler! {
    /// Ask a sink to deliver a parked record again
    /// (`POST /admin/audit-delivery/retry`). **Our own extension.**
    #[utoipa::path(
        post, path = "/admin/audit-delivery/retry", tag = "admin-audit-delivery",
        request_body(content = serde_json::Value,
                     description = "`{ \"sink\": …, \"chain\": …, \"record_id\": … }`: the \
                                    sink (`fhir_feed`, `syslog`), the log (`system`, \
                                    `access_log`) and the record's id in it.",
                     example = json!({ "sink": "fhir_feed", "chain": "system", "record_id": 42 })),
        responses(
            (status = 204, description = "The sink offers the record before the next record \
                                          past its cursor; delivered, it leaves the parked \
                                          set, refused again, it stays parked."),
            (status = 400, description = "The body is not the object above.",
             body = serde_json::Value),
            (status = 404, description = "The sink parked no such record.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store is off.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_retry_parked_audit_record => |state, parts| super::body::gated(state, parts, retry)
}

guarded_handler! {
    /// Dismiss a parked record, which the sink will not deliver
    /// (`POST /admin/audit-delivery/dismiss`). **Our own extension.**
    #[utoipa::path(
        post, path = "/admin/audit-delivery/dismiss", tag = "admin-audit-delivery",
        request_body(content = serde_json::Value,
                     description = "`{ \"sink\": …, \"chain\": …, \"record_id\": … }`, as \
                                    for a retry.",
                     example = json!({ "sink": "syslog", "chain": "access_log", "record_id": 7 })),
        responses(
            (status = 204, description = "The record leaves the parked set; the sink never \
                                          delivers it. The record stays in the store."),
            (status = 400, description = "The body is not the object above.",
             body = serde_json::Value),
            (status = 404, description = "The sink parked no such record.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store is off.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_dismiss_parked_audit_record => |state, parts| super::body::gated(state, parts, dismiss)
}

guarded_handler! {
    /// Forget a sink: remove its cursors and parked records
    /// (`POST /admin/audit-delivery/forget`). **Our own extension.**
    #[utoipa::path(
        post, path = "/admin/audit-delivery/forget", tag = "admin-audit-delivery",
        request_body(content = serde_json::Value,
                     description = "`{ \"sink\": … }`, a sink (`fhir_feed`, `syslog`) the \
                                    configuration no longer names, or one an operator \
                                    restarts from the delivery frontier.",
                     example = json!({ "sink": "syslog" })),
        responses(
            (status = 200, description = "`cursors` and `parked`: how many of each went. A \
                                          sink configured again starts at the delivery \
                                          frontier of its first frontier round.",
             body = serde_json::Value),
            (status = 400, description = "The body does not name a sink.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store is off.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_forget_audit_sink => |state, parts| super::body::gated(state, parts, forget)
}

/// The `admin_audit_delivery` operation.
async fn view(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let view = state.backend().audit_delivery().await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::to_value(view).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

/// The sink, the log and the record id the body names.
fn parked(parts: &RequestParts) -> Result<(String, String, i64), RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let sink = required_str(&body, "sink")?.to_owned();
    let chain = required_str(&body, "chain")?.to_owned();
    let record_id = body
        .get("record_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| {
            RestError::from(ApiError::BadRequest(
                "`record_id` is the record's id in its log, an integer".to_owned(),
            ))
        })?;
    Ok((sink, chain, record_id))
}

/// The `admin_retry_parked_audit_record` operation.
async fn retry(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let (sink, chain, id) = parked(&parts)?;
    state
        .backend()
        .retry_parked_audit_record(&sink, &chain, id)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_dismiss_parked_audit_record` operation.
async fn dismiss(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let (sink, chain, id) = parked(&parts)?;
    state
        .backend()
        .dismiss_parked_audit_record(&sink, &chain, id)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_forget_audit_sink` operation.
async fn forget(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let sink = required_str(&body, "sink")?;
    let (cursors, parked) = state.backend().forget_audit_sink(sink).await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "cursors": cursors, "parked": parked }),
    ))
}
