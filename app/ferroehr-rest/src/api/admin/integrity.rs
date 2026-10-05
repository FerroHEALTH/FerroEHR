// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **storage-integrity** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs this route, and no SM interface
//! declares it either. The released Admin API is exactly two EHR deletes
//! (`specifications/admin.openapi.yaml`), and
//! `docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc` declares
//! deletes, archival and reporting — nothing that inspects stored data for
//! damage. No openEHR spec governs storage mechanics at all, so both the
//! two-copy storage design and this route over it are ours.
//!
//! What it exposes: the storage keeps every version's content twice — the
//! materialized `version.body` a point read serves, and the decomposed
//! `node` rows the AQL engine queries. Read-time signature verification (RM
//! common `master06-change_control_package.adoc` §Digital Signature) covers
//! the first copy. This route re-derives the second one and compares, so
//! tampering or corruption of either becomes visible.
//!
//! It is a `POST` because it is an action, not a resource: RFC 9110 §9.3.3
//! defines `POST` as "providing a block of data … to a data-handling process",
//! while a `GET` would present an expensive whole-repository scan as a
//! cacheable representation.
//!
//! Gating: mounted under `/admin/`, so it inherits the group's two gates
//! unchanged — the coarse RBAC `OperationClass::Admin` classifier (`401`
//! unauthenticated / `403` non-admin, our own authorization design; the
//! released admin operations carry `security: []`) and the
//! `AppConfig::admin.enabled` config gate, which answers `405` with an empty
//! `Allow` while the group is off (`crate::api::admin::dispatch`, whose ground
//! is the overview rule "If a method is recognized but not allowed for the
//! target resource, the response SHOULD be `405 Method Not Allowed` status
//! code" — `docs/overview/Requests_and_responses.md` §"HTTP Methods").

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (the \
              storage-parity report is an operational document, not an RM resource)"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http::{HeaderValue, StatusCode, header};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use ferroehr::service::admin::integrity::rebuild::{NodeRebuildOutcome, NodeRebuildReport};
use ferroehr::service::admin::integrity::{
    StorageParityEvent, StorageParityReport, StorageParityScope,
};

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The storage-integrity extension route as a native `utoipa-axum` router —
/// **no ITS-REST contract** (see the module docs). Group-relative path (nested
/// under `base_path`); the operation runs through [`guarded_dispatch`] with
/// [`dispatch`].
pub(crate) fn integrity_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_verify_storage_parity))
        .routes(routes!(admin_rebuild_version_nodes))
}

/// Sweep the stored versions for content-copy disagreement
/// (`POST /admin/integrity/verify`).
///
/// **Our own extension — no ITS-REST operation governs this, and it realizes
/// no SM operation either** (module docs). Every stored version in both
/// storage tiers is reassembled from its `node` rows and compared with its
/// materialized body; the response is the resulting report.
#[utoipa::path(
    post, path = "/admin/integrity/verify", tag = "admin-integrity",
    params(
        ("ehr_id" = Option<String>, Query,
         description = "Cover only versions belonging to this EHR. A sweep reads \
                        every byte of what it covers, so this is how an operator \
                        verifies one record without reading the repository."),
        ("committed_since" = Option<String>, Query,
         description = "Cover only versions whose validity begins at or after \
                        this RFC 3339 instant, for verifying what changed since \
                        an incident."),
        ("Accept" = Option<String>, Header,
         description = "`application/x-ndjson` streams the sweep as it runs, \
                        which is how a whole large repository is verified in \
                        one pass: the aggregated form computes its report \
                        before the response exists and is bounded by the \
                        server's 30 s request timeout. The stream must be \
                        asked for by name — a wildcard, or no `Accept` at all, \
                        keeps the aggregated document. Our own extension; no \
                        openEHR spec governs this media type."),
    ),
    responses(
        (status = 200, description = "The sweep ran. The body is the report: \
                                      how many stored versions were read, how \
                                      many carried a body, how many mismatches \
                                      were found, and the mismatching versions \
                                      by identifier. `mismatch_count` is the \
                                      full count; `mismatches` is capped and \
                                      `truncated` says whether the cap was \
                                      reached (every mismatch is logged at \
                                      `warn` whatever the cap does). A finding \
                                      is NOT a request failure — the sweep \
                                      succeeded and is reporting what it saw, \
                                      so the status stays `200`. Under \
                                      `Accept: application/x-ndjson` the same \
                                      sweep is streamed instead: one JSON \
                                      object per line, each carrying a `type` \
                                      — a `mismatch` as it is found, a \
                                      `progress` tick per enumerated page, and \
                                      one closing `summary` with the same \
                                      counts. A sweep that fails after the \
                                      response began cannot change the status \
                                      code, so it ends with an `error` line \
                                      instead of a `summary`: read to the end \
                                      to tell a finished sweep from an aborted \
                                      one.",
         content(
             (serde_json::Value = "application/json", example = json!({
                 "versions_checked": 128,
                 "versions_with_body": 126,
                 "versions_without_body": 2,
                 "mismatch_count": 1,
                 "mismatches": [{
                     "vo_id": "8849182c-82ad-4088-a07f-48ead4180515",
                     "sys_version": 2,
                     "kind": "COMPOSITION",
                     "defect": "content_differs"
                 }],
                 "truncated": false,
                 "elapsed_ms": 431
             })),
             (String = "application/x-ndjson", example = json!(
                 "{\"type\":\"mismatch\",\"vo_id\":\"8849182c-82ad-4088-a07f-48ead4180515\",\
                   \"sys_version\":2,\"kind\":\"COMPOSITION\",\"defect\":\"content_differs\"}\n\
                  {\"type\":\"progress\",\"versions_checked\":500,\"versions_with_body\":498,\
                   \"versions_without_body\":2,\"mismatch_count\":1}\n\
                  {\"type\":\"summary\",\"versions_checked\":128,\"versions_with_body\":126,\
                   \"versions_without_body\":2,\"mismatch_count\":1,\"elapsed_ms\":431}\n"
             )),
         )),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []` and declare no such \
                                      branch.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_verify_storage_parity(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_verify_storage_parity", parts, dispatch).await
}

/// Re-derive the node rows of every damaged version from its stored body
/// (`POST /admin/integrity/rebuild-nodes`).
///
/// **Our own extension — no ITS-REST operation governs this, and it realizes
/// no SM operation either** (module docs). It is the repair the storage-parity
/// sweep had no counterpart for: the sweep reports where a version's two
/// content copies disagree, and this replaces the derived copy from the
/// authoritative one.
#[utoipa::path(
    post, path = "/admin/integrity/rebuild-nodes", tag = "admin-integrity",
    params(
        ("ehr_id" = Option<String>, Query,
         description = "Cover only versions belonging to this EHR."),
        ("vo_id" = Option<String>, Query,
         description = "Cover only versions of this versioned object."),
        ("sys_version" = Option<i32>, Query,
         description = "Cover only this version of that object. The ordinal is \
                        per-object, so it needs `vo_id`; alone it is a `400`."),
        ("committed_since" = Option<String>, Query,
         description = "Cover only versions whose validity begins at or after \
                        this RFC 3339 instant, for repairing what an incident \
                        touched."),
    ),
    responses(
        (status = 200, description = "The rebuild ran. The body reports how \
                                      many versions the underlying sweep read, \
                                      how many it found damaged, how many were \
                                      rebuilt, and how many were refused, plus \
                                      one record per damaged version naming \
                                      the defect that selected it and what \
                                      happened to it. A refusal is NOT a \
                                      request failure: the version's own \
                                      stored body is the damaged copy, so \
                                      re-deriving the index from it would \
                                      spread the damage; the version's rows \
                                      are left exactly as they were and the \
                                      run continues. `records` is capped and \
                                      `truncated` says whether the cap was \
                                      reached; every record is also logged. A \
                                      run over an undamaged scope writes \
                                      nothing.",
         content(
             (serde_json::Value = "application/json", example = json!({
                 "versions_checked": 128,
                 "versions_damaged": 2,
                 "versions_rebuilt": 1,
                 "versions_refused": 1,
                 "records": [{
                     "vo_id": "8849182c-82ad-4088-a07f-48ead4180515",
                     "sys_version": 2,
                     "kind": "COMPOSITION",
                     "defect": "content_differs",
                     "outcome": "rebuilt",
                     "node_rows": 41
                 }, {
                     "vo_id": "1f0b7d64-6b2a-4a1f-9f0e-6b1d2a3c4d5e",
                     "sys_version": 1,
                     "kind": "COMPOSITION",
                     "defect": "content_differs",
                     "outcome": "refused",
                     "reason": "the stored body does not decompose: …"
                 }],
                 "truncated": false,
                 "elapsed_ms": 812
             })),
         )),
        (status = 400, description = "`sys_version` without `vo_id`, or a \
                                      malformed identifier or instant.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_rebuild_version_nodes(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_rebuild_version_nodes", parts, dispatch).await
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
    // The whole ADMIN group is opt-in; the gate and its two grounds are stated
    // once, in the group dispatcher.
    if let Some(refusal) = super::dispatch::admin_group_gate(&state) {
        return Ok(refusal);
    }
    match op {
        "admin_verify_storage_parity" => {
            let scope = parity_scope(parts.query.as_deref())?;
            if negotiate::accepts_ndjson(&parts.headers) {
                return Ok(stream_parity(&state, scope));
            }
            let report = state.backend().verify_storage_parity(scope).await?;
            Ok(negotiate::respond(
                &parts.headers,
                StatusCode::OK,
                &parity_report(&report),
            ))
        }
        "admin_rebuild_version_nodes" => {
            let scope = parity_scope(parts.query.as_deref())?;
            let report = state.backend().rebuild_version_nodes(scope).await?;
            Ok(negotiate::respond(
                &parts.headers,
                StatusCode::OK,
                &rebuild_report(&report),
            ))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted admin integrity operation: {other}"
        )))),
    }
}

/// The optional bounds narrowing which stored versions a sweep, or the rebuild
/// driven by one, covers.
///
/// A sweep reads every byte of what it covers, so an operator verifying one
/// record, one object, one version, or everything committed since an incident,
/// should not have to read the whole repository. Every parameter is optional
/// and they compose; the default covers everything.
///
/// Our own extension — no ITS-REST operation governs these routes at all.
fn parity_scope(query: Option<&str>) -> Result<StorageParityScope, RestError> {
    let ehr_id = match params::query_param(query, "ehr_id") {
        Some(raw) => Some(raw.parse::<uuid::Uuid>().map_err(|e| {
            RestError(ApiError::BadRequest(format!(
                "query parameter `ehr_id` must be a UUID, got {raw:?}: {e}"
            )))
        })?),
        None => None,
    };
    let committed_since = match params::query_param(query, "committed_since") {
        Some(raw) => Some(raw.parse::<jiff::Timestamp>().map_err(|e| {
            RestError(ApiError::BadRequest(format!(
                "query parameter `committed_since` must be an RFC 3339 timestamp, got \
                 {raw:?}: {e}"
            )))
        })?),
        None => None,
    };
    let vo_id = match params::query_param(query, "vo_id") {
        Some(raw) => Some(raw.parse::<uuid::Uuid>().map_err(|e| {
            RestError(ApiError::BadRequest(format!(
                "query parameter `vo_id` must be a UUID, got {raw:?}: {e}"
            )))
        })?),
        None => None,
    };
    let sys_version = match params::query_param(query, "sys_version") {
        Some(raw) => Some(raw.parse::<i32>().map_err(|e| {
            RestError(ApiError::BadRequest(format!(
                "query parameter `sys_version` must be an integer, got {raw:?}: {e}"
            )))
        })?),
        None => None,
    };
    // The ordinal is per-object, so on its own it would name one version of
    // every object in the scope — which is never what an operator naming a
    // version means.
    if sys_version.is_some() && vo_id.is_none() {
        return Err(RestError(ApiError::BadRequest(
            "query parameter `sys_version` names a version of one object and needs `vo_id`"
                .to_owned(),
        )));
    }
    Ok(StorageParityScope {
        ehr_id,
        committed_since,
        vo_id,
        sys_version,
    })
}

/// How many rendered lines the streamed sweep may run ahead of the client.
///
/// The channel IS the flow control: a client that reads slowly stops the sweep
/// at its next batch, and a client that disconnects drops the receiver, which
/// ends the sweep at its next send rather than letting it read the repository
/// for nobody.
const STREAM_BUFFER: usize = 64;

/// Answer the sweep as a line-delimited JSON stream.
///
/// The aggregated form computes the whole report before the response exists, so
/// it is bounded by the router's `REQUEST_TIMEOUT` and a large enough
/// repository outruns it. `tower_http`'s `TimeoutLayer` races only the inner
/// service's response FUTURE (its `ResponseFuture` polls the inner future
/// against one `Sleep`); once the head is produced the layer is out of the way,
/// and the separate body deadlines — `ResponseBodyTimeoutLayer` /
/// `ResponseBodyDeadlineLayer`, `tower_http::timeout` §"Body timeouts" — are
/// not applied by `crate::router`. So a response whose head returns at once and
/// whose body is written as the sweep proceeds has no such ceiling.
///
/// The cost is that the status code is committed before the work is: a fault
/// after the head can only be reported as the final line.
///
/// NOTE: no openEHR spec governs storage mechanics or this media type — our own
/// design/extension.
fn stream_parity(state: &AppState, scope: StorageParityScope) -> Response {
    let service = state.backend_handle();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(STREAM_BUFFER);
    drop(tokio::spawn(async move {
        let mut sweep = service.storage_parity_sweep(scope);
        loop {
            let batch = match sweep.next_batch().await {
                Ok(Some(batch)) => batch,
                Ok(None) => return,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "storage parity stream: the sweep failed after the response began"
                    );
                    // The diagnostic stays in the log: the wire line says the
                    // sweep did not finish, which is what a client can act on.
                    drop(
                        tx.send(parity_line(&json!({
                            "type": "error",
                            "message": "the sweep failed before it completed; see the server log",
                        })))
                        .await,
                    );
                    return;
                }
            };
            for event in batch {
                if tx.send(parity_line(&parity_event(&event))).await.is_err() {
                    return;
                }
            }
        }
    }));
    let stream = futures::stream::poll_fn(move |cx| {
        rx.poll_recv(cx)
            .map(|line| line.map(Ok::<Bytes, std::convert::Infallible>))
    });
    let mut response = axum::body::Body::from_stream(stream).into_response();
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(negotiate::APPLICATION_NDJSON),
    );
    response
}

/// One line of the stream: the rendered object plus its terminator.
fn parity_line(value: &Value) -> Bytes {
    let mut line = value.to_string();
    line.push('\n');
    Bytes::from(line)
}

/// Render one sweep event as its wire object.
///
/// Written out explicitly, like [`parity_report`]: the wire contract is this
/// function, not a serde attribute on a service type. Every object carries a
/// `type` so a reader dispatches on one key.
fn parity_event(event: &StorageParityEvent) -> Value {
    match event {
        StorageParityEvent::Mismatch(mismatch) => json!({
            "type": "mismatch",
            "domain": mismatch.domain.as_str(),
            "vo_id": mismatch.vo_id.to_string(),
            "sys_version": mismatch.sys_version,
            "kind": mismatch.kind,
            "defect": mismatch.defect.as_str(),
        }),
        StorageParityEvent::Progress { counts } => json!({
            "type": "progress",
            "versions_checked": counts.versions_checked,
            "versions_with_body": counts.versions_with_body,
            "versions_without_body": counts.versions_without_body,
            "mismatch_count": counts.mismatch_count,
        }),
        StorageParityEvent::Summary { counts, elapsed_ms } => json!({
            "type": "summary",
            "versions_checked": counts.versions_checked,
            "versions_with_body": counts.versions_with_body,
            "versions_without_body": counts.versions_without_body,
            "mismatch_count": counts.mismatch_count,
            "elapsed_ms": elapsed_ms,
        }),
    }
}

/// Render the storage-parity report as the response body.
///
/// The shape is ours end to end (no openEHR spec governs storage mechanics),
/// so it is written out explicitly here rather than derived: the wire contract
/// is this function, not a serde attribute on a service type.
fn parity_report(report: &StorageParityReport) -> Value {
    json!({
        "versions_checked": report.versions_checked,
        "versions_with_body": report.versions_with_body,
        "versions_without_body": report.versions_without_body,
        "mismatch_count": report.mismatch_count,
        "mismatches": report
            .mismatches
            .iter()
            .map(|m| json!({
                "domain": m.domain.as_str(),
                "vo_id": m.vo_id.to_string(),
                "sys_version": m.sys_version,
                "kind": m.kind,
                "defect": m.defect.as_str(),
            }))
            .collect::<Vec<Value>>(),
        "truncated": report.truncated,
        "elapsed_ms": report.elapsed_ms,
    })
}

/// Render the node-rebuild report as the response body.
///
/// Written out explicitly, like [`parity_report`]: the wire contract is this
/// function, not a serde attribute on a service type. A record carries the
/// key its outcome earns — `node_rows` for a rebuild, `reason` for a
/// refusal — so a reader never has to interpret a placeholder.
fn rebuild_report(report: &NodeRebuildReport) -> Value {
    json!({
        "versions_checked": report.versions_checked,
        "versions_damaged": report.versions_damaged,
        "versions_rebuilt": report.versions_rebuilt,
        "versions_refused": report.versions_refused,
        "records": report
            .records
            .iter()
            .map(|record| {
                let mut object = json!({
                    "domain": record.domain.as_str(),
                    "vo_id": record.vo_id.to_string(),
                    "sys_version": record.sys_version,
                    "kind": record.kind,
                    "defect": record.defect.as_str(),
                    "outcome": record.outcome.as_str(),
                });
                if let Some(map) = object.as_object_mut() {
                    match &record.outcome {
                        NodeRebuildOutcome::Rebuilt { node_rows } => {
                            map.insert("node_rows".to_owned(), json!(node_rows));
                        }
                        NodeRebuildOutcome::Refused { reason } => {
                            map.insert("reason".to_owned(), json!(reason));
                        }
                    }
                }
                object
            })
            .collect::<Vec<Value>>(),
        "truncated": report.truncated,
        "elapsed_ms": report.elapsed_ms,
    })
}
