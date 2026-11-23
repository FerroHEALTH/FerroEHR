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
use crate::api::refusals::{ResponseMap, json_responses};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;
use ferroehr::system_log::store::delivery::ParkedAct;

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
            ParkedRefusal,
            AdminRefusal
        )
    )]
    admin_retry_parked_audit_record => |state, parts| super::body::gated(state, parts, |state, parts| {
        on_parked(state, parts, ParkedAct::Retry)
    })
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
            ParkedRefusal,
            AdminRefusal
        )
    )]
    admin_dismiss_parked_audit_record => |state, parts| super::body::gated(state, parts, |state, parts| {
        on_parked(state, parts, ParkedAct::Dismiss)
    })
}

guarded_handler! {
    /// Forget a sink: remove its cursors and parked records
    /// (`POST /admin/audit-delivery/forget`). **Our own extension.**
    #[utoipa::path(
        post, path = "/admin/audit-delivery/forget", tag = "admin-audit-delivery",
        request_body(content = serde_json::Value,
                     description = "`{ \"sink\": … }`, a sink (`fhir_feed`, `syslog`) the \
                                    configuration no longer names, or one an operator \
                                    restarts from the records written after the server next starts.",
                     example = json!({ "sink": "syslog" })),
        responses(
            (status = 200, description = "`cursors` and `parked`: how many of each went. A \
                                          sink configured again starts at the last id \
                                          each log has drawn when the server next starts.",
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
        &view,
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

/// The `admin_retry_parked_audit_record` and
/// `admin_dismiss_parked_audit_record` operations: `act` done to the parked
/// record the body names.
async fn on_parked(
    state: AppState,
    parts: RequestParts,
    act: ParkedAct,
) -> Result<Response, RestError> {
    let (sink, chain, id) = parked(&parts)?;
    state
        .backend()
        .act_on_parked_audit_record(act, &sink, &chain, id)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The refusals of a retry and a dismissal past the admin gate, as one
/// response map for the OpenAPI document.
struct ParkedRefusal;

impl utoipa::IntoResponses for ParkedRefusal {
    fn responses() -> ResponseMap {
        json_responses(&[
            (400, "The body is not the object above."),
            (404, "The sink parked no such record."),
            (409, "The local audit store is off."),
        ])
    }
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
