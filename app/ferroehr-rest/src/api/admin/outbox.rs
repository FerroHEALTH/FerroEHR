// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **outbox feed** wire: the commit events after a reader's cursor,
//! and the acknowledgement that moves the cursor — **our own extension**.
//!
//! No openEHR spec governs eventing: ITS-REST 1.1.0 defines no change
//! notification, so the routes, payloads and status codes are our own design.
//! A reader is one `[record.change_feed] readers` names; the family's export server reads
//! the feed and delivers the events on to a message broker. The clinical and the party
//! outbox each keep their own `seq` and their own cursor, chosen by the
//! `domain` parameter.
//!
//! Gating: mounted under `/admin/`, so the routes inherit the group's RBAC
//! Admin class; the SMART route table holds both to the `__ferroehr.outbox`
//! scope. The `[api] admin` toggle does not reach them: a deployment that names a
//! reader serves its feed. Every read is one access record.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the PHI-free event envelope is open \
              operational JSON, and the OpenAPI examples are written as JSON"
)]

use axum::response::Response;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::extensions::outbox::{FeedEvent, OutboxDomain};
use openehr_its::rest::runtime::ApiError;

use crate::api::{RequestParts, guarded_handler};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The operation id of the feed read.
pub(crate) const READ: &str = "admin_outbox_read";
/// The operation id of the acknowledgement.
pub(crate) const ACK: &str = "admin_outbox_ack";

/// The page size when `limit` is absent.
const DEFAULT_LIMIT: i64 = 100;
/// The largest page a read may ask for.
const MAX_LIMIT: i64 = 1_000;

/// The outbox feed routes — **no ITS-REST contract** (module docs).
pub(crate) fn outbox_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_outbox_read))
        .routes(routes!(admin_outbox_ack))
}

/// The feed page: the reader and its events.
#[derive(Debug, serde::Serialize)]
struct FeedPage<'a> {
    /// The reader the page was read for.
    reader: &'a str,
    /// The events after the reader's cursor, in `seq` order.
    events: Vec<FeedEvent>,
}

/// The acknowledgement body.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    /// The highest `seq` the reader has delivered.
    last_seq: i64,
}

guarded_handler! {
    /// Read the events after a reader's cursor
    /// (`GET /admin/outbox/{reader}`).
    ///
    /// **Our own extension — no openEHR spec governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/outbox/{reader}", tag = "admin-outbox",
        params(
            ("reader" = String, Path, description = "A reader `[record.change_feed] readers` names."),
            ("limit" = Option<i64>, Query, description = "The most events to return, 1 to 1000; 100 when absent."),
            ("domain" = Option<String>, Query, description = "The outbox to read: `clinical` (the default) or `party`. Each has its own `seq` and its own cursor.")
        ),
        responses(
            (status = 200, description = "`{ \"reader\": …, \"events\": [...] }`: the \
                                          events after the reader's cursor in `seq` \
                                          order, empty when there are none. Each event \
                                          is the stored PHI-free envelope with its \
                                          `seq` beside it. A read moves no cursor.",
             body = serde_json::Value,
             example = json!({
                 "reader": "ferrobridge",
                 "events": [{
                     "seq": 42,
                     "contribution_id": "0196f1f4-8c3a-7d2e-9b1a-3c5d7e9f1a2b",
                     "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                     "committed_at": "2026-11-02T09:15:00.123456Z",
                     "versions": [{
                         "vo_id": "0196f1f4-8c3b-7a10-8f2e-4d6c8b0a2e4f",
                         "kind": "COMPOSITION",
                         "sys_version": 3,
                         "version_tree_id": "3",
                         "change_type": "251",
                         "template_id": "vital_signs.v1"
                     }]
                 }]
             })),
            (status = 400, description = "`limit` is not an integer from 1 to 1000, or \
                                          `domain` is neither `clinical` nor `party`.",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "Not in the Admin class, or the token lacks the \
                                          `__ferroehr.outbox` scope.",
             body = serde_json::Value),
            (status = 404, description = "The reader is unknown or inactive: \
                                          `[record.change_feed] readers` does not name it.",
             body = serde_json::Value)
        )
    )]
    admin_outbox_read as READ => read
}

guarded_handler! {
    /// Acknowledge the events up to a `seq`, moving the reader's cursor
    /// (`POST /admin/outbox/{reader}/ack`).
    ///
    /// **Our own extension — no openEHR spec governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/outbox/{reader}/ack", tag = "admin-outbox",
        params(
            ("reader" = String, Path, description = "A reader `[record.change_feed] readers` names."),
            ("domain" = Option<String>, Query, description = "The outbox whose cursor moves: `clinical` (the default) or `party`.")
        ),
        request_body(content = serde_json::Value,
                     description = "`{ \"last_seq\": … }`: the highest `seq` the reader has delivered.",
                     example = json!({ "last_seq": 42 })),
        responses(
            (status = 204, description = "The cursor is at `last_seq`. Acknowledging \
                                          the current cursor again is a `204` too."),
            (status = 400, description = "The body is not `{ \"last_seq\": <integer> }`, \
                                          or `domain` is neither `clinical` nor `party`.",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "Not in the Admin class, or the token lacks the \
                                          `__ferroehr.outbox` scope.",
             body = serde_json::Value),
            (status = 404, description = "The reader is unknown or inactive.",
             body = serde_json::Value),
            (status = 409, description = "`last_seq` lies below the cursor, which never \
                                          moves back, or above the highest row of the \
                                          outbox.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value)
        )
    )]
    admin_outbox_ack as ACK => ack
}

/// The `admin_outbox_read` operation.
pub(crate) async fn read(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let reader = parts.path.get("reader").ok_or_else(|| {
        RestError::from(ApiError::Internal(
            "missing path parameter `reader`".to_owned(),
        ))
    })?;
    let domain = domain(parts.query.as_deref())?;
    let limit = limit(parts.query.as_deref())?;
    let events = state.backend().outbox_events(domain, reader, limit).await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &FeedPage { reader, events },
    ))
}

/// The `admin_outbox_ack` operation.
pub(crate) async fn ack(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let reader = parts.path.get("reader").ok_or_else(|| {
        RestError::from(ApiError::Internal(
            "missing path parameter `reader`".to_owned(),
        ))
    })?;
    let domain = domain(parts.query.as_deref())?;
    let ack: Ack = negotiate::decode::typed_json(&parts.headers, &parts.body, "JSON body")?;
    state
        .backend()
        .outbox_acknowledge(domain, reader, ack.last_seq)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `domain` query parameter, the clinical outbox when absent.
fn domain(query: Option<&str>) -> Result<OutboxDomain, RestError> {
    match params::query_param(query, "domain") {
        None => Ok(OutboxDomain::default()),
        Some(token) => OutboxDomain::parse(&token).ok_or_else(|| {
            RestError::from(ApiError::BadRequest(format!(
                "`domain` must be `clinical` or `party`, got {token:?}"
            )))
        }),
    }
}

/// The `limit` query parameter, [`DEFAULT_LIMIT`] when absent.
fn limit(query: Option<&str>) -> Result<i64, RestError> {
    let Some(raw) = params::query_param(query, "limit") else {
        return Ok(DEFAULT_LIMIT);
    };
    raw.parse::<i64>()
        .ok()
        .filter(|limit| (1..=MAX_LIMIT).contains(limit))
        .ok_or_else(|| {
            RestError::from(ApiError::BadRequest(format!(
                "`limit` must be an integer from 1 to {MAX_LIMIT}, got {raw:?}"
            )))
        })
}

#[cfg(test)]
mod tests {
    use super::{domain, limit};
    use ferroehr::extensions::outbox::OutboxDomain;

    #[test]
    fn limit_defaults_to_100_and_takes_1_to_1000() {
        assert_eq!(limit(None).ok(), Some(100));
        assert_eq!(limit(Some("limit=1")).ok(), Some(1));
        assert_eq!(limit(Some("limit=1000")).ok(), Some(1000));
        for bad in ["limit=0", "limit=1001", "limit=-5", "limit=ten", "limit="] {
            assert!(limit(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn domain_defaults_to_clinical_and_names_two_outboxes() {
        assert_eq!(domain(None).ok(), Some(OutboxDomain::Clinical));
        assert_eq!(domain(Some("domain=party")).ok(), Some(OutboxDomain::Party));
        assert!(domain(Some("domain=audit")).is_err());
    }
}
