// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **refused ITI-93 messages** wire: list the PMIR feed messages
//! the consumer refused, retry one once its cause is fixed, or settle one,
//! **our own extension** (#5725).
//!
//! A refused message keeps the identifiers and record subjects it split
//! unresolved until an operator acts. A retry applies it again at its
//! arrival position; a settlement closes it unapplied, keeping its splits
//! (`discard`) or lifting them (`lift-splits`), with the operator's reason.
//! No settlement applies a message by an operator's choice of identity. PMIR
//! leaves the Consumer's handling to its capabilities (PMIR 1.6.0
//! §2:3.93.4.1.3), and no openEHR ITS-REST operation governs these routes.

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

use ferroehr::service::demographic::identity_feed::operator::Decision;

use super::body::{AdminRefusal, required_str};
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The operation id of the list.
pub(crate) const LIST: &str = "admin_pmir_messages_list";
/// The operation id of a retry.
pub(crate) const RETRY: &str = "admin_pmir_message_retry";
/// The operation id of a settlement.
pub(crate) const SETTLE: &str = "admin_pmir_message_settle";

/// The refused-message routes — **no ITS-REST contract** (module docs).
pub(crate) fn pmir_message_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_pmir_messages_list))
        .routes(routes!(admin_pmir_message_retry))
        .routes(routes!(admin_pmir_message_settle))
}

guarded_handler! {
    /// List the refused ITI-93 messages still to retry or settle
    /// (`GET /admin/pmir/messages`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/pmir/messages", tag = "admin-identifiers",
        responses(
            (status = 200, description = "`{ \"messages\": [...] }`, oldest first: each \
                                          refused message with its refusal code, receipt \
                                          and refusal time, apply attempts, the \
                                          Registry's `Patient.id`s it names and the \
                                          `PERSON`s whose identifiers or record subjects \
                                          it keeps split, as counts. No identifier value \
                                          is answered. Each message listed is one access \
                                          record.",
             body = serde_json::Value,
             example = json!({ "messages": [{
                 "message_id": "0196f1f4-8c3a-7d2e-9b1a-3c5d7e9f1a2b",
                 "reason": "unknown-domain-type",
                 "received_at": "2026-11-16T09:15:00Z",
                 "refused_at": "2026-11-16T09:15:01Z",
                 "attempts": 1,
                 "identities": ["0196f1f4-8c3b-7a10-8f2e-4d6c8b0a2e4f"],
                 "splits": [{
                     "person": "0d6c7f1e-3b7a-4c2e-9f61-2a8e5d4c3b21",
                     "identifiers": 1,
                     "subjects": 0
                 }]
             }]})),
            AdminRefusal
        )
    )]
    admin_pmir_messages_list as LIST => |state, parts| super::body::gated(state, parts, list)
}

guarded_handler! {
    /// Retry a refused ITI-93 message once its cause is fixed
    /// (`POST /admin/pmir/messages/{message_id}/retry`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/pmir/messages/{message_id}/retry", tag = "admin-identifiers",
        params(("message_id" = String, Path, description = "The refused message's `MessageHeader.id`.")),
        responses(
            (status = 202, description = "`{ \"message_id\": …, \"state\": \"received\" }`: \
                                          the message is back at its arrival position, \
                                          and the applier applies it next under the \
                                          configuration and grants in force.",
             body = serde_json::Value),
            (status = 404, description = "No such message.", body = serde_json::Value),
            (status = 409, description = "The message is not refused, or a message \
                                          received after it that names one of its \
                                          identities has applied: settle it instead.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_pmir_message_retry as RETRY => |state, parts| super::body::gated(state, parts, retry)
}

guarded_handler! {
    /// Settle a refused ITI-93 message without applying it
    /// (`POST /admin/pmir/messages/{message_id}/settle`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/pmir/messages/{message_id}/settle", tag = "admin-identifiers",
        params(("message_id" = String, Path, description = "The refused message's `MessageHeader.id`.")),
        request_body(content = serde_json::Value,
                     description = "`{ \"decision\": …, \"reason\": … }`: `discard` keeps \
                                    the message's splits, `lift-splits` removes them so the \
                                    identifiers and record subjects resolve as before the \
                                    message; the reason is 1 to 1000 characters and names \
                                    no identifier.",
                     example = json!({
                         "decision": "discard",
                         "reason": "the Registry corrected the identity in a later message"
                     })),
        responses(
            (status = 200, description = "`{ \"message_id\": …, \"decision\": …, \
                                          \"lifted\": … }`: the message is settled, \
                                          `lifted` counting the splits removed.",
             body = serde_json::Value),
            (status = 400, description = "The decision is neither `discard` nor \
                                          `lift-splits`, or the reason is missing, empty \
                                          or longer than 1000 characters.",
             body = serde_json::Value),
            (status = 404, description = "No such message.", body = serde_json::Value),
            (status = 409, description = "The message is not refused, and is not a \
                                          discarded one whose splits are being lifted.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_pmir_message_settle as SETTLE => |state, parts| super::body::gated(state, parts, settle)
}

/// The `admin_pmir_messages_list` operation.
async fn list(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let listed = state.backend().pmir_refused().await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &listed,
    ))
}

/// The `admin_pmir_message_retry` operation.
async fn retry(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let message_id = message_id(&parts)?;
    let retried = state.backend().pmir_retry(message_id).await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::ACCEPTED,
        &retried,
    ))
}

/// The `admin_pmir_message_settle` operation.
async fn settle(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let message_id = message_id(&parts)?;
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let code = required_str(&body, "decision")?;
    let decision = Decision::of(code).ok_or_else(|| {
        RestError::from(ApiError::BadRequest(format!(
            "`decision` must be `discard` or `lift-splits`, got {code:?}"
        )))
    })?;
    let reason = required_str(&body, "reason")?;
    let settled = state
        .backend()
        .pmir_settle(message_id, decision, reason)
        .await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &settled,
    ))
}

/// The `message_id` path parameter.
fn message_id(parts: &RequestParts) -> Result<&str, RestError> {
    parts
        .path
        .get("message_id")
        .map(String::as_str)
        .ok_or_else(|| {
            RestError::from(ApiError::Internal(
                "missing path parameter `message_id`".to_owned(),
            ))
        })
}
